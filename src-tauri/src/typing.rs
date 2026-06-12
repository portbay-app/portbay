//! System-wide transcript insertion for "dictate anywhere" (macOS).
//!
//! When the local STT engine finishes a system-wide capture, the final
//! transcript has to land in **whatever app the user was dictating into** —
//! PortBay is not frontmost and there is no field of ours to splice into.
//! This module is that delivery path, a Rust port of the "reliable paste"
//! mode every shipping dictation tool converges on (FluidVoice's
//! TypingService, freeflow's AppState insertion):
//!
//! 1. Snapshot the general pasteboard (string contents + change count).
//! 2. Write the transcript, alongside `org.nspasteboard.TransientType` so
//!    clipboard managers (Maccy, Raycast, Paste) skip recording it.
//! 3. Synthesize ⌘V with `CGEventCreateKeyboardEvent` and post it into the
//!    **session event tap** (`CGEventPost`) — the level a real keypress
//!    enters at, so Chromium-family web editors (ChatGPT's ProseMirror in
//!    Chrome/Brave/Electron) accept the paste, not just native AppKit
//!    fields. The session tap delivers to the frontmost app, so the pid
//!    captured at Fn-down is re-checked first: an app switch mid-dictation
//!    rescues the words to the clipboard instead of pasting into the wrong
//!    window.
//! 4. After the app has had time to consume the paste, restore the user's
//!    original clipboard — only if nothing else wrote to it in between
//!    (change-count check).
//!
//! Posting keyboard events to another app requires the user to grant
//! PortBay **Accessibility** (AXIsProcessTrusted) — the same TCC gate every
//! dictation app shows on first run. [`ax_trusted`] is the probe; granting
//! flows through the existing MacPermissionDialog drag-to-grant sheet.
//!
//! Everything here is local: the transcript goes to the pasteboard and into
//! the target app, never anywhere else.

/// Pure decision core for `replace_recent_insertion_via_keys` — the parts of
/// the keyed fallback that decide whether any synthetic key is ever posted,
/// when the ⇧← burst pauses to re-verify focus, and whether the bail-out
/// collapse → is safe to post. Extracted from the FFI path (self-documented
/// as the most fragile in the module) so the guards are unit-testable
/// without a window server.
pub(crate) mod keyed_guard {
    /// Ceiling on the keyed replace's select-back span: each character costs
    /// one synthetic ⇧← pair, so a very long transcript would visibly crawl.
    /// Above this the raw words simply stay (the AX path already covered
    /// native fields of any length).
    pub const MAX_CHARS: usize = 500;
    /// ⇧← burst batch size. Batches keep the burst from coalescing in the
    /// target's queue, and every batch boundary re-probes the frontmost pid:
    /// the events go to the HID tap (frontmost app, not a specific pid), so
    /// a focus switch mid-burst must abort before the next batch lands in
    /// whatever window is now in front.
    pub const BATCH: usize = 32;

    /// Chars to select back over, or None when the span is empty or too long
    /// — both bail before any key is posted. Arrow keys move per character/
    /// grapheme; `chars()` is the right unit for plain dictation text, and
    /// the read-back verify catches any editor whose caret strides differ
    /// (clusters, soft breaks).
    pub fn span_chars(old: &str) -> Option<usize> {
        let n = old.chars().count();
        (n > 0 && n <= MAX_CHARS).then_some(n)
    }

    /// True after the `i`-th ⇧← pair (0-based) when the burst should pause
    /// for a breather and a focus re-probe.
    pub fn batch_boundary(i: usize) -> bool {
        (i + 1).is_multiple_of(BATCH)
    }

    /// Whether the probed frontmost pid is still the captured target — the
    /// gate for continuing the burst and for posting the bail collapse →.
    /// A failed probe (None) counts as lost: posting blind is never safe.
    pub fn focus_still_target(front: Option<i32>, target_pid: i32) -> bool {
        front == Some(target_pid)
    }

    /// The selection read-back proves we selected exactly what we typed:
    /// every ⇧← was posted AND AXSelectedText returns precisely `old`.
    /// An unreadable selection (None) is a fail — never paste over an
    /// unverified selection.
    pub fn selection_verified(
        posted: usize,
        count: usize,
        selected: Option<&str>,
        old: &str,
    ) -> bool {
        posted == count && selected == Some(old)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_ulong, c_void};

    use super::keyed_guard;

    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplicationActivationPolicy, NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard,
        NSPasteboardTypeString, NSRunningApplication, NSWorkspace,
    };
    use objc2_foundation::{NSData, NSDictionary, NSString};
    use serde::Serialize;

    // ---------------------------------------------------------------------
    // Accessibility trust
    // ---------------------------------------------------------------------

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        /// CFStringRef constant — the "show the system prompt" option key.
        static kAXTrustedCheckOptionPrompt: *const c_void;

        // Accessibility direct-insert fallback (FluidVoice's deepest layer):
        // write the transcript straight into the focused element's value via
        // the AX API — a channel entirely separate from synthesized events, so
        // it reaches the odd app that ignores both typing and ⌘V.
        fn AXUIElementCreateSystemWide() -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void, // CFStringRef
            value: *mut *const c_void,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: *const c_void,
            attribute: *const c_void, // CFStringRef
            value: *const c_void,
        ) -> i32;
        /// Extract a primitive (here a CFRange) out of an AXValue wrapper.
        fn AXValueGetValue(value: *const c_void, the_type: u32, value_ptr: *mut c_void) -> bool;
        /// Wrap a primitive (a CFRange) back into an AXValue.
        fn AXValueCreate(the_type: u32, value_ptr: *const c_void) -> *const c_void;
    }

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        /// Whether ANY process holds Secure Event Input (password dialogs,
        /// password managers, the login window). While it's held, keyboards
        /// are cryptographically isolated — a transcript must not be typed.
        fn IsSecureEventInputEnabled() -> bool;
    }

    /// kAXValueTypeCFRange — the AXValue tag for a CFRange (selected-text range).
    const KAX_VALUE_TYPE_CFRANGE: u32 = 4;
    /// kAXErrorSuccess.
    const KAX_ERROR_SUCCESS: i32 = 0;

    /// CoreFoundation `CFRange` (CFIndex == signed long == isize on 64-bit).
    /// AX selected-text ranges are expressed in UTF-16 code units.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CFRange {
        location: isize,
        length: isize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *mut *const c_void,
            values: *mut *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
    }

    /// Whether this process may synthesize keyboard events into other apps
    /// (System Settings → Privacy & Security → Accessibility).
    pub fn ax_trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    /// Probe trust AND, when missing, fire macOS's own Accessibility prompt
    /// — the dialog that *registers PortBay in the Accessibility list*, so
    /// the user only has to flip the switch (no manual drag). Called when
    /// the user enables "Dictate anywhere", never at launch (a surprise
    /// permission prompt is the one thing every TCC guideline forbids).
    /// Main thread. Returns the current trust state (the prompt itself
    /// resolves later, in System Settings).
    pub fn ax_prompt(mtm: MainThreadMarker) -> bool {
        let _ = mtm;
        if ax_trusted() {
            return true;
        }
        unsafe {
            // NULL callbacks: the one key is a framework constant that
            // outlives the call, and AX matches it by pointer identity.
            let mut keys = [kAXTrustedCheckOptionPrompt];
            let mut values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_mut_ptr(),
                values.as_mut_ptr(),
                1,
                std::ptr::null(),
                std::ptr::null(),
            );
            let trusted = AXIsProcessTrustedWithOptions(options);
            if !options.is_null() {
                CFRelease(options);
            }
            trusted
        }
    }

    // ---------------------------------------------------------------------
    // CGEvent FFI — keyboard synthesis
    // ---------------------------------------------------------------------

    type CGEventRef = *mut c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        /// Create an event source in a given state. We use it so our synthetic
        /// ⌘V carries a real source identity (see `EVENT_SOURCE_COMBINED`),
        /// the way a hardware keypress does — the NULL source the events were
        /// built with reaches native Cocoa fields (the omnibox) but Chromium's
        /// renderer applies stricter validation and can drop a sourceless key
        /// event, which is the difference between the omnibox accepting the
        /// paste and ChatGPT/Gmail's contenteditable ignoring it. Returns a
        /// CFTypeRef the caller must `CFRelease`; NULL on failure (we then post
        /// sourceless, the prior behavior).
        fn CGEventSourceCreate(state_id: i32) -> *const c_void;
        /// Suppress the user's *physical* input for the brief interval around
        /// our synthetic post, so a key they're holding (the Fn/globe key, in
        /// hold-to-dictate mode) is NOT merged into the synthetic ⌘V. Without
        /// this the posted ⌘V picks up the live Fn flag and arrives as Fn+⌘V —
        /// an unknown shortcut most apps reject with the system beep, which is
        /// the "omnibox pastes, Word/Gmail/ChatGPT beep" bug (the omnibox is
        /// just lenient about the stray modifier). The proven paste recipe
        /// (Maccy) sets this on its source.
        fn CGEventSourceSetLocalEventsFilterDuringSuppressionState(
            source: *const c_void,
            filter: u32,
            state: u32,
        );
        fn CGEventCreateKeyboardEvent(
            source: *const c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
        fn CGEventKeyboardSetUnicodeString(
            event: CGEventRef,
            string_length: c_ulong,
            unicode_string: *const u16,
        );
        fn CGEventPost(tap: u32, event: CGEventRef);
        /// Post an event straight to a specific process's event queue rather
        /// than the global event stream — FluidVoice's primary delivery for
        /// typed text. Reaches the target app's focused field directly.
        fn CGEventPostToPid(pid: i32, event: CGEventRef);
        fn CFRelease(cf: *const c_void);
    }

    /// Max UTF-16 code units per synthesized typing event. `CGEventKeyboard
    /// SetUnicodeString` truncates very long strings in some apps, so a long
    /// transcript is typed in chunks of this size. 20 is the widely-cited
    /// reliable ceiling.
    const UNICODE_CHUNK: usize = 20;

    /// kCGEventFlagMaskCommand.
    const MASK_COMMAND: u64 = 1 << 20;
    /// kVK_ANSI_V — the physical V key on ANSI layouts. The unicode payload
    /// below carries the literal "v" so non-QWERTY layouts still read the
    /// event as ⌘V (apps match key equivalents by character).
    const KEY_V: u16 = 9;
    /// kVK_LeftArrow / kVK_RightArrow — the keyed replace's select-back and
    /// its bail-out collapse.
    const KEY_LEFT: u16 = 123;
    const KEY_RIGHT: u16 = 124;
    /// kCGEventFlagMaskShift.
    const MASK_SHIFT: u64 = 1 << 17;
    /// kCGEventSourceStateCombinedSessionState — the source state a real
    /// keypress reports. Shipping clipboard/paste tools (e.g. Maccy) build
    /// their synthetic ⌘V from a source in this state.
    const EVENT_SOURCE_COMBINED: i32 = 0;
    /// Suppression filter applied to the source:
    /// kCGEventFilterMaskPermitLocalMouseEvents (1) |
    /// kCGEventFilterMaskPermitSystemDefinedEvents (4). It deliberately OMITS
    /// kCGEventFilterMaskPermitLocalKeyboardEvents (2) — so during injection
    /// the user's held keyboard keys (the Fn/globe held to dictate) are
    /// suppressed and can't contaminate the synthetic ⌘V.
    const SUPPRESS_FILTER: u32 = 1 | 4;
    /// kCGEventSuppressionStateSuppressionInterval — apply the filter for the
    /// short interval around the post (not the remote-mouse-drag state).
    const SUPPRESS_STATE: u32 = 0;

    /// Owns a `CGEventSourceRef` for the duration of one paste. Wraps the raw
    /// pointer so the value can be held across the `tokio::sleep`s between the
    /// posted key events (a bare `*const c_void` is `!Send` and would make the
    /// spawned `insert_text` future non-`Send`); the pointer is only ever
    /// touched on this one task and CGEvent posting is thread-safe, so the
    /// `Send` assertion is sound. `Drop` releases it on every exit path,
    /// including an early `?` from a failed `post_key`.
    struct EventSource(*const c_void);
    unsafe impl Send for EventSource {}
    impl Drop for EventSource {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }
    /// kCGHIDEventTap — inject the synthetic ⌘V at the point where HID (real
    /// hardware) events enter the window server: the lowest, most universal
    /// level, honored uniformly by every app's input path. The session taps we
    /// used before (plain `kCGSessionEventTap` and the annotated variant) sit
    /// HIGHER and are "for sending events to specific applications" — which is
    /// exactly why AppKit's omnibox honored them but Word's text engine and the
    /// web renderers (Gmail/ChatGPT) silently ignored the ⌘V. HID-level events
    /// reach the frontmost app the same way a keyboard does, so `insert_text`
    /// confirms the captured target is still frontmost before posting.
    const HID_EVENT_TAP: u32 = 0;

    /// Post one synthetic key event into the annotated session event tap.
    /// `flags` is the modifier mask in effect; `v_char` stamps the literal "v"
    /// payload so the event reads as ⌘V on non-QWERTY layouts. Split into
    /// key-down / key-up so the inter-event delay can be an async
    /// `tokio::sleep` in `insert_text` (not a blocking sleep on the shared
    /// Tokio worker). Returns Err only when the event could not be created —
    /// posting has no failure signal.
    fn post_key(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
        flags: u64,
        v_char: bool,
    ) -> Result<(), String> {
        unsafe {
            let ev = CGEventCreateKeyboardEvent(source, virtual_key, key_down);
            if ev.is_null() {
                return Err("could not create keyboard event".into());
            }
            CGEventSetFlags(ev, flags);
            if v_char {
                let v: [u16; 1] = ['v' as u16];
                CGEventKeyboardSetUnicodeString(ev, 1, v.as_ptr());
            }
            CGEventPost(HID_EVENT_TAP, ev);
            CFRelease(ev);
        }
        Ok(())
    }

    /// Pid of the frontmost app right now — the lightweight check
    /// `insert_text` runs before a session-tap paste (no icon encoding,
    /// unlike `capture_front_target`). Main thread (NSWorkspace).
    fn front_pid(mtm: MainThreadMarker) -> Option<i32> {
        let _ = mtm;
        let workspace = NSWorkspace::sharedWorkspace();
        Some(workspace.frontmostApplication()?.processIdentifier())
    }

    /// Re-activate the app `pid` (make its window key). Called right before the
    /// paste: a web editor (ChatGPT's ProseMirror in Chrome/Brave) BLURS its
    /// focused element when our notch orders in front, so the ⌘V has no
    /// editable to land in even though the keystroke arrives — re-activating
    /// restores the contenteditable's DOM focus. The target is already
    /// frontmost (the guard just checked), so this is seamless: no app switch,
    /// it only re-keys the window. Native fields don't need it, but it's
    /// harmless there. Main thread (NSRunningApplication). `1 << 1` is
    /// NSApplicationActivateIgnoringOtherApps (raw msg_send to dodge the
    /// typed API's macOS-14 deprecation lint).
    ///
    /// Currently unused: `insert_text` stopped re-activating the target,
    /// because the re-activation itself reset web-editor DOM focus (the
    /// omnibox-pastes / contenteditable-beeps bug — the prior "notch blurs it"
    /// theory was wrong; the notch is non-activating and never blurred it).
    /// Retained behind `allow(dead_code)` in case the diagnosis flips and a
    /// targeted re-activation is needed.
    #[allow(dead_code)]
    fn activate_pid(pid: i32, mtm: MainThreadMarker) {
        let _ = mtm;
        unsafe {
            let cls = objc2::class!(NSRunningApplication);
            let app: *mut objc2::runtime::AnyObject =
                objc2::msg_send![cls, runningApplicationWithProcessIdentifier: pid];
            if !app.is_null() {
                let _: bool = objc2::msg_send![app, activateWithOptions: 1usize << 1];
            }
        }
    }

    // ---------------------------------------------------------------------
    // Target-app capture
    // ---------------------------------------------------------------------

    /// The app the user was dictating into, captured at Fn-down (frontmost
    /// at that instant). The icon rides along for the overlay's leading
    /// glyph — FluidVoice's signature "you are dictating into X" cue.
    #[derive(Debug, Clone)]
    pub struct FrontTarget {
        pub pid: i32,
        pub name: String,
        /// `data:image/png;base64,…` of the app icon at 32 pt, or None when
        /// the icon could not be encoded (the overlay falls back to a dot).
        pub icon_data_url: Option<String>,
        /// Bundle identifier (e.g. `com.apple.Terminal`), when the frontmost
        /// app has one — the stable key the anywhere rewrite resolves a
        /// per-app `RewriteContext` from (see `dictation_anywhere`).
        pub bundle_id: Option<String>,
    }

    /// Frontmost application right now. Main thread only (NSWorkspace).
    pub fn capture_front_target(mtm: MainThreadMarker) -> Option<FrontTarget> {
        let _ = mtm; // NSWorkspace is documented main-thread; the marker is the contract.
        let workspace = NSWorkspace::sharedWorkspace();
        let front = workspace.frontmostApplication()?;
        let pid = front.processIdentifier();
        let name = front
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_default();
        let bundle_id = front.bundleIdentifier().map(|b| b.to_string());
        let icon_data_url = icon_png_data_url(&front);
        Some(FrontTarget {
            pid,
            name,
            icon_data_url,
            bundle_id,
        })
    }

    /// A user-facing running application, for the per-app rewrite-context
    /// editor (Settings → Smart Dictation). `bundle_id` is the stable key the
    /// anywhere rewrite resolves a `RewriteContext` from.
    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct AppInfo {
        pub name: String,
        pub bundle_id: String,
        pub icon_data_url: Option<String>,
    }

    /// One running app from the main-thread enumeration: identity plus its
    /// icon as full-size PNG bytes. The per-icon downscale is deferred to
    /// [`finalize_app_infos`] so it can run off the main thread — encoding
    /// every running app's icon inline (a 1024² decode + resample each) would
    /// stall the WKWebview, whose render loop shares the main thread. That
    /// inline encode is what made the "Polish everywhere" toggle jank: turning
    /// it on kicks off the app list.
    pub struct RawAppIcon {
        name: String,
        bundle_id: String,
        icon_png: Option<Vec<u8>>,
    }

    /// Main-thread pass: enumerate dock-visible apps (regular activation
    /// policy — skips agents/daemons) and grab each icon's full-size PNG bytes
    /// via AppKit. No image-crate resampling here — that is the expensive part
    /// and runs in [`finalize_app_infos`] off the main thread.
    pub fn collect_running_apps(mtm: MainThreadMarker) -> Vec<RawAppIcon> {
        let _ = mtm; // NSWorkspace is documented main-thread; the marker is the contract.
        let workspace = NSWorkspace::sharedWorkspace();
        let running = workspace.runningApplications();
        let mut apps: Vec<RawAppIcon> = Vec::new();
        for i in 0..running.count() {
            let app = running.objectAtIndex(i);
            if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
                continue;
            }
            let Some(bundle_id) = app.bundleIdentifier().map(|b| b.to_string()) else {
                continue;
            };
            let name = app
                .localizedName()
                .map(|n| n.to_string())
                .unwrap_or_else(|| bundle_id.clone());
            apps.push(RawAppIcon {
                name,
                bundle_id,
                icon_png: icon_png_bytes(&app),
            });
        }
        apps
    }

    /// Name + bundle id of every dock-visible running app — the icon-free
    /// sibling of [`collect_running_apps`] for callers that only classify
    /// (the favicon Automation-consent probe). Main thread (NSWorkspace).
    pub fn running_app_identities(mtm: MainThreadMarker) -> Vec<(String, String)> {
        let _ = mtm; // NSWorkspace is documented main-thread; the marker is the contract.
        let workspace = NSWorkspace::sharedWorkspace();
        let running = workspace.runningApplications();
        let mut apps: Vec<(String, String)> = Vec::new();
        for i in 0..running.count() {
            let app = running.objectAtIndex(i);
            if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
                continue;
            }
            let Some(bundle_id) = app.bundleIdentifier().map(|b| b.to_string()) else {
                continue;
            };
            let name = app
                .localizedName()
                .map(|n| n.to_string())
                .unwrap_or_else(|| bundle_id.clone());
            apps.push((name, bundle_id));
        }
        apps
    }

    /// Off-main pass: dedup by bundle id (multiple instances), downscale each
    /// icon to a data URL, then sort alphabetically. Pure Rust (image crate +
    /// base64) — run it on a blocking worker, never the main thread. The
    /// picker the per-app context map is built from.
    pub fn finalize_app_infos(mut raw: Vec<RawAppIcon>) -> Vec<AppInfo> {
        raw.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
        raw.dedup_by(|a, b| a.bundle_id == b.bundle_id);
        let mut apps: Vec<AppInfo> = raw
            .into_iter()
            .map(|r| AppInfo {
                name: r.name,
                bundle_id: r.bundle_id,
                icon_data_url: r.icon_png.as_deref().and_then(png_bytes_to_data_url),
            })
            .collect();
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps
    }

    /// App icon → full-size PNG bytes (native AppKit encode; no resample).
    /// Main-thread (NSImage), but cheap relative to the image-crate downscale,
    /// which callers run separately — off the main thread for the app list.
    /// Best-effort; None on any hiccup.
    fn icon_png_bytes(app: &NSRunningApplication) -> Option<Vec<u8>> {
        let icon = app.icon()?;
        let tiff = icon.TIFFRepresentation()?;
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
        let props = NSDictionary::new();
        let png =
            unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
        let bytes = png.to_vec();
        (!bytes.is_empty()).then_some(bytes)
    }

    /// Downscale full-size icon PNG bytes to a small data URL. Pure image-crate
    /// work (decode + resample + re-encode + base64), no AppKit — so it is safe
    /// (and intended) to run off the main thread. A source icon can carry a
    /// 1024² rep, so the raw PNG runs up to ~512 KB; the overlay/picker render
    /// it at ≤16 pt, so 64 px keeps the data URL a few KB. None on any hiccup.
    pub fn png_bytes_to_data_url(bytes: &[u8]) -> Option<String> {
        use base64::Engine;
        let small = downscale_png(bytes, ICON_PX)?;
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(small)
        ))
    }

    /// App icon → small PNG data URL (full pipeline, single icon). Used where a
    /// lone icon is wanted on the main thread (the Fn-down target capture); the
    /// bulk app-list path splits the two stages across threads instead — see
    /// [`collect_running_apps`] / [`finalize_app_infos`].
    fn icon_png_data_url(app: &NSRunningApplication) -> Option<String> {
        png_bytes_to_data_url(&icon_png_bytes(app)?)
    }

    /// Arbitrary encoded image bytes → PNG bytes via AppKit's NSBitmapImageRep
    /// (ImageIO underneath). This is how favicon bytes get decoded: favicons
    /// usually arrive as .ico, which the png-only image-crate build cannot
    /// read, while ImageIO handles ICO/PNG/JPEG natively — the same decode
    /// the app icon rides through `icon_png_bytes`. Pure decode work with no
    /// window-server tie, so it is safe off the main thread (the favicon
    /// fetch task calls it). Best-effort; None on undecodable bytes.
    pub fn decode_image_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
        let data = NSData::with_bytes(bytes);
        let rep = NSBitmapImageRep::imageRepWithData(&data)?;
        let props = NSDictionary::new();
        let png =
            unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
        let png = png.to_vec();
        (!png.is_empty()).then_some(png)
    }

    /// Target edge for the overlay's leading app-icon glyph.
    const ICON_PX: u32 = 64;

    /// Decode a PNG and re-encode it scaled to fit within `max_px` on the long
    /// edge (aspect preserved — app icons are square). Returns None if the bytes
    /// don't decode or the re-encode fails.
    fn downscale_png(bytes: &[u8], max_px: u32) -> Option<Vec<u8>> {
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
        // Triangle (bilinear) over Lanczos3: at a 64 px target the quality
        // difference is imperceptible, and it's markedly cheaper per icon —
        // this runs once per running app when the picker loads.
        let resized = img.resize(max_px, max_px, image::imageops::FilterType::Triangle);
        let mut out = std::io::Cursor::new(Vec::new());
        resized.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    }

    // ---------------------------------------------------------------------
    // Pasteboard
    // ---------------------------------------------------------------------

    /// What we need to put the user's clipboard back: the plain-string
    /// contents (the overwhelmingly common case) and the change count our
    /// own write produced.
    struct PasteboardHold {
        previous: Option<String>,
        our_change: isize,
    }

    /// Snapshot the pasteboard, then write `text` (+ the transient-type
    /// marker so clipboard managers skip it). Main thread.
    fn write_transcript(text: &str, mtm: MainThreadMarker) -> PasteboardHold {
        let _ = mtm;
        let pb = NSPasteboard::generalPasteboard();
        let previous = unsafe { pb.stringForType(NSPasteboardTypeString) }.map(|s| s.to_string());
        let transient = NSString::from_str("org.nspasteboard.TransientType");
        unsafe {
            pb.clearContents();
            // Declare the transient marker FIRST so managers polling on the
            // change tick see it alongside the string.
            pb.setString_forType(&NSString::from_str(""), &transient);
            pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString);
        }
        let our_change = pb.changeCount();
        PasteboardHold {
            previous,
            our_change,
        }
    }

    /// Restore the snapshot — unless someone else has written to the
    /// pasteboard since our paste (then their copy wins, obviously).
    fn restore(hold: &PasteboardHold, mtm: MainThreadMarker) {
        let _ = mtm;
        let pb = NSPasteboard::generalPasteboard();
        if pb.changeCount() != hold.our_change {
            return;
        }
        unsafe {
            pb.clearContents();
            if let Some(prev) = &hold.previous {
                pb.setString_forType(&NSString::from_str(prev), NSPasteboardTypeString);
            }
        }
    }

    // ---------------------------------------------------------------------
    // The insertion pipeline
    // ---------------------------------------------------------------------

    /// Deliver `text` into the app `target_pid`. Primary path mirrors
    /// FluidVoice: TYPE the transcript directly into the target process as
    /// Unicode text (`CGEventKeyboardSetUnicodeString` → `CGEventPostToPid`).
    /// That is text *entry*, not a paste *command*, so it lands in apps that
    /// silently ignore a synthetic ⌘V (Word's own text engine, Chromium web
    /// fields) — the omnibox-works / everywhere-else-fails bug. The clipboard
    /// ⌘V path is kept only as a fallback (and for very long transcripts).
    ///
    /// Async because typing chunks are spaced by short `tokio::sleep`s.
    pub async fn insert_text(
        app: &tauri::AppHandle,
        text: String,
        target_pid: i32,
    ) -> Result<(), String> {
        if !ax_trusted() {
            return Err("accessibility permission not granted".into());
        }
        if text.is_empty() {
            return Ok(());
        }

        // We inject into `target_pid` directly; confirm it's still frontmost
        // (the user didn't switch apps mid-dictation) so the words can't land
        // in the wrong window. A mismatch bails to the caller's rescue.
        let front = on_main(app, front_pid).await?;
        if front != Some(target_pid) {
            tracing::warn!(
                target_pid,
                front = front.unwrap_or(-1),
                "dictation: focus moved before insert — rescuing to clipboard"
            );
            return Err("focus moved before insert".into());
        }

        // SECURITY: never inject into a password surface. Two independent
        // checks — the focused element's secure AX subrole, and the global
        // Secure-Event-Input flag every password dialog raises (which AX
        // often can't read into). The caller's rescue leaves the words on
        // the clipboard, so the user still decides what happens to them.
        let secure = on_main(app, |mtm| {
            let _ = mtm;
            unsafe { IsSecureEventInputEnabled() || ax_focused_is_secure_field() }
        })
        .await?;
        if secure {
            tracing::warn!(
                target_pid,
                "dictation: secure input field focused — transcript withheld"
            );
            return Err("secure input active — transcript not typed".into());
        }

        let chars = text.chars().count();
        tracing::info!(
            target_pid,
            chars,
            "dictation: inserting transcript via unicode typing (postToPid)"
        );

        // Let the Fn-up keystroke clear the event stream before we inject.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;

        // Primary: type the transcript straight into the target process.
        match type_unicode_to_pid(target_pid, &text).await {
            Ok(()) => {
                tracing::info!(
                    target_pid,
                    "dictation: transcript typed (unicode/postToPid)"
                );
                Ok(())
            }
            Err(detail) => {
                // Only event-creation failure reaches here (posting itself has
                // no signal). Try the Accessibility direct-insert, then the
                // clipboard ⌘V — FluidVoice's fallback order.
                tracing::warn!(
                    target_pid,
                    detail = %detail,
                    "dictation: unicode typing failed — trying accessibility insert"
                );
                if insert_via_accessibility(app, &text).await {
                    tracing::info!(
                        target_pid,
                        "dictation: transcript inserted via accessibility"
                    );
                    return Ok(());
                }
                tracing::warn!(
                    target_pid,
                    "dictation: accessibility insert failed — falling back to clipboard ⌘V"
                );
                insert_via_clipboard_paste(app, text, target_pid).await
            }
        }
    }

    /// Guarded in-place replace for the paste-first dictation flow. After the
    /// raw transcript is typed into the target, the polished version (when it
    /// differs) swaps in here — but ONLY when the text immediately before the
    /// caret is still exactly `old` (what we just inserted) AND focus hasn't
    /// left the target window. That guard makes it impossible to delete
    /// anything the user typed: if they kept typing, moved the caret, selected
    /// text, or the field isn't AX-readable (many web editors), this returns
    /// false and the raw words simply stay. It NEVER blind-replaces a field.
    pub async fn replace_recent_insertion(
        app: &tauri::AppHandle,
        old: &str,
        new: &str,
        target_pid: i32,
    ) -> bool {
        if !ax_trusted() || old.is_empty() {
            return false;
        }
        // Focus must still be the window we pasted into — otherwise the caret
        // we're about to read belongs to someone else.
        match on_main(app, front_pid).await {
            Ok(Some(pid)) if pid == target_pid => {}
            _ => return false,
        }
        let old = old.to_string();
        let new = new.to_string();
        on_main(app, move |mtm| {
            let _ = mtm;
            unsafe { ax_replace_recent_insertion(&old, &new) }
        })
        .await
        .unwrap_or(false)
    }

    /// Keyed fallback for the paste-first polish swap, for editors whose AX
    /// value can be read-verified but not written (Chromium contenteditables
    /// — ChatGPT, Gmail — typically expose AXSelectedText/range while
    /// rejecting `AXValue` writes), where [`replace_recent_insertion`]
    /// necessarily returned false. Strategy: select back over exactly what we
    /// typed with synthetic ⇧← (HID tap — the level web renderers honor, per
    /// the ⌘V history), VERIFY the selection reads back as exactly `old` via
    /// AXSelectedText, then paste `new` over it with the proven clipboard-⌘V
    /// machinery. Every uncertainty bails without modifying anything:
    ///   * focus left the target, caret not readable/collapsed, span too long
    ///     → return false before any key is posted;
    ///   * the selected text isn't exactly `old` (user typed, caret moved,
    ///     the editor normalized whitespace, AXSelectedText unsupported) →
    ///     collapse the selection back to its end (one →) and return false —
    ///     the caret is back where it was and the raw words stay;
    ///   * focus leaves the target mid-burst (re-probed at every batch
    ///     boundary) → abort immediately, posting nothing further — not even
    ///     the collapse →, which would land in the newly focused app.
    ///
    /// This is deliberately the LAST resort in the chain and the most
    /// fragile path in the module — flag any in-app anomaly against it.
    pub async fn replace_recent_insertion_via_keys(
        app: &tauri::AppHandle,
        old: &str,
        new: &str,
        target_pid: i32,
    ) -> bool {
        if !ax_trusted() || old.is_empty() {
            return false;
        }
        let Some(count) = keyed_guard::span_chars(old) else {
            return false;
        };
        match on_main(app, front_pid).await {
            Ok(Some(pid)) if pid == target_pid => {}
            _ => return false,
        }
        // A live selection would be destroyed by our select-back, and an
        // unreadable range means we can't know — both bail before any event.
        match on_main(app, |mtm| {
            let _ = mtm;
            unsafe { ax_focused_caret_collapsed() }
        })
        .await
        {
            Ok(Some(true)) => {}
            _ => return false,
        }

        // Select back over the span we typed. Same source posture as the
        // paste path: combined session state + held-key suppression, so a
        // still-held Fn can't contaminate the synthetic ⇧←.
        let source = EventSource(unsafe { CGEventSourceCreate(EVENT_SOURCE_COMBINED) });
        if !source.0.is_null() {
            unsafe {
                CGEventSourceSetLocalEventsFilterDuringSuppressionState(
                    source.0,
                    SUPPRESS_FILTER,
                    SUPPRESS_STATE,
                );
            }
        }
        let mut posted = 0usize;
        for i in 0..count {
            if post_key(source.0, KEY_LEFT, true, MASK_SHIFT, false).is_err()
                || post_key(source.0, KEY_LEFT, false, MASK_SHIFT, false).is_err()
            {
                break;
            }
            posted += 1;
            // Small breathers keep the burst from coalescing in the target's
            // queue; batches stay well under perceptible jank. Each boundary
            // also re-probes focus: the ⇧← go to the HID tap (frontmost app,
            // not a pid), so the moment focus leaves the target the rest of
            // the burst would select text in whatever window is now in
            // front. Abort hard — and post no collapse → either, for the
            // same reason. (The final partial batch has no boundary probe;
            // the verify + focus-guarded collapse below cover that window.)
            if keyed_guard::batch_boundary(i) {
                tokio::time::sleep(std::time::Duration::from_millis(8)).await;
                let front = on_main(app, front_pid).await.ok().flatten();
                if !keyed_guard::focus_still_target(front, target_pid) {
                    tracing::warn!(
                        target_pid,
                        posted,
                        "dictation: keyed replace aborted mid-burst (focus left target)"
                    );
                    return false;
                }
            }
        }
        // Let the editor settle before reading the selection back.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;

        let selected = on_main(app, |mtm| {
            let _ = mtm;
            unsafe { ax_focused_selected_text() }
        })
        .await
        .ok()
        .flatten();
        let verified = keyed_guard::selection_verified(posted, count, selected.as_deref(), old);

        if !verified {
            // Collapse to the selection's end — the caret returns to where
            // it was, nothing was modified, the fast raw words stay. But a
            // verify failure is also exactly how a focus switch presents
            // (ax_focused_selected_text reads the *globally* focused
            // element), and the → posts to the HID tap — so only collapse
            // when the target is still frontmost; otherwise the keystroke
            // would land in the app the user just switched to.
            let front = on_main(app, front_pid).await.ok().flatten();
            if keyed_guard::focus_still_target(front, target_pid) {
                let _ = post_key(source.0, KEY_RIGHT, true, 0, false);
                let _ = post_key(source.0, KEY_RIGHT, false, 0, false);
                tracing::debug!(
                    target_pid,
                    "dictation: keyed replace bailed (selection didn't verify) — raw stays"
                );
            } else {
                tracing::warn!(
                    target_pid,
                    "dictation: keyed replace bailed and focus left target — collapse skipped"
                );
            }
            return false;
        }
        drop(source);

        // The selection IS our raw span — paste the polished text over it.
        let ok = insert_via_clipboard_paste(app, new.to_string(), target_pid)
            .await
            .is_ok();
        if ok {
            tracing::info!(
                target_pid,
                "dictation: polished swap via keyed select+paste"
            );
        }
        ok
    }

    /// Type `text` into process `pid` as Unicode characters — FluidVoice's
    /// primary insertion. Each chunk is one keyDown/keyUp whose unicode string
    /// is the chunk's characters, posted directly to the target pid. NULL event
    /// source (the characters are delivered verbatim; modifier state is
    /// irrelevant). Chunked so a long transcript never trips the per-event
    /// unicode-string truncation.
    async fn type_unicode_to_pid(pid: i32, text: &str) -> Result<(), String> {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        for chunk in utf16.chunks(UNICODE_CHUNK) {
            post_unicode_chunk(pid, chunk)?;
            // A small gap keeps the ordered chunks from coalescing/racing in
            // the target's input queue.
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        Ok(())
    }

    /// Post one keyDown/keyUp pair carrying `chunk` as the event's unicode
    /// string, straight to `pid`. Synchronous FFI (no `.await`), so no raw
    /// pointer is held across a suspension point.
    fn post_unicode_chunk(pid: i32, chunk: &[u16]) -> Result<(), String> {
        unsafe {
            let down = CGEventCreateKeyboardEvent(std::ptr::null(), 0, true);
            if down.is_null() {
                return Err("could not create keyboard event".into());
            }
            CGEventKeyboardSetUnicodeString(down, chunk.len() as c_ulong, chunk.as_ptr());
            CGEventPostToPid(pid, down);
            CFRelease(down);

            let up = CGEventCreateKeyboardEvent(std::ptr::null(), 0, false);
            if up.is_null() {
                return Err("could not create keyboard event".into());
            }
            CGEventKeyboardSetUnicodeString(up, chunk.len() as c_ulong, chunk.as_ptr());
            CGEventPostToPid(pid, up);
            CFRelease(up);
        }
        Ok(())
    }

    /// Accessibility fallback (FluidVoice's deepest layer): write `text` into
    /// the system-wide focused element's value via the AX API — a channel
    /// independent of synthesized key events, so it can reach an app that
    /// ignores both typing and ⌘V. Runs on the main thread. Returns true only
    /// when it actually set the value.
    ///
    /// Safety contract: it NEVER blind-replaces a field. It splices `text` into
    /// the element's current value at the selected range (cursor) and requires
    /// BOTH a readable value AND a valid selected range — if either is missing
    /// it returns false (→ clipboard fallback) rather than risk wiping content.
    async fn insert_via_accessibility(app: &tauri::AppHandle, text: &str) -> bool {
        let text = text.to_string();
        on_main(app, move |mtm| {
            let _ = mtm;
            unsafe { ax_insert_at_cursor(&text) }
        })
        .await
        .unwrap_or(false)
    }

    /// The system-wide focused UI element, or null. Caller releases.
    unsafe fn ax_focused_element() -> *const c_void {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return std::ptr::null();
        }
        let focus_attr = NSString::from_str("AXFocusedUIElement");
        let mut focused: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            system,
            &*focus_attr as *const NSString as *const c_void,
            &mut focused,
        );
        CFRelease(system);
        if err != KAX_ERROR_SUCCESS {
            return std::ptr::null();
        }
        focused
    }

    /// Whether the system-wide focused element is a secure (password) text
    /// field — `AXSubrole == AXSecureTextField`. `true` only on a positive
    /// match: an unreadable focus answers `false` so ordinary fields (web
    /// editors often expose no subrole at all) keep working; the global
    /// Secure-Event-Input check covers the password dialogs AX can't read.
    unsafe fn ax_focused_is_secure_field() -> bool {
        let focused = ax_focused_element();
        if focused.is_null() {
            return false;
        }
        let subrole_attr = NSString::from_str("AXSubrole");
        let mut value: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            focused,
            &*subrole_attr as *const NSString as *const c_void,
            &mut value,
        );
        CFRelease(focused);
        if err != KAX_ERROR_SUCCESS || value.is_null() {
            return false;
        }
        let secure = (*(value as *const NSString)).to_string() == "AXSecureTextField";
        CFRelease(value);
        secure
    }

    /// Whether the focused element's AXSelectedTextRange is a collapsed
    /// caret. `Some(false)` = a selection is active; `None` = unreadable
    /// (no focused element, or the editor doesn't expose the range) — the
    /// keyed replace treats both non-`Some(true)` answers as "don't touch".
    unsafe fn ax_focused_caret_collapsed() -> Option<bool> {
        let focused = ax_focused_element();
        if focused.is_null() {
            return None;
        }
        let range_attr = NSString::from_str("AXSelectedTextRange");
        let mut range_ref: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            focused,
            &*range_attr as *const NSString as *const c_void,
            &mut range_ref,
        );
        CFRelease(focused);
        if err != KAX_ERROR_SUCCESS || range_ref.is_null() {
            return None;
        }
        let mut range = CFRange {
            location: 0,
            length: 0,
        };
        let got = AXValueGetValue(
            range_ref,
            KAX_VALUE_TYPE_CFRANGE,
            &mut range as *mut CFRange as *mut c_void,
        );
        CFRelease(range_ref);
        got.then_some(range.length == 0)
    }

    /// The focused element's AXSelectedText, when the editor exposes it —
    /// the keyed replace's read-back verification channel.
    unsafe fn ax_focused_selected_text() -> Option<String> {
        let focused = ax_focused_element();
        if focused.is_null() {
            return None;
        }
        let text_attr = NSString::from_str("AXSelectedText");
        let mut text_ref: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            focused,
            &*text_attr as *const NSString as *const c_void,
            &mut text_ref,
        );
        CFRelease(focused);
        if err != KAX_ERROR_SUCCESS || text_ref.is_null() {
            return None;
        }
        let text: String = (*(text_ref as *const NSString)).to_string();
        CFRelease(text_ref);
        Some(text)
    }

    /// Resolve the focused element and splice `text` at its cursor. Main
    /// thread; all AX refs are released on every path.
    unsafe fn ax_insert_at_cursor(text: &str) -> bool {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return false;
        }
        let focus_attr = NSString::from_str("AXFocusedUIElement");
        let mut focused: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            system,
            &*focus_attr as *const NSString as *const c_void,
            &mut focused,
        );
        CFRelease(system);
        if err != KAX_ERROR_SUCCESS || focused.is_null() {
            return false;
        }
        let ok = ax_splice_at_cursor(focused, text);
        CFRelease(focused);
        ok
    }

    /// Read the element's current value + selected range, splice `text` in at
    /// the range (UTF-16), write it back, and move the caret past the insert.
    unsafe fn ax_splice_at_cursor(element: *const c_void, text: &str) -> bool {
        let value_attr = NSString::from_str("AXValue");
        let range_attr = NSString::from_str("AXSelectedTextRange");

        // Current value (string) — its presence proves an editable text element.
        let mut value_ref: *const c_void = std::ptr::null();
        if AXUIElementCopyAttributeValue(
            element,
            &*value_attr as *const NSString as *const c_void,
            &mut value_ref,
        ) != KAX_ERROR_SUCCESS
            || value_ref.is_null()
        {
            return false;
        }
        let current: String = (*(value_ref as *const NSString)).to_string();
        CFRelease(value_ref);

        // Selected range (the cursor / selection to replace).
        let mut range_ref: *const c_void = std::ptr::null();
        if AXUIElementCopyAttributeValue(
            element,
            &*range_attr as *const NSString as *const c_void,
            &mut range_ref,
        ) != KAX_ERROR_SUCCESS
            || range_ref.is_null()
        {
            return false;
        }
        let mut range = CFRange {
            location: 0,
            length: 0,
        };
        let got = AXValueGetValue(
            range_ref,
            KAX_VALUE_TYPE_CFRANGE,
            &mut range as *mut CFRange as *mut c_void,
        );
        CFRelease(range_ref);
        if !got {
            return false;
        }

        // Splice on UTF-16 (AX ranges are UTF-16 code-unit based).
        let cur: Vec<u16> = current.encode_utf16().collect();
        let ins: Vec<u16> = text.encode_utf16().collect();
        let loc = (range.location.max(0) as usize).min(cur.len());
        let end = (loc + range.length.max(0) as usize).min(cur.len());
        let mut next: Vec<u16> = Vec::with_capacity(cur.len() - (end - loc) + ins.len());
        next.extend_from_slice(&cur[..loc]);
        next.extend_from_slice(&ins);
        next.extend_from_slice(&cur[end..]);
        let new_value = String::from_utf16_lossy(&next);

        let new_ns = NSString::from_str(&new_value);
        if AXUIElementSetAttributeValue(
            element,
            &*value_attr as *const NSString as *const c_void,
            &*new_ns as *const NSString as *const c_void,
        ) != KAX_ERROR_SUCCESS
        {
            return false;
        }

        // Best-effort: drop the caret just after the inserted text.
        let caret = CFRange {
            location: (loc + ins.len()) as isize,
            length: 0,
        };
        let caret_value = AXValueCreate(
            KAX_VALUE_TYPE_CFRANGE,
            &caret as *const CFRange as *const c_void,
        );
        if !caret_value.is_null() {
            let _ = AXUIElementSetAttributeValue(
                element,
                &*range_attr as *const NSString as *const c_void,
                caret_value,
            );
            CFRelease(caret_value);
        }
        true
    }

    /// Resolve the focused element and replace the `old` span ending at the
    /// caret with `new`. Main thread; releases every AX ref on every path.
    unsafe fn ax_replace_recent_insertion(old: &str, new: &str) -> bool {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return false;
        }
        let focus_attr = NSString::from_str("AXFocusedUIElement");
        let mut focused: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            system,
            &*focus_attr as *const NSString as *const c_void,
            &mut focused,
        );
        CFRelease(system);
        if err != KAX_ERROR_SUCCESS || focused.is_null() {
            return false;
        }
        let ok = ax_replace_span_before_caret(focused, old, new);
        CFRelease(focused);
        ok
    }

    /// Read value + caret; if the `old.len()` UTF-16 units immediately before a
    /// COLLAPSED caret equal `old`, replace that span with `new` and re-place
    /// the caret after it. Any mismatch — caret moved, a selection is present,
    /// the span isn't ours, or the value is unreadable — returns false and
    /// touches nothing. This is the guard that makes the paste-first replace
    /// safe in arbitrary apps.
    unsafe fn ax_replace_span_before_caret(element: *const c_void, old: &str, new: &str) -> bool {
        let value_attr = NSString::from_str("AXValue");
        let range_attr = NSString::from_str("AXSelectedTextRange");

        let mut value_ref: *const c_void = std::ptr::null();
        if AXUIElementCopyAttributeValue(
            element,
            &*value_attr as *const NSString as *const c_void,
            &mut value_ref,
        ) != KAX_ERROR_SUCCESS
            || value_ref.is_null()
        {
            return false;
        }
        let current: String = (*(value_ref as *const NSString)).to_string();
        CFRelease(value_ref);

        let mut range_ref: *const c_void = std::ptr::null();
        if AXUIElementCopyAttributeValue(
            element,
            &*range_attr as *const NSString as *const c_void,
            &mut range_ref,
        ) != KAX_ERROR_SUCCESS
            || range_ref.is_null()
        {
            return false;
        }
        let mut range = CFRange {
            location: 0,
            length: 0,
        };
        let got = AXValueGetValue(
            range_ref,
            KAX_VALUE_TYPE_CFRANGE,
            &mut range as *mut CFRange as *mut c_void,
        );
        CFRelease(range_ref);
        // Require a collapsed caret (no active selection) before we touch text.
        if !got || range.length != 0 {
            return false;
        }

        let cur: Vec<u16> = current.encode_utf16().collect();
        let old16: Vec<u16> = old.encode_utf16().collect();
        let caret = (range.location.max(0) as usize).min(cur.len());
        if caret < old16.len() {
            return false;
        }
        let start = caret - old16.len();
        // GUARD: the span ending at the caret must be EXACTLY what we inserted.
        if cur[start..caret] != old16[..] {
            return false;
        }

        let new16: Vec<u16> = new.encode_utf16().collect();
        let mut next: Vec<u16> = Vec::with_capacity(cur.len() - old16.len() + new16.len());
        next.extend_from_slice(&cur[..start]);
        next.extend_from_slice(&new16);
        next.extend_from_slice(&cur[caret..]);
        let new_value = String::from_utf16_lossy(&next);

        let new_ns = NSString::from_str(&new_value);
        if AXUIElementSetAttributeValue(
            element,
            &*value_attr as *const NSString as *const c_void,
            &*new_ns as *const NSString as *const c_void,
        ) != KAX_ERROR_SUCCESS
        {
            return false;
        }

        let new_caret = CFRange {
            location: (start + new16.len()) as isize,
            length: 0,
        };
        let caret_value = AXValueCreate(
            KAX_VALUE_TYPE_CFRANGE,
            &new_caret as *const CFRange as *const c_void,
        );
        if !caret_value.is_null() {
            let _ = AXUIElementSetAttributeValue(
                element,
                &*range_attr as *const NSString as *const c_void,
                caret_value,
            );
            CFRelease(caret_value);
        }
        true
    }

    /// Fallback insertion: clipboard + synthetic ⌘V (the prior primary path).
    /// Snapshots the pasteboard, writes the transcript, posts ⌘V at the HID
    /// tap with held-modifier suppression, then restores the clipboard. Used
    /// only when direct typing couldn't be synthesized.
    ///
    /// Limitation (deliberate): the snapshot/restore covers plain-text
    /// clipboard contents; a copied image/file present at dictation time is
    /// not restored — same trade freeflow ships.
    async fn insert_via_clipboard_paste(
        app: &tauri::AppHandle,
        text: String,
        target_pid: i32,
    ) -> Result<(), String> {
        let hold = on_main(app, move |mtm| write_transcript(&text, mtm)).await?;
        tracing::info!(
            target_pid,
            "dictation: delivering transcript via ⌘V (HID tap, fallback)"
        );

        let source = EventSource(unsafe { CGEventSourceCreate(EVENT_SOURCE_COMBINED) });
        if !source.0.is_null() {
            unsafe {
                CGEventSourceSetLocalEventsFilterDuringSuppressionState(
                    source.0,
                    SUPPRESS_FILTER,
                    SUPPRESS_STATE,
                );
            }
        }
        post_key(source.0, KEY_V, true, MASK_COMMAND, true)?;
        tokio::time::sleep(std::time::Duration::from_millis(12)).await;
        post_key(source.0, KEY_V, false, MASK_COMMAND, true)?;
        drop(source);

        // Let the target consume the paste before restoring the clipboard.
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        on_main(app, move |mtm| restore(&hold, mtm)).await?;
        Ok(())
    }

    /// Put `text` on the clipboard *to stay* — no transient marker, no
    /// restore. The rescue path when the synthetic ⌘V could not deliver a
    /// transcript (and the tray's paste-again fallback): the words survive
    /// on the clipboard for the user's own ⌘V instead of vanishing.
    pub async fn copy_text_persistent(app: &tauri::AppHandle, text: String) -> Result<(), String> {
        on_main(app, move |mtm| {
            let _ = mtm;
            let pb = NSPasteboard::generalPasteboard();
            unsafe {
                pb.clearContents();
                pb.setString_forType(&NSString::from_str(&text), NSPasteboardTypeString);
            }
        })
        .await
    }

    /// Run `f` on the main thread and await its result.
    async fn on_main<T: Send + 'static>(
        app: &tauri::AppHandle,
        f: impl FnOnce(MainThreadMarker) -> T + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let mtm = MainThreadMarker::new().expect("run_on_main_thread is the main thread");
            let _ = tx.send(f(mtm));
        })
        .map_err(|e| format!("main-thread hop failed: {e}"))?;
        rx.await.map_err(|_| "main-thread task dropped".to_string())
    }
}

#[cfg(target_os = "macos")]
pub use macos::{
    ax_prompt, ax_trusted, capture_front_target, collect_running_apps, copy_text_persistent,
    decode_image_to_png, finalize_app_infos, insert_text, png_bytes_to_data_url,
    replace_recent_insertion, replace_recent_insertion_via_keys, running_app_identities, AppInfo,
    FrontTarget,
};

#[cfg(not(target_os = "macos"))]
mod stub {
    /// See the macOS module; on other platforms the feature is absent.
    #[derive(Debug, Clone)]
    pub struct FrontTarget {
        pub pid: i32,
        pub name: String,
        pub icon_data_url: Option<String>,
        pub bundle_id: Option<String>,
    }

    pub fn ax_trusted() -> bool {
        false
    }

    pub async fn insert_text(
        _app: &tauri::AppHandle,
        _text: String,
        _target_pid: i32,
    ) -> Result<(), String> {
        Err("dictate-anywhere is macOS-only".into())
    }

    pub async fn copy_text_persistent(
        _app: &tauri::AppHandle,
        _text: String,
    ) -> Result<(), String> {
        Err("dictate-anywhere is macOS-only".into())
    }

    pub async fn replace_recent_insertion(
        _app: &tauri::AppHandle,
        _old: &str,
        _new: &str,
        _target_pid: i32,
    ) -> bool {
        false
    }

    pub async fn replace_recent_insertion_via_keys(
        _app: &tauri::AppHandle,
        _old: &str,
        _new: &str,
        _target_pid: i32,
    ) -> bool {
        false
    }

    /// See the macOS module; on other platforms there is no app picker.
    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct AppInfo {
        pub name: String,
        pub bundle_id: String,
        pub icon_data_url: Option<String>,
    }
}

#[cfg(not(target_os = "macos"))]
pub use stub::{
    ax_trusted, copy_text_persistent, insert_text, replace_recent_insertion,
    replace_recent_insertion_via_keys, AppInfo, FrontTarget,
};

#[cfg(test)]
mod tests {
    use super::keyed_guard::*;

    // span_chars — the "is any key ever posted" gate.

    #[test]
    fn span_chars_rejects_empty() {
        assert_eq!(span_chars(""), None);
    }

    #[test]
    fn span_chars_counts_single_char() {
        assert_eq!(span_chars("a"), Some(1));
    }

    #[test]
    fn span_chars_accepts_exactly_max() {
        let s = "x".repeat(MAX_CHARS);
        assert_eq!(span_chars(&s), Some(MAX_CHARS));
    }

    #[test]
    fn span_chars_rejects_over_max() {
        let s = "x".repeat(MAX_CHARS + 1);
        assert_eq!(span_chars(&s), None);
    }

    #[test]
    fn span_chars_counts_chars_not_bytes() {
        // 4 chars, 13 bytes — each costs one ⇧← pair, so the unit must be
        // chars; a byte count would over-select and fail the verify.
        assert_eq!(span_chars("é🦀ü—"), Some(4));
    }

    // batch_boundary — where the burst pauses to re-probe focus.

    #[test]
    fn batch_boundary_fires_at_end_of_each_batch() {
        assert!(batch_boundary(BATCH - 1));
        assert!(batch_boundary(2 * BATCH - 1));
    }

    #[test]
    fn batch_boundary_quiet_inside_a_batch() {
        assert!(!batch_boundary(0));
        assert!(!batch_boundary(BATCH - 2));
        assert!(!batch_boundary(BATCH)); // first event of batch 2, not a boundary
    }

    // focus_still_target — gates both burst continuation and the collapse →.

    #[test]
    fn focus_matching_pid_continues() {
        assert!(focus_still_target(Some(42), 42));
    }

    #[test]
    fn focus_other_app_aborts() {
        assert!(!focus_still_target(Some(43), 42));
    }

    #[test]
    fn focus_probe_failure_aborts() {
        // A failed frontmost probe must read as "lost": posting keys blind
        // is never safe.
        assert!(!focus_still_target(None, 42));
    }

    // selection_verified — the "may we paste over it" proof.

    #[test]
    fn verify_requires_full_burst_and_exact_match() {
        assert!(selection_verified(5, 5, Some("hello"), "hello"));
    }

    #[test]
    fn verify_fails_on_short_burst() {
        // post_key failed mid-burst: even an exact read-back can't prove the
        // selection covers the whole span we meant to select.
        assert!(!selection_verified(4, 5, Some("hello"), "hello"));
    }

    #[test]
    fn verify_fails_on_mismatched_selection() {
        // Editor normalized whitespace / user typed — raw words must stay.
        assert!(!selection_verified(5, 5, Some("hell o"), "hello"));
    }

    #[test]
    fn verify_fails_on_unreadable_selection() {
        assert!(!selection_verified(5, 5, None, "hello"));
    }

    #[test]
    fn verify_fails_on_empty_selection_readback() {
        // Collapsed caret reading back "" must not verify against any span.
        assert!(!selection_verified(5, 5, Some(""), "hello"));
    }
}
