//! Appearance-aware Dock icon (macOS).
//!
//! Apple's own light/dark app-icon variants are authored as a layered
//! Icon Composer `.icon` and rendered by the system per appearance. Tauri
//! can't bundle that yet (it ships a single static `.icns`, and `set_icon`
//! can't touch the Dock — see tauri-apps/tauri#14979, #14207). So we achieve
//! the same visible result by swapping `NSApplication`'s `applicationIconImage`
//! at runtime to match the system appearance:
//!
//!   - **Light** — the boat on a white squircle (same as the bundle icon).
//!   - **Dark**  — the boat on Apple's standard dark icon gradient,
//!     `#313131` (top) → `#141414` (bottom).
//!
//! Appearance is read straight from `NSApplication.effectiveAppearance` (not
//! Tauri's per-window `theme()`, which isn't reliably settled during `setup`
//! and wouldn't fire a change event when the app simply *launches* into Dark
//! mode). The swap only affects the running app's Dock tile; Finder/closed-app
//! still shows the bundled `.icns`. No-op on non-macOS.

#[cfg(target_os = "macos")]
const LIGHT_PNG: &[u8] = include_bytes!("appicon/light.png");
#[cfg(target_os = "macos")]
const DARK_PNG: &[u8] = include_bytes!("appicon/dark.png");

/// Does the running bundle carry the compiled Liquid Glass catalog?
///
/// `tauri build` copies `Assets.car` next to the executable at
/// `PortBay.app/Contents/Resources/Assets.car` (via `bundle.resources`). Its
/// presence means macOS 26+ will render the layered icon itself, so the runtime
/// swap should stand down. Under `tauri dev` the executable runs outside a full
/// bundle and the file is absent, so the swap takes over.
#[cfg(target_os = "macos")]
fn bundled_liquid_glass_icon_present() -> bool {
    std::env::current_exe()
        .ok()
        // …/Contents/MacOS/PortBay -> …/Contents/Resources/Assets.car
        .and_then(|exe| {
            exe.parent()
                .map(|macos_dir| macos_dir.join("../Resources/Assets.car"))
        })
        .map(|car| car.exists())
        .unwrap_or(false)
}

/// Set the Dock icon to match the current system appearance. Must run on the
/// main thread (Tauri's `setup` and window-event callbacks both do). Best-effort:
/// any failure leaves the bundled icon in place.
pub fn apply() {
    #[cfg(target_os = "macos")]
    {
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_app_kit::{
            NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSImage,
        };
        use objc2_foundation::{NSArray, NSData, NSProcessInfo};

        // macOS 26+ (Tahoe) renders the Liquid Glass `.icon` itself — the
        // compiled Assets.car + CFBundleIconName that `tauri build` copies into
        // Contents/Resources (registered via bundle.resources + Info.plist) —
        // including every appearance (Default/Dark/Clear/Tinted). When that
        // catalog is present, overriding the Dock tile with
        // `setApplicationIconImage` would clobber the system's
        // appearance-aware rendering, so we defer to the OS and return early.
        //
        // Under `tauri dev`, though, bundle.resources is NOT applied: there's
        // no Assets.car and no merged CFBundleIconName, so the OS falls back to
        // the static `.icns` (the light squircle) with no Dark variant. There's
        // nothing to clobber, so fall through to the runtime swap and give dev
        // the correct appearance-matched icon. See icons/macos-liquid-glass/README.md.
        if NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 26
            && bundled_liquid_glass_icon_present()
        {
            return;
        }

        let Some(mtm) = MainThreadMarker::new() else {
            tracing::warn!("dock_icon: not on main thread; leaving bundle icon");
            return;
        };
        let app = NSApplication::sharedApplication(mtm);

        // Resolve Light vs Dark from the app's effective appearance — the
        // authoritative signal, correct even at launch into Dark mode.
        let appearance = app.effectiveAppearance();
        // SAFETY: NSAppearanceNameAqua / NSAppearanceNameDarkAqua are AppKit's
        // own immutable global NSString constants; reading these extern statics
        // is sound.
        let (is_dark, appearance_name) = unsafe {
            let names = NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]);
            let dark = appearance
                .bestMatchFromAppearancesWithNames(&names)
                .is_some_and(|best| best.isEqualToString(NSAppearanceNameDarkAqua));
            (dark, appearance.name().to_string())
        };
        tracing::info!(is_dark, appearance = %appearance_name, "dock_icon: applying");

        let bytes: &[u8] = if is_dark { DARK_PNG } else { LIGHT_PNG };
        let data = NSData::with_bytes(bytes);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            tracing::warn!("dock_icon: NSImage init from PNG failed; leaving bundle icon");
            return;
        };
        unsafe {
            app.setApplicationIconImage(Some(&image));
        }
        tracing::info!(
            "dock_icon: set {} variant",
            if is_dark { "dark" } else { "light" }
        );
    }
}
