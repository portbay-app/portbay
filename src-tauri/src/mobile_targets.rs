//! Run-destination enumeration + toolchain pre-flight for mobile projects.
//!
//! This is the on-demand half of the mobile run UX: the launch scripts in
//! [`crate::mobile`] stay pure (no `simctl`/`adb` I/O at reconcile time), and
//! everything that *does* shell out to the platform tools lives here, invoked
//! only when the user opens the destination picker or the rail loads its
//! toolchain checks. Callers must run these off the async command workers
//! (`spawn_blocking`) — they're exactly the blocking-subprocess class that has
//! starved the async pool before.
//!
//! Targets are normalized into one [`RunTarget`] shape across all four kinds.
//! Target ids round-trip into `MobileRunConfig.device` and are interpreted by
//! the launch scripts:
//!   - iOS simulator → udid
//!   - Android       → adb serial, or `avd:<name>` for a not-yet-booted AVD
//!   - Flutter       → any of the above, or a flutter device id
//!   - Expo          → the pseudo-ids `ios` / `android` (Metro opens the sim)
//!
//! Physical iOS devices are enumerated (so the picker is honest about what's
//! plugged in) but flagged unsupported until the `devicectl` + signing flow
//! lands (plan: Phase 4).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::registry::ProjectType;

/// One run destination, normalized across platforms.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunTarget {
    /// What gets pinned into `MobileRunConfig.device` (udid / serial /
    /// `avd:<name>` / flutter device id / `ios`/`android` for Expo).
    pub id: String,
    pub name: String,
    /// `ios` | `android` | `macos` | `web` | `unknown`.
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// `simulator` | `emulator` | `physical` | `desktop` | `web`.
    pub kind: String,
    /// `booted` | `shutdown` | `connected`.
    pub state: String,
    /// Set when the target is listed but can't be run yet (e.g. physical iOS
    /// pending the signing flow, unauthorized Android device). The picker
    /// shows it disabled with this reason — listed-but-honest beats hidden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported_reason: Option<String>,
}

/// One toolchain pre-flight check for the rail's Checks section.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightCheck {
    pub label: String,
    pub ok: bool,
    /// Short status when ok; actionable fix when not.
    pub detail: String,
}

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

/// Enumerate run destinations for a project of `kind` at `path`. Blocking —
/// run on the blocking pool. The per-kind tool calls are independent
/// subprocesses, so each arm fans them out on scoped threads: wall-clock is
/// the slowest single tool, not their sum (the picker's "Scanning devices…"
/// used to be `flutter devices` + `simctl` + the adb chain back to back).
pub fn list_targets(kind: ProjectType, path: &Path) -> Vec<RunTarget> {
    match kind {
        ProjectType::Xcode => {
            let (mut out, physical) = std::thread::scope(|s| {
                let sims = s.spawn(ios_simulators);
                let phys = s.spawn(ios_physical_devices);
                (
                    sims.join().unwrap_or_default(),
                    phys.join().unwrap_or_default(),
                )
            });
            out.extend(physical);
            out
        }
        ProjectType::Android => android_targets(),
        ProjectType::Flutter => flutter_targets(path),
        ProjectType::Expo => expo_targets(),
        _ => Vec::new(),
    }
}

/// iOS simulators via `xcrun simctl list devices --json`.
fn ios_simulators() -> Vec<RunTarget> {
    let Some(json) = run_capture("xcrun", &["simctl", "list", "devices", "--json"]) else {
        return Vec::new();
    };
    parse_simctl_json(&json)
}

pub(crate) fn parse_simctl_json(json: &str) -> Vec<RunTarget> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(devices) = v.get("devices").and_then(|d| d.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (runtime, list) in devices {
        let os = runtime_label(runtime);
        let Some(list) = list.as_array() else {
            continue;
        };
        for d in list {
            let available = d
                .get("isAvailable")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            if !available {
                continue;
            }
            let (Some(name), Some(udid)) = (
                d.get("name").and_then(|s| s.as_str()),
                d.get("udid").and_then(|s| s.as_str()),
            ) else {
                continue;
            };
            let booted = d.get("state").and_then(|s| s.as_str()) == Some("Booted");
            out.push(RunTarget {
                id: udid.to_string(),
                name: name.to_string(),
                platform: platform_from_runtime(runtime),
                os_version: os.clone(),
                kind: "simulator".into(),
                state: if booted { "booted" } else { "shutdown" }.into(),
                unsupported_reason: None,
            });
        }
    }
    // Booted first, then by OS (newest runtimes tend to sort last — keep the
    // map order otherwise; the frontend groups anyway).
    out.sort_by_key(|t| (t.state != "booted", t.name.clone()));
    out
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-18-2` → `iOS 18.2`.
fn runtime_label(runtime: &str) -> Option<String> {
    let tail = runtime.rsplit("SimRuntime.").next()?;
    let mut parts = tail.splitn(2, '-');
    let os = parts.next()?;
    let ver = parts.next().map(|v| v.replace('-', "."));
    Some(match ver {
        Some(v) => format!("{os} {v}"),
        None => os.to_string(),
    })
}

fn platform_from_runtime(runtime: &str) -> String {
    let lc = runtime.to_ascii_lowercase();
    if lc.contains("watchos")
        || lc.contains("tvos")
        || lc.contains("xros")
        || lc.contains("visionos")
    {
        // Not iPhone-class, but still a valid xcodebuild destination.
        "ios".into()
    } else {
        "ios".into()
    }
}

/// Physical iOS devices via `devicectl` — listed but flagged unsupported until
/// the signing-aware install/launch path lands (Phase 4 of the plan). Output
/// goes through `--json-output` to a temp file (devicectl has no stdout JSON).
fn ios_physical_devices() -> Vec<RunTarget> {
    let tmp = std::env::temp_dir().join(format!("portbay-devicectl-{}.json", std::process::id()));
    // `--timeout 5`: devicectl otherwise waits out its full discovery window
    // when a known device is offline; attached/paired devices answer fast.
    let ok = Command::new("xcrun")
        .args([
            "devicectl",
            "list",
            "devices",
            "--timeout",
            "5",
            "--json-output",
        ])
        .arg(&tmp)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Vec::new();
    }
    let json = std::fs::read_to_string(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    parse_devicectl_json(&json)
}

pub(crate) fn parse_devicectl_json(json: &str) -> Vec<RunTarget> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(devices) = v.pointer("/result/devices").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    devices
        .iter()
        .filter_map(|d| {
            let name = d.pointer("/deviceProperties/name")?.as_str()?;
            let udid = d
                .pointer("/hardwareProperties/udid")
                .and_then(|u| u.as_str())
                .or_else(|| d.get("identifier").and_then(|u| u.as_str()))?;
            let os = d
                .pointer("/deviceProperties/osVersionNumber")
                .and_then(|o| o.as_str())
                .map(|o| format!("iOS {o}"));
            // Reachability, from observed devicectl JSON: a paired device is
            // runnable when its tunnel is up OR a transport (usb /
            // localNetwork) is present — devicectl brings the tunnel up on
            // demand. `tunnelState: unavailable` with no transport means the
            // device genuinely can't be reached right now.
            let paired = d
                .pointer("/connectionProperties/pairingState")
                .and_then(|t| t.as_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("paired"));
            let tunnel = d
                .pointer("/connectionProperties/tunnelState")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let has_transport = d
                .pointer("/connectionProperties/transportType")
                .and_then(|t| t.as_str())
                .is_some_and(|s| !s.is_empty());
            let reachable = paired && (tunnel.eq_ignore_ascii_case("connected") || has_transport);
            let unsupported_reason = if !paired {
                Some("Not paired — connect the device and pair it in Xcode first.".to_string())
            } else if !reachable {
                Some(
                    "Not reachable — connect by cable (unlock + Trust) or join the same \
                     Wi-Fi, then re-scan."
                        .to_string(),
                )
            } else {
                None
            };
            Some(RunTarget {
                id: udid.to_string(),
                name: name.to_string(),
                platform: "ios".into(),
                os_version: os,
                kind: "physical".into(),
                state: if reachable { "connected" } else { "shutdown" }.into(),
                unsupported_reason,
            })
        })
        .collect()
}

/// Android: connected devices/emulators (`adb devices -l`) + creatable AVDs
/// (`emulator -list-avds`), with booted AVDs deduped out of the AVD list.
/// The adb chain and the emulator listing are independent tools — run them
/// concurrently (the AVD dedup only joins the results at the end).
fn android_targets() -> Vec<RunTarget> {
    let ((mut out, booted_avds), avd_text) = std::thread::scope(|s| {
        let adb_side = s.spawn(|| {
            let adb = adb_bin();
            let out = match adb
                .as_deref()
                .and_then(|adb| run_capture(adb, &["devices", "-l"]))
            {
                Some(text) => parse_adb_devices(&text),
                None => Vec::new(),
            };
            // Which AVDs are already running (their emulator-* serial is listed)?
            let mut booted_avds: Vec<String> = Vec::new();
            if let Some(adb) = adb.as_deref() {
                for t in out.iter().filter(|t| t.id.starts_with("emulator-")) {
                    if let Some(name) = run_capture(adb, &["-s", &t.id, "emu", "avd", "name"]) {
                        if let Some(first) = name.lines().next() {
                            booted_avds.push(first.trim().to_string());
                        }
                    }
                }
            }
            (out, booted_avds)
        });
        let avds =
            s.spawn(|| emulator_bin().and_then(|emulator| run_capture(&emulator, &["-list-avds"])));
        (
            adb_side.join().unwrap_or_default(),
            avds.join().ok().flatten(),
        )
    });
    if let Some(text) = avd_text {
        out.extend(parse_avd_list(&text, &booted_avds));
    }
    out
}

pub(crate) fn parse_adb_devices(text: &str) -> Vec<RunTarget> {
    text.lines()
        .skip(1) // "List of devices attached"
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let serial = cols.next()?;
            let state = cols.next()?;
            if serial.is_empty() || state == "offline" {
                return None;
            }
            let rest: Vec<&str> = cols.collect();
            let model = rest
                .iter()
                .find_map(|c| c.strip_prefix("model:"))
                .map(|m| m.replace('_', " "));
            let is_emulator = serial.starts_with("emulator-");
            let unauthorized = state == "unauthorized";
            Some(RunTarget {
                id: serial.to_string(),
                name: model.unwrap_or_else(|| serial.to_string()),
                platform: "android".into(),
                os_version: None,
                kind: if is_emulator { "emulator" } else { "physical" }.into(),
                state: if is_emulator { "booted" } else { "connected" }.into(),
                unsupported_reason: unauthorized.then(|| {
                    "Unauthorized — accept the USB-debugging prompt on the device.".into()
                }),
            })
        })
        .collect()
}

pub(crate) fn parse_avd_list(text: &str, booted_avds: &[String]) -> Vec<RunTarget> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty() && !l.starts_with("INFO") && !l.starts_with("WARNING") && !l.contains('|')
            // emulator sometimes logs "... | informative" lines
        })
        .filter(|name| !booted_avds.iter().any(|b| b == name))
        .map(|name| RunTarget {
            id: format!("avd:{name}"),
            name: name.replace('_', " "),
            platform: "android".into(),
            os_version: None,
            kind: "emulator".into(),
            state: "shutdown".into(),
            unsupported_reason: None,
        })
        .collect()
}

/// Flutter: live devices from `flutter devices --machine`, plus bootable iOS
/// simulators and AVDs the flutter CLI doesn't list while they're shut down.
/// PortBay's launch script boots a pinned shutdown sim/AVD before `flutter run`.
///
/// `--device-timeout 5` halves flutter's default 10 s discovery wait — that
/// wait only benefits slow *networked* devices; cabled phones and booted
/// sims/emulators answer within it. The flag-less retry covers Flutter SDKs
/// old enough (< 2.5) to reject the flag. The three tools run concurrently:
/// the old serial chain (flutter → simctl → adb) was the picker's 10–15 s
/// "Scanning devices…".
fn flutter_targets(path: &Path) -> Vec<RunTarget> {
    let has_ios = path.join("ios").is_dir();
    let has_android = path.join("android").is_dir();
    let (mut out, sims, droids) = std::thread::scope(|s| {
        let flutter = s.spawn(|| {
            run_capture(
                "flutter",
                &["devices", "--machine", "--device-timeout", "5"],
            )
            .or_else(|| run_capture("flutter", &["devices", "--machine"]))
            .map(|json| parse_flutter_devices(&json))
            .unwrap_or_default()
        });
        // Shut-down iOS simulators / AVDs, only for sides the project has.
        let sims = s.spawn(move || {
            if has_ios {
                ios_simulators()
            } else {
                Vec::new()
            }
        });
        let droids = s.spawn(move || {
            if has_android {
                android_targets()
            } else {
                Vec::new()
            }
        });
        (
            flutter.join().unwrap_or_default(),
            sims.join().unwrap_or_default(),
            droids.join().unwrap_or_default(),
        )
    });
    let live_ids: Vec<String> = out.iter().map(|t| t.id.clone()).collect();
    out.extend(
        sims.into_iter()
            .filter(|t| t.state == "shutdown" && !live_ids.contains(&t.id)),
    );
    out.extend(droids.into_iter().filter(|t| t.id.starts_with("avd:")));
    out
}

pub(crate) fn parse_flutter_devices(json: &str) -> Vec<RunTarget> {
    // `flutter devices --machine` may print plain-text notes before the JSON
    // array (e.g. first-run banners); start at the first '['.
    let json = match json.find('[') {
        Some(i) => &json[i..],
        None => return Vec::new(),
    };
    let Ok(devices) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    devices
        .iter()
        .filter_map(|d| {
            let id = d.get("id")?.as_str()?;
            let name = d.get("name")?.as_str()?;
            let target = d
                .get("targetPlatform")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            let is_emulator = d.get("emulator").and_then(|e| e.as_bool()).unwrap_or(false);
            let sdk = d.get("sdk").and_then(|s| s.as_str()).map(str::to_string);
            let platform = if target.starts_with("android") {
                "android"
            } else if target.starts_with("ios") {
                "ios"
            } else if target.starts_with("darwin") {
                "macos"
            } else if target.starts_with("web") {
                "web"
            } else {
                "unknown"
            };
            let kind = match (platform, is_emulator) {
                ("ios", true) => "simulator",
                ("android", true) => "emulator",
                ("macos", _) => "desktop",
                ("web", _) => "web",
                _ => "physical",
            };
            Some(RunTarget {
                id: id.to_string(),
                name: name.to_string(),
                platform: platform.into(),
                os_version: sdk,
                kind: kind.into(),
                // flutter only lists targets that are runnable right now.
                state: if is_emulator { "booted" } else { "connected" }.into(),
                unsupported_reason: None,
            })
        })
        .collect()
}

/// Expo's launch pins are the Metro `--ios` / `--android` switches, so the
/// picker offers exactly those two destinations, with live boot state derived
/// from what's actually running.
fn expo_targets() -> Vec<RunTarget> {
    let ios_booted = ios_simulators().iter().any(|t| t.state == "booted");
    let android_booted = adb_bin()
        .as_deref()
        .and_then(|adb| run_capture(adb, &["devices"]))
        .map(|t| {
            t.lines()
                .skip(1)
                .any(|l| l.split_whitespace().nth(1) == Some("device"))
        })
        .unwrap_or(false);
    vec![
        RunTarget {
            id: "ios".into(),
            name: "iOS Simulator".into(),
            platform: "ios".into(),
            os_version: None,
            kind: "simulator".into(),
            state: if ios_booted { "booted" } else { "shutdown" }.into(),
            unsupported_reason: None,
        },
        RunTarget {
            id: "android".into(),
            name: "Android Emulator".into(),
            platform: "android".into(),
            os_version: None,
            kind: "emulator".into(),
            state: if android_booted { "booted" } else { "shutdown" }.into(),
            unsupported_reason: None,
        },
    ]
}

// ---------------------------------------------------------------------------
// Pre-flight
// ---------------------------------------------------------------------------

/// Toolchain checks for the rail's Checks section — actionable error states
/// instead of a crashed run with a raw log. Blocking; run on the blocking pool.
pub fn preflight(kind: ProjectType, path: &Path) -> Vec<PreflightCheck> {
    match kind {
        ProjectType::Xcode => ios_preflight(),
        ProjectType::Android => android_preflight(path),
        ProjectType::Flutter => {
            let mut checks = vec![check_flutter()];
            if path.join("ios").is_dir() {
                checks.extend(ios_preflight());
            }
            if path.join("android").is_dir() {
                checks.push(check_adb());
            }
            checks
        }
        ProjectType::Expo => vec![
            check_bin("node", "Node.js", "Install Node.js — Metro needs it."),
            PreflightCheck {
                label: "Expo project".into(),
                ok: path.join("package.json").exists(),
                detail: if path.join("package.json").exists() {
                    "package.json present".into()
                } else {
                    "No package.json found in the project folder.".into()
                },
            },
        ],
        _ => Vec::new(),
    }
}

fn ios_preflight() -> Vec<PreflightCheck> {
    let xcode = run_capture("xcode-select", &["-p"]);
    let xcode_ok = xcode.as_deref().is_some_and(|p| !p.trim().is_empty());
    let sims = ios_simulators();
    let has_sim = sims.iter().any(|t| t.kind == "simulator");
    vec![
        PreflightCheck {
            label: "Xcode tools".into(),
            ok: xcode_ok,
            detail: if xcode_ok {
                xcode.unwrap_or_default().trim().to_string()
            } else {
                "Run `xcode-select --install` or install Xcode.".into()
            },
        },
        PreflightCheck {
            label: "iOS simulators".into(),
            ok: has_sim,
            detail: if has_sim {
                format!(
                    "{} available",
                    sims.iter().filter(|t| t.kind == "simulator").count()
                )
            } else {
                "No simulators — create one in Xcode → Devices & Simulators.".into()
            },
        },
    ]
}

fn android_preflight(path: &Path) -> Vec<PreflightCheck> {
    let mut checks = vec![check_adb()];
    let targets = android_targets();
    let has_target = !targets.is_empty();
    checks.push(PreflightCheck {
        label: "Device or AVD".into(),
        ok: has_target,
        detail: if has_target {
            format!("{} available", targets.len())
        } else {
            "No device/emulator — create an AVD in Android Studio.".into()
        },
    });
    let gradlew = path.join("gradlew").exists();
    checks.push(PreflightCheck {
        label: "Gradle wrapper".into(),
        ok: gradlew,
        detail: if gradlew {
            "gradlew present".into()
        } else {
            "No ./gradlew in the project — builds need the wrapper.".into()
        },
    });
    checks
}

fn check_flutter() -> PreflightCheck {
    check_bin(
        "flutter",
        "Flutter CLI",
        "Install Flutter and add it to PATH.",
    )
}

fn check_adb() -> PreflightCheck {
    let ok = adb_bin().is_some();
    PreflightCheck {
        label: "adb".into(),
        ok,
        detail: if ok {
            "Found".into()
        } else {
            "Install Android platform-tools (adb) and add them to PATH.".into()
        },
    }
}

fn check_bin(bin: &str, label: &str, fix: &str) -> PreflightCheck {
    let ok = which(bin).is_some();
    PreflightCheck {
        label: label.into(),
        ok,
        detail: if ok { "Found".into() } else { fix.into() },
    }
}

// ---------------------------------------------------------------------------
// Tool discovery / subprocess plumbing
// ---------------------------------------------------------------------------

/// `adb`, resolved from PATH or the default Android SDK location. The launch
/// scripts assume PATH (Process Compose runs a login-ish env); enumeration is
/// more forgiving so the picker works even when the GUI app's PATH is minimal.
pub(crate) fn adb_bin() -> Option<String> {
    resolve_tool("adb", &["platform-tools/adb"])
}

fn emulator_bin() -> Option<String> {
    resolve_tool("emulator", &["emulator/emulator"])
}

fn resolve_tool(bin: &str, sdk_relative: &[&str]) -> Option<String> {
    if which(bin).is_some() {
        return Some(bin.to_string());
    }
    for root in android_sdk_roots() {
        for rel in sdk_relative {
            let candidate = root.join(rel);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn android_sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                roots.push(PathBuf::from(v));
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Library/Android/sdk"));
    }
    roots
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join(bin))
        .find(|candidate| candidate.is_file())
}

/// Run a tool and capture stdout on success. Stderr is discarded; a non-zero
/// exit or spawn failure returns None (the caller treats it as "no targets").
fn run_capture(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simctl_json_parses_booted_and_shutdown() {
        let json = r#"{
          "devices": {
            "com.apple.CoreSimulator.SimRuntime.iOS-18-2": [
              {"name":"iPhone 16","udid":"AAAA-1111","state":"Booted","isAvailable":true},
              {"name":"iPhone SE (3rd generation)","udid":"BBBB-2222","state":"Shutdown","isAvailable":true},
              {"name":"Broken","udid":"CCCC-3333","state":"Shutdown","isAvailable":false}
            ]
          }
        }"#;
        let t = parse_simctl_json(json);
        assert_eq!(t.len(), 2); // unavailable filtered out
        assert_eq!(t[0].name, "iPhone 16");
        assert_eq!(t[0].state, "booted");
        assert_eq!(t[0].os_version.as_deref(), Some("iOS 18.2"));
        assert_eq!(t[1].state, "shutdown");
    }

    #[test]
    fn adb_devices_parses_emulator_and_physical() {
        let text = "List of devices attached\n\
                    emulator-5554          device product:sdk_gphone64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
                    R5CT1234ABC            device usb:1-1 product:beyond1 model:SM_G973F device:beyond1 transport_id:2\n\
                    R5CTUNAUTH             unauthorized usb:1-2 transport_id:3\n";
        let t = parse_adb_devices(text);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].kind, "emulator");
        assert_eq!(t[0].state, "booted");
        assert_eq!(t[1].kind, "physical");
        assert_eq!(t[1].name, "SM G973F");
        assert_eq!(t[1].state, "connected");
        assert!(t[2].unsupported_reason.is_some());
    }

    #[test]
    fn avd_list_dedupes_booted_avds_and_prefixes_ids() {
        let text = "Pixel_8_API_35\nPixel_Tablet_API_34\n";
        let t = parse_avd_list(text, &["Pixel_8_API_35".to_string()]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "avd:Pixel_Tablet_API_34");
        assert_eq!(t[0].name, "Pixel Tablet API 34");
        assert_eq!(t[0].state, "shutdown");
    }

    #[test]
    fn flutter_devices_parses_machine_json_with_banner_noise() {
        let json = r#"Some first-run banner
[
  {"name":"iPhone 16","id":"AAAA-1111","isSupported":true,"targetPlatform":"ios","emulator":true,"sdk":"iOS 18.2"},
  {"name":"macOS","id":"macos","isSupported":true,"targetPlatform":"darwin-arm64","emulator":false,"sdk":"macOS 15"},
  {"name":"sdk gphone64","id":"emulator-5554","isSupported":true,"targetPlatform":"android-arm64","emulator":true,"sdk":"Android 14"}
]"#;
        let t = parse_flutter_devices(json);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].kind, "simulator");
        assert_eq!(t[1].kind, "desktop");
        assert_eq!(t[1].platform, "macos");
        assert_eq!(t[2].kind, "emulator");
        assert_eq!(t[2].platform, "android");
    }

    #[test]
    fn devicectl_reachable_devices_are_runnable() {
        // Paired + transport present (tunnel comes up on demand) ⇒ runnable.
        // This is the exact shape a paired Wi-Fi iPhone reports.
        let json = r#"{"result":{"devices":[{
            "identifier":"X-1",
            "deviceProperties":{"name":"Nour’s iPhone","osVersionNumber":"18.5"},
            "hardwareProperties":{"udid":"00008120-XYZ"},
            "connectionProperties":{"tunnelState":"disconnected","pairingState":"paired","transportType":"localNetwork"}
        }]}}"#;
        let t = parse_devicectl_json(json);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].kind, "physical");
        assert_eq!(t[0].state, "connected");
        assert_eq!(t[0].id, "00008120-XYZ");
        assert!(t[0].unsupported_reason.is_none());
    }

    #[test]
    fn devicectl_unreachable_devices_are_listed_but_flagged() {
        let json = r#"{"result":{"devices":[{
            "identifier":"X-2",
            "deviceProperties":{"name":"Nour’s iPad","osVersionNumber":"26.1"},
            "hardwareProperties":{"udid":"00008132-ABC"},
            "connectionProperties":{"tunnelState":"unavailable","pairingState":"paired"}
        }]}}"#;
        let t = parse_devicectl_json(json);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].state, "shutdown");
        assert!(t[0]
            .unsupported_reason
            .as_deref()
            .is_some_and(|r| r.contains("Not reachable")));
    }

    #[test]
    fn runtime_label_formats_versions() {
        assert_eq!(
            runtime_label("com.apple.CoreSimulator.SimRuntime.iOS-18-2").as_deref(),
            Some("iOS 18.2")
        );
        assert_eq!(
            runtime_label("com.apple.CoreSimulator.SimRuntime.watchOS-11-0").as_deref(),
            Some("watchOS 11.0")
        );
    }
}
