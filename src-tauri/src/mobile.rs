//! Launch-command generation for mobile projects (iOS / Android / Flutter /
//! Expo).
//!
//! PortBay's Play/Stop is a Process Compose process: Play runs a long-running
//! `command`, Stop sends it SIGTERM. Native mobile "runs" aren't naturally
//! long-running (build → install → launch is one-shot), so each command here
//! ends by **attaching to a live stream** — `simctl launch --console-pty`,
//! `adb logcat`, `flutter run`, or the Expo Metro server. The stream keeps the
//! PC process alive while the app runs, and Stop tears it down.
//!
//! Device selection is resolved **at run time inside the shell** (not here), so
//! the command is self-contained and reflects whatever simulator/emulator the
//! user actually has booted: an explicit `MobileRunConfig.device` wins,
//! otherwise the first booted device, otherwise the first available one. This
//! keeps PC-config generation pure (no `simctl`/`adb` I/O in the reconcile
//! tick) and lets the same registry entry work across machines.
//!
//! The shell here targets `/bin/sh` (what Process Compose invokes) and is kept
//! to one logical line via `&&`/`;`/`if…fi` so it round-trips through the YAML
//! `command` scalar cleanly. Paths are referenced relative to the project dir
//! (PC sets `working_dir` to the project path), sidestepping spaces in the
//! absolute path.

use crate::registry::{MobileRunConfig, Project, ProjectType};

/// Build the Play command for a mobile project, or `None` if the kind isn't a
/// mobile kind. The returned string is handed to Process Compose as the
/// process `command` and run from the project directory.
pub fn launch_command(p: &Project) -> Option<String> {
    let cfg = p.mobile_run.clone().unwrap_or_default();
    launch_command_for(p.kind, &cfg)
}

/// Core: the Play command for a mobile `kind` + its run config. Shared by the
/// reconciler (via [`launch_command`]) and the folder detector, so a mobile
/// project launches identically whether the command is generated at reconcile
/// time or stamped onto the registry at add time. Returns `None` for
/// non-mobile kinds.
pub fn launch_command_for(kind: ProjectType, cfg: &MobileRunConfig) -> Option<String> {
    match kind {
        ProjectType::Xcode => Some(ios_command(cfg)),
        ProjectType::Android => Some(android_command(cfg)),
        ProjectType::Flutter => Some(flutter_command(cfg)),
        ProjectType::Expo => Some(expo_command(cfg)),
        _ => None,
    }
}

/// True for kinds whose Play is a mobile simulator/emulator launch rather than
/// an HTTP dev server. Used by the PC-config layer to skip HTTP readiness
/// probes (a simulator launch never answers an HTTP GET).
pub fn is_mobile_kind(kind: ProjectType) -> bool {
    matches!(
        kind,
        ProjectType::Xcode | ProjectType::Android | ProjectType::Flutter | ProjectType::Expo
    )
}

/// Single-quote a value for safe interpolation into the `/bin/sh` command.
/// Empty stays empty (callers test for that). `'` is escaped the POSIX way.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// iOS (Xcode)
// ---------------------------------------------------------------------------

/// boot a simulator → build the scheme → install → launch attached to the
/// console. Resolves project/workspace, scheme, and device at run time.
fn ios_command(cfg: &MobileRunConfig) -> String {
    // Pinned device udid/name (empty → auto). `-configuration` follows the
    // flavor when set, else Debug.
    let pinned_dev = cfg.device.clone().unwrap_or_default();
    let pinned_scheme = cfg.target.clone().unwrap_or_default();
    let config = match cfg.flavor.as_deref() {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => "Debug".to_string(),
    };

    format!(
        "set -e; \
         WS=$(ls -d *.xcworkspace 2>/dev/null | head -1); \
         PROJ=$(ls -d *.xcodeproj 2>/dev/null | head -1); \
         if [ -n \"$WS\" ]; then PF=\"-workspace $WS\"; else PF=\"-project $PROJ\"; fi; \
         DEV={pinned_dev}; \
         if [ -z \"$DEV\" ]; then DEV=$(xcrun simctl list devices booted | grep -oE '[0-9A-Fa-f-]{{36}}' | head -1); fi; \
         if [ -z \"$DEV\" ]; then DEV=$(xcrun simctl list devices available | grep -i iPhone | grep -oE '[0-9A-Fa-f-]{{36}}' | head -1); fi; \
         if [ -z \"$DEV\" ]; then echo 'no iOS simulator found — create one in Xcode' >&2; exit 1; fi; \
         xcrun simctl boot \"$DEV\" 2>/dev/null || true; \
         open -a Simulator || true; \
         SCHEME={pinned_scheme}; \
         if [ -z \"$SCHEME\" ]; then SCHEME=$(xcodebuild $PF -list 2>/dev/null | awk '/Schemes:/{{getline; print; exit}}' | sed 's/^ *//'); fi; \
         xcodebuild $PF -scheme \"$SCHEME\" -configuration {config} -destination \"id=$DEV\" -derivedDataPath .portbay-build build; \
         APP=$(find .portbay-build/Build/Products -maxdepth 2 -name '*.app' | head -1); \
         BID=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' \"$APP/Info.plist\"); \
         xcrun simctl install \"$DEV\" \"$APP\"; \
         exec xcrun simctl launch --console-pty \"$DEV\" \"$BID\"",
        pinned_dev = sq(&pinned_dev),
        pinned_scheme = sq(&pinned_scheme),
        config = sq(&config),
    )
}

// ---------------------------------------------------------------------------
// Android (Gradle)
// ---------------------------------------------------------------------------

/// ensure a device/emulator → `gradlew installDebug` → launch the monkey
/// LAUNCHER intent → attach to `adb logcat`. Reads `applicationId` from the
/// app module's gradle for the launch intent.
fn android_command(cfg: &MobileRunConfig) -> String {
    // Pinned serial (empty → auto). A product `flavor` prefixes the install
    // task (`installStagingDebug`); the build *type* is always Debug here, so a
    // flavor of "debug"/"release" is treated as the type — not doubled into
    // `installDebugDebug`. No flavor → `installDebug`.
    let pinned_serial = cfg.device.clone().unwrap_or_default();
    let product_flavor = cfg.flavor.as_deref().map(str::trim).filter(|f| {
        !f.is_empty() && !f.eq_ignore_ascii_case("debug") && !f.eq_ignore_ascii_case("release")
    });
    let install_task = match product_flavor {
        Some(f) => format!("install{}Debug", capitalize(f)),
        None => "installDebug".to_string(),
    };
    let module = match cfg.target.as_deref() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => "app".to_string(),
    };

    format!(
        "set -e; \
         SER={pinned_serial}; \
         if [ -z \"$SER\" ]; then SER=$(adb devices | awk 'NR>1 && $2==\"device\"{{print $1; exit}}'); fi; \
         if [ -z \"$SER\" ]; then AVD=$(emulator -list-avds 2>/dev/null | head -1); \
           if [ -n \"$AVD\" ]; then (emulator -avd \"$AVD\" >/dev/null 2>&1 &); adb wait-for-device; \
             SER=$(adb devices | awk 'NR>1 && $2==\"device\"{{print $1; exit}}'); \
           else echo 'no Android device/emulator — create an AVD in Android Studio' >&2; exit 1; fi; \
         fi; \
         GR=./gradlew; [ -x \"$GR\" ] || GR=gradlew; \
         \"$GR\" :{module}:{install_task}; \
         PKG=$(grep -RhoE 'applicationId[ =\"]+[a-zA-Z0-9_.]+' {module}/build.gradle* 2>/dev/null | grep -oE '[a-zA-Z0-9_.]+$' | head -1); \
         if [ -n \"$PKG\" ]; then adb -s \"$SER\" shell monkey -p \"$PKG\" -c android.intent.category.LAUNCHER 1 || true; fi; \
         exec adb -s \"$SER\" logcat",
        pinned_serial = sq(&pinned_serial),
        module = module,
        install_task = install_task,
    )
}

// ---------------------------------------------------------------------------
// Flutter
// ---------------------------------------------------------------------------

/// `flutter run` stays attached for the session (hot-reload host). Honour a
/// pinned device and flavor when set.
fn flutter_command(cfg: &MobileRunConfig) -> String {
    let mut cmd = String::from("exec flutter run");
    if let Some(d) = cfg.device.as_deref().filter(|s| !s.is_empty()) {
        cmd.push_str(&format!(" -d {}", sq(d)));
    }
    if let Some(f) = cfg.flavor.as_deref().filter(|s| !s.is_empty()) {
        cmd.push_str(&format!(" --flavor {}", sq(f)));
    }
    if let Some(t) = cfg.target.as_deref().filter(|s| !s.is_empty()) {
        cmd.push_str(&format!(" -t {}", sq(t)));
    }
    cmd
}

// ---------------------------------------------------------------------------
// Expo
// ---------------------------------------------------------------------------

/// Expo's Metro dev server is itself long-running, so Play maps to it directly.
/// A pinned `device` of `ios`/`android` auto-opens that simulator on start.
fn expo_command(cfg: &MobileRunConfig) -> String {
    let mut cmd = String::from("exec npx expo start");
    match cfg.device.as_deref().map(str::trim) {
        Some("ios") => cmd.push_str(" --ios"),
        Some("android") => cmd.push_str(" --android"),
        _ => {}
    }
    cmd
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ProjectType;

    #[test]
    fn is_mobile_kind_covers_the_four_targets() {
        for k in [
            ProjectType::Xcode,
            ProjectType::Android,
            ProjectType::Flutter,
            ProjectType::Expo,
        ] {
            assert!(is_mobile_kind(k));
        }
        assert!(!is_mobile_kind(ProjectType::Next));
        assert!(!is_mobile_kind(ProjectType::Php));
    }

    #[test]
    fn ios_command_builds_installs_and_attaches_console() {
        let cmd = ios_command(&MobileRunConfig::default());
        assert!(cmd.contains("xcodebuild"));
        assert!(cmd.contains("simctl install"));
        assert!(cmd.contains("simctl launch --console-pty"));
        // auto device discovery present when none pinned
        assert!(cmd.contains("simctl list devices booted"));
    }

    #[test]
    fn ios_command_honours_pinned_scheme_and_device() {
        let cfg = MobileRunConfig {
            target: Some("App".into()),
            device: Some("ABC-123".into()),
            flavor: None,
        };
        let cmd = ios_command(&cfg);
        assert!(cmd.contains("SCHEME='App'"));
        assert!(cmd.contains("DEV='ABC-123'"));
    }

    #[test]
    fn android_command_installs_and_attaches_logcat() {
        let cmd = android_command(&MobileRunConfig::default());
        assert!(cmd.contains(":app:installDebug"));
        assert!(cmd.contains("adb -s \"$SER\" logcat"));
        assert!(cmd.contains("emulator -list-avds"));
    }

    #[test]
    fn android_flavor_selects_variant_install_task() {
        let cfg = MobileRunConfig {
            flavor: Some("staging".into()),
            target: None,
            device: None,
        };
        assert!(android_command(&cfg).contains("installStagingDebug"));
    }

    #[test]
    fn android_build_type_flavor_is_not_doubled() {
        // The folder detector passes flavor="debug" (the build *type*); it must
        // not become `installDebugDebug`.
        let cfg = MobileRunConfig {
            flavor: Some("debug".into()),
            target: None,
            device: None,
        };
        let cmd = android_command(&cfg);
        assert!(cmd.contains(":app:installDebug"));
        assert!(!cmd.contains("installDebugDebug"));
    }

    #[test]
    fn flutter_and_expo_are_long_running() {
        assert!(flutter_command(&MobileRunConfig::default()).starts_with("exec flutter run"));
        assert_eq!(
            expo_command(&MobileRunConfig::default()),
            "exec npx expo start"
        );
        let cfg = MobileRunConfig {
            device: Some("ios".into()),
            ..Default::default()
        };
        assert!(expo_command(&cfg).ends_with("--ios"));
    }
}
