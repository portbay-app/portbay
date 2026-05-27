//! Platform-specific window vibrancy for PortBay's translucent shell.
//!
//! PortBay's window is a "liquid glass" surface: the OS compositor renders a
//! blurred backdrop *behind* a transparent webview, and the CSS layers only
//! subtle, cheap overlays on top — deliberately **no heavy `backdrop-filter`**.
//! Doing the blur in the compositor (not WebKit) is what makes the window feel
//! native and keeps navigation repaints flash-free.
//!
//! ## Platform status
//!
//! - **macOS** — real implementation. A native `NSVisualEffectView` (the
//!   `Sidebar` material) sits behind the whole window shell; the tray popover
//!   uses the denser `HudWindow` material. See [`macos`].
//! - **Windows** — placeholder only. The seam for future **Mica** lives in
//!   [`windows::apply_main`]. We intentionally do **not** enable Acrylic / blur
//!   by default (washed-out and power-hungry on Win11). Until Mica is wired up
//!   the window keeps its plain themed background.
//! - **Linux** — intentionally left as-is: no compositor vibrancy. The plain
//!   themed background is the supported look. See [`linux::apply_main`].
//!
//! Every entry point is safe to call on every platform — non-macOS targets are
//! currently no-ops, so callers (`lib.rs` setup) don't need their own `cfg`s.

use tauri::WebviewWindow;

/// Apply the main window's shell vibrancy.
///
/// macOS gets a real `NSVisualEffectView`; other platforms fall back to the
/// plain themed window background (see the module docs).
pub fn apply_main(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    macos::apply_main(window);
    #[cfg(target_os = "windows")]
    windows::apply_main(window);
    #[cfg(target_os = "linux")]
    linux::apply_main(window);
    // Any other target (BSDs, etc.): no vibrancy, but keep `window` "used".
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let _ = window;
}

/// Apply the tray popover's vibrancy (a denser HUD-style blur on macOS).
///
/// Only macOS implements this today; elsewhere the popover uses its themed
/// background.
pub fn apply_tray_panel(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    macos::apply_tray_panel(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

#[cfg(target_os = "macos")]
mod macos {
    use tauri::WebviewWindow;
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};

    /// `Sidebar` is the lightest, most "system-chrome" material — it matches
    /// the Finder / Mail sidebar look and reads cleanly behind PortBay's
    /// translucent surfaces without the heavier tint of `Menu` / `HudWindow`.
    pub fn apply_main(window: &WebviewWindow) {
        // `FollowsWindowActiveState` dims the blur when the window loses focus,
        // matching the native behaviour of a system sidebar.
        if let Err(e) = apply_vibrancy(
            window,
            NSVisualEffectMaterial::Sidebar,
            Some(NSVisualEffectState::FollowsWindowActiveState),
            None,
        ) {
            tracing::warn!(error = %e, "failed to apply main-window vibrancy");
        }
    }

    /// The tray popover floats over arbitrary desktop content, so it uses the
    /// denser `HudWindow` material for legibility.
    pub fn apply_tray_panel(window: &WebviewWindow) {
        if let Err(e) = apply_vibrancy(window, NSVisualEffectMaterial::HudWindow, None, None) {
            tracing::warn!(error = %e, "failed to apply tray-panel vibrancy");
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use tauri::WebviewWindow;

    /// Placeholder for future **Mica** support (Windows 11 22H2+).
    ///
    /// TODO(windows-vibrancy): apply Mica here, e.g.
    /// `let _ = window_vibrancy::apply_mica(window, None);`, once it's been
    /// validated against PortBay's CSS surfaces and a graceful Windows 10
    /// fallback is in place. We deliberately avoid Acrylic / blur-behind by
    /// default — it reads washed-out and is power-hungry. Until then the window
    /// keeps its plain themed background.
    pub fn apply_main(_window: &WebviewWindow) {}
}

#[cfg(target_os = "linux")]
mod linux {
    use tauri::WebviewWindow;

    /// Linux has no supported compositor-vibrancy path in PortBay today; the
    /// plain themed background is the intended look.
    ///
    /// TODO(linux-vibrancy): if we ever target a specific desktop environment
    /// with a blur protocol (e.g. KDE's `kwin` blur), wire it here. Left as a
    /// deliberate no-op.
    pub fn apply_main(_window: &WebviewWindow) {}
}
