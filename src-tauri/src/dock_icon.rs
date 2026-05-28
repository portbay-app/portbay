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
        // compiled Assets.car + CFBundleIconName injected by
        // scripts/inject-macos-liquid-glass-icon.sh — including every
        // appearance (Default/Dark/Clear/Tinted). Overriding the Dock tile
        // with `setApplicationIconImage` here would clobber it, so the runtime
        // swap is scoped to macOS 11–15, which has no Liquid Glass and shows
        // the static bundle icon. See icons/macos-liquid-glass/README.md.
        if NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 26
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
