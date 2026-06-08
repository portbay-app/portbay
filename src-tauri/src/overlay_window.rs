//! The notch dictation overlay window (macOS).
//!
//! A tiny always-on-top webview (`dictation-overlay` in tauri.conf.json)
//! that renders the FluidVoice-style notch HUD while a system-wide
//! dictation session runs: a black shape growing out of the camera notch
//! with the target app's icon, a live 5-bar waveform, and the streaming
//! transcript preview. The Svelte side lives at
//! `src/routes/dictation-overlay/+page.svelte`; this module owns the
//! native window behavior that a plain Tauri window can't express:
//!
//! - **Never steals focus**: the window ignores mouse events entirely
//!   (clicks fall through to the app below) and is shown with
//!   `orderFrontRegardless` — never `makeKey…` — so the target app keeps
//!   keyboard focus for the whole session. This is the property the
//!   dictation loop depends on: the transcript must paste into the app the
//!   user was in, and macOS dictation HUDs (and FluidVoice's NSPanel) work
//!   exactly this way.
//! - **Floats over everything**: screen-saver window level (above normal
//!   and floating windows), joins all Spaces, and is allowed next to
//!   full-screen apps (`fullScreenAuxiliary`) — dictation must work in a
//!   full-screen editor.
//! - **Anchors to the notch**: positioned top-center on the screen the
//!   pointer is on, sized from the real notch geometry (safe-area insets +
//!   auxiliary top areas, macOS 12+). Screens without a notch get the
//!   DynamicNotchKit fallback: an arbitrary 200 pt "virtual notch" the
//!   shape expands from under the menu bar.
//!
//! The window exists from app start (created by tauri.conf.json, hidden)
//! so showing it is an order-front, not a webview cold-start.

/// Label of the overlay webview. Must match `tauri.conf.json`
/// `app.windows[]` and `capabilities/dictation-overlay.json`.
pub const OVERLAY_WINDOW_LABEL: &str = "dictation-overlay";

/// Where the overlay sits on the pointer's screen — mirrors
/// `preferences.dictation.overlay_position` ("notch" | "bottom").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPlacement {
    /// Top-center, growing out of the camera housing (or the virtual-notch
    /// fallback under the menu bar).
    Notch,
    /// A floating pill near the bottom of the screen — the option for Macs
    /// without a notch.
    Bottom,
}

impl OverlayPlacement {
    pub fn from_pref(value: &str) -> Self {
        if value == "bottom" {
            Self::Bottom
        } else {
            Self::Notch
        }
    }
}

/// Geometry the Svelte overlay needs to draw the notch shape, resolved per
/// show (the user may have moved to another display). Logical points.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotchGeometry {
    /// Overlay window size (the shape is drawn centered inside it).
    pub window_width: f64,
    pub window_height: f64,
    /// Physical notch (or virtual fallback) size.
    pub notch_width: f64,
    pub notch_height: f64,
    /// False on screens without a camera housing — the shape then renders
    /// as a floating island under the menu bar instead of blending into
    /// real hardware.
    pub has_notch: bool,
    /// "notch" | "bottom" — which variant the webview draws (the native
    /// side already placed the window accordingly).
    pub placement: &'static str,
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::runtime::AnyObject;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen, NSWindowCollectionBehavior};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tauri::{AppHandle, Manager};

    use super::{NotchGeometry, OverlayPlacement, OVERLAY_WINDOW_LABEL};

    /// Extra width around the notch so the expanded shape has room for its
    /// corner curves and side padding (DynamicNotchKit's safe-area inset is
    /// 15 pt per side plus the 15 pt top-corner radii).
    const SIDE_ROOM: f64 = 110.0;
    /// Window height — tall enough for the expanded preview (60 pt) plus
    /// the control row, paddings, and the exit-animation overshoot.
    const WINDOW_HEIGHT: f64 = 200.0;
    /// Virtual notch for screens without a camera housing (DynamicNotchKit
    /// uses an arbitrary 300 pt; 200 reads better at our content width).
    const FALLBACK_NOTCH_WIDTH: f64 = 200.0;

    /// kCGScreenSaverWindowLevel — above floating panels and the Dock, the
    /// level freeflow's HUD uses so it shows over everything but the lock
    /// screen.
    const SCREEN_SAVER_LEVEL: isize = 1000;

    /// NSWindowSharingNone — the overlay never appears in screen
    /// recordings or shares (a dictation HUD over someone's screen-share
    /// is noise at best, a transcript leak at worst).
    const SHARING_NONE: usize = 0;

    /// Bottom placement: how far the pill floats above the visible frame's
    /// bottom edge (above the Dock when it's pinned there).
    const BOTTOM_OFFSET: f64 = 50.0;

    /// Resolve the overlay's NSWindow. The unsafe block is confined here.
    fn ns_window(app: &AppHandle) -> Option<(tauri::WebviewWindow, *mut AnyObject)> {
        let window = app.get_webview_window(OVERLAY_WINDOW_LABEL)?;
        let ptr = window.ns_window().ok()? as *mut AnyObject;
        if ptr.is_null() {
            return None;
        }
        Some((window, ptr))
    }

    /// One-time setup at app start (main thread): make the window inert.
    /// Mouse events pass through while hidden (`setIgnoresMouseEvents`;
    /// `show` flips this so the stop button is clickable during a session),
    /// it can never become key/main, floats at screen-saver level, follows
    /// the user to every Space, and may overlap full-screen apps.
    pub fn configure(app: &AppHandle) {
        let Some((window, ns)) = ns_window(app) else {
            tracing::warn!("dictation-overlay window not found — overlay disabled");
            return;
        };
        // Tauri-level: clicks fall through to whatever is below.
        let _ = window.set_ignore_cursor_events(true);
        unsafe {
            let _: () = objc2::msg_send![ns, setLevel: SCREEN_SAVER_LEVEL];
            let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary;
            let _: () = objc2::msg_send![ns, setCollectionBehavior: behavior];
            let _: () = objc2::msg_send![ns, setHasShadow: false];
            // Excluded from screen recordings and shares — the HUD is for
            // the person dictating, not their audience.
            let _: () = objc2::msg_send![ns, setSharingType: SHARING_NONE];
            // Clicking the overlay's stop button must not activate PortBay —
            // the dictation target keeps focus for the paste. Private but
            // long-stable AppKit (the NSPanel-nonactivating equivalent for a
            // plain NSWindow); probed so a future macOS that drops it
            // degrades to "click works, focus moves" — and the session-tap
            // paste's frontmost guard then rescues the transcript to the
            // clipboard rather than misfiring it into PortBay.
            // TRACK: re-verify this private selector each macOS major; if the
            // warning below ever fires in the wild, switch the overlay to a
            // borderless non-activating NSPanel (the supported equivalent).
            // (The paste itself goes through the session event tap, which
            // targets the frontmost app — so as long as the overlay doesn't
            // activate, the dictation target stays frontmost and keeps it.)
            let sel = objc2::sel!(_setPreventsActivation:);
            let responds: bool = objc2::msg_send![ns, respondsToSelector: sel];
            if responds {
                let _: () = objc2::msg_send![ns, _setPreventsActivation: true];
            } else {
                tracing::warn!(
                    "dictation: _setPreventsActivation unavailable — overlay clicks may move focus"
                );
            }
        }
        tracing::info!("dictation: notch overlay window configured");
    }

    /// Place the overlay on the screen the pointer is on — over the notch,
    /// or as a floating pill near the bottom — and order it front
    /// **without** activating PortBay. Main thread.
    pub fn show_on_pointer_screen(
        app: &AppHandle,
        mtm: MainThreadMarker,
        placement: OverlayPlacement,
    ) -> Option<NotchGeometry> {
        let (window, ns) = ns_window(app)?;
        let screen = pointer_screen(mtm)?;
        let frame = screen.frame();

        let geometry = match placement {
            OverlayPlacement::Notch => {
                let (notch_width, notch_height, has_notch) = notch_size(&screen);
                let width = notch_width + SIDE_ROOM * 2.0;
                let rect = NSRect::new(
                    NSPoint::new(
                        frame.origin.x + (frame.size.width - width) / 2.0,
                        frame.origin.y + frame.size.height - WINDOW_HEIGHT,
                    ),
                    NSSize::new(width, WINDOW_HEIGHT),
                );
                unsafe {
                    let _: () = objc2::msg_send![ns, setFrame: rect, display: false];
                }
                NotchGeometry {
                    window_width: width,
                    window_height: WINDOW_HEIGHT,
                    notch_width,
                    notch_height,
                    has_notch,
                    placement: "notch",
                }
            }
            OverlayPlacement::Bottom => {
                // Same window size as the notch variant (the webview is
                // transparent; the pill anchors to the stage's bottom) —
                // only the frame's origin differs: bottom-center of the
                // VISIBLE frame (above the Dock), offset upward, clamped so
                // a short screen can't push the window off the top.
                let visible = screen.visibleFrame();
                let width = FALLBACK_NOTCH_WIDTH + SIDE_ROOM * 2.0;
                let x = (frame.origin.x + (frame.size.width - width) / 2.0)
                    .max(visible.origin.x)
                    .min(visible.origin.x + (visible.size.width - width).max(0.0));
                let y = (visible.origin.y + BOTTOM_OFFSET).min(
                    (visible.origin.y + visible.size.height - WINDOW_HEIGHT)
                        .max(visible.origin.y),
                );
                let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(width, WINDOW_HEIGHT));
                unsafe {
                    let _: () = objc2::msg_send![ns, setFrame: rect, display: false];
                }
                NotchGeometry {
                    window_width: width,
                    window_height: WINDOW_HEIGHT,
                    notch_width: FALLBACK_NOTCH_WIDTH,
                    notch_height: 0.0,
                    has_notch: false,
                    placement: "bottom",
                }
            }
        };

        unsafe {
            let _: () = objc2::msg_send![ns, orderFrontRegardless];
        }
        // The stop button needs real clicks while the HUD is up. The window
        // never activates (configure's preventsActivation), so the dictation
        // target keeps focus; the strip around the notch eats clicks only
        // for the session's duration — hide() restores click-through.
        let _ = window.set_ignore_cursor_events(false);
        Some(geometry)
    }

    /// Order the overlay out. The Svelte side has already played its exit
    /// animation by the time this runs (the caller sequences that).
    pub fn hide(app: &AppHandle) {
        if let Some((window, ns)) = ns_window(app) {
            // Back to fully inert between sessions.
            let _ = window.set_ignore_cursor_events(true);
            unsafe {
                let nil: *mut AnyObject = std::ptr::null_mut();
                let _: () = objc2::msg_send![ns, orderOut: nil];
            }
        }
    }

    /// The screen under the mouse pointer (FluidVoice presents on it), with
    /// the main screen as fallback.
    fn pointer_screen(mtm: MainThreadMarker) -> Option<objc2::rc::Retained<NSScreen>> {
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        for screen in &screens {
            let f = screen.frame();
            if mouse.x >= f.origin.x
                && mouse.x < f.origin.x + f.size.width
                && mouse.y >= f.origin.y
                && mouse.y < f.origin.y + f.size.height
            {
                return Some(screen.clone());
            }
        }
        NSScreen::mainScreen(mtm).or_else(|| screens.iter().next())
    }

    /// Notch geometry of `screen` — DynamicNotchKit's math: width is the
    /// frame minus the two auxiliary top areas, height is the top safe-area
    /// inset. Screens without a housing get the virtual-notch fallback
    /// (menu-bar height tall).
    fn notch_size(screen: &NSScreen) -> (f64, f64, bool) {
        let frame = screen.frame();
        let visible = screen.visibleFrame();
        let menubar_height =
            (frame.origin.y + frame.size.height) - (visible.origin.y + visible.size.height);

        // safeAreaInsets + auxiliary areas are macOS 12+; probe before use
        // (the bundle still targets 11.0 even though the STT engine that
        // drives this feature needs 14+).
        let responds = |sel: objc2::runtime::Sel| -> bool {
            unsafe { objc2::msg_send![screen, respondsToSelector: sel] }
        };
        if responds(objc2::sel!(safeAreaInsets))
            && responds(objc2::sel!(auxiliaryTopLeftArea))
            && responds(objc2::sel!(auxiliaryTopRightArea))
        {
            let insets: objc2_foundation::NSEdgeInsets =
                unsafe { objc2::msg_send![screen, safeAreaInsets] };
            if insets.top > 0.0 {
                let left: NSRect = unsafe { objc2::msg_send![screen, auxiliaryTopLeftArea] };
                let right: NSRect = unsafe { objc2::msg_send![screen, auxiliaryTopRightArea] };
                let width = frame.size.width - left.size.width - right.size.width;
                if width > 0.0 && width < frame.size.width {
                    return (width, insets.top, true);
                }
            }
        }
        (FALLBACK_NOTCH_WIDTH, menubar_height.max(24.0), false)
    }
}

#[cfg(target_os = "macos")]
pub use macos::{configure, hide, show_on_pointer_screen};

#[cfg(test)]
mod tests {
    use super::OverlayPlacement;

    #[test]
    fn placement_from_pref_defaults_to_notch() {
        assert!(matches!(OverlayPlacement::from_pref("bottom"), OverlayPlacement::Bottom));
        assert!(matches!(OverlayPlacement::from_pref("notch"), OverlayPlacement::Notch));
        // Unknown / empty → notch (the default surface), never a panic.
        assert!(matches!(OverlayPlacement::from_pref(""), OverlayPlacement::Notch));
        assert!(matches!(OverlayPlacement::from_pref("weird"), OverlayPlacement::Notch));
    }
}
