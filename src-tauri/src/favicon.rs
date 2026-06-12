//! Active-tab favicon resolution for the dictation notch overlay.
//!
//! When the Fn-down target is a BROWSER, the overlay's leading glyph should
//! show the SITE being dictated into — the ChatGPT favicon, not the Chrome
//! logo. The browser's AppleScript surface gives us the active tab's URL,
//! and the site's own `/favicon.ico` gives us the glyph. Everything here
//! runs OFF the Fn-down hot path (`dictation_anywhere::spawn_favicon_swap`
//! fires it as a fire-and-forget task after the target capture), so the
//! prebuffer/overlay arming never wait on it — and every failure (no
//! AppleScript URL surface, Automation consent denied, no favicon, offline)
//! degrades silently to the browser's app icon, which is already on screen.
//!
//! PRIVACY: the tab URL never leaves the machine except as one direct
//! request to the site's own origin for `/favicon.ico` — no third-party
//! favicon service (Google s2 and friends) ever sees the user's browsing.
//! Resolved icons are cached per host (memory + disk), so a site the user
//! dictates into regularly costs one fetch, ever.
//!
//! CONSENT: reading a browser's tab URL is Apple-Events Automation, which
//! macOS gates behind a per-(app, browser) consent dialog. A cosmetic icon
//! swap must NEVER be what makes that dialog appear mid-dictation — so the
//! script is gated on [`automation_consent`] (a no-prompt TCC probe) and
//! only runs once consent already exists. The dialog itself is fired only
//! from the settings panel's explicit "enable site icons" affordance
//! (`request_automation_consent`), a user-initiated moment.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

/// Which AppleScript dialect a browser speaks for "the active tab's URL".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserFamily {
    /// `URL of front document` — WebKit's scripting model.
    Safari,
    /// `URL of active tab of front window` — the Chromium family all ship
    /// Chrome's scripting dictionary.
    Chromium,
}

/// Browsers whose active-tab URL is scriptable, keyed by bundle id —
/// mirrors `TERMINAL_BUNDLES` in `dictation_anywhere`. Firefox (and the
/// Gecko forks) are deliberately absent: Gecko ships no AppleScript URL
/// surface, so those targets keep the browser's app icon.
const SAFARI_BUNDLES: &[&str] = &[
    "com.apple.Safari",
    "com.apple.SafariTechnologyPreview",
    // Orion advertises Safari-compatible scripting; a miss falls back
    // silently like any other AppleScript failure.
    "com.kagi.kagimacOS",
];
const CHROMIUM_BUNDLES: &[&str] = &[
    "com.google.Chrome",
    "com.google.Chrome.beta",
    "com.google.Chrome.canary",
    "com.microsoft.edgemac",
    "com.brave.Browser",
    "com.brave.Browser.beta",
    "com.vivaldi.Vivaldi",
    "company.thebrowser.Browser", // Arc
    "com.operasoftware.Opera",
    "org.chromium.Chromium",
];

/// Classify a bundle id as a scriptable browser. Case-insensitive
/// (NSWorkspace casing isn't guaranteed stable — same posture as the
/// terminal-bundle match). None = not a browser we can read a tab URL from.
pub fn browser_family(bundle_id: &str) -> Option<BrowserFamily> {
    browser_script_target(bundle_id).map(|(family, _)| family)
}

/// The family plus the CANONICAL bundle id from our own const list — the id
/// that gets interpolated into the AppleScript, so a live bundle string can
/// never inject script syntax (we only ever script ids we wrote ourselves).
fn browser_script_target(bundle_id: &str) -> Option<(BrowserFamily, &'static str)> {
    if let Some(id) = SAFARI_BUNDLES
        .iter()
        .find(|b| b.eq_ignore_ascii_case(bundle_id))
    {
        return Some((BrowserFamily::Safari, id));
    }
    CHROMIUM_BUNDLES
        .iter()
        .find(|b| b.eq_ignore_ascii_case(bundle_id))
        .map(|id| (BrowserFamily::Chromium, *id))
}

/// Every scriptable browser bundle id, for the consent surface's "which of
/// these are running" enumeration.
pub fn scriptable_bundles() -> impl Iterator<Item = &'static str> {
    SAFARI_BUNDLES.iter().chain(CHROMIUM_BUNDLES).copied()
}

/// Apple-Events Automation consent for one (PortBay, browser) pair —
/// `kTCCServiceAppleEvents` is granted per target app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationConsent {
    /// The user already said yes — scripting will not prompt.
    Granted,
    /// macOS has never asked for this pair; scripting WOULD prompt.
    NotDetermined,
    /// Asked and refused (or otherwise blocked) — only System Settings ›
    /// Privacy & Security › Automation can flip it back.
    Denied,
    /// The browser isn't running, so consent can't be determined (the
    /// dialog needs a live target).
    NotRunning,
}

/// `AEDeterminePermissionToAutomateTarget` — the only supported way to ask
/// "may I?" without the side effect of the consent dialog. CoreServices C
/// API; no objc2 binding exists, hence the local FFI.
#[cfg(target_os = "macos")]
mod automation {
    use std::ffi::c_void;

    type OSStatus = i32;

    /// Opaque-enough mirror of the two-word AEDesc struct — created and
    /// disposed strictly by the AE calls below, never inspected.
    #[repr(C)]
    struct AEDesc {
        descriptor_type: u32,
        data_handle: *mut c_void,
    }

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn AECreateDesc(
            type_code: u32,
            data_ptr: *const c_void,
            data_size: isize,
            result: *mut AEDesc,
        ) -> OSStatus;
        fn AEDisposeDesc(desc: *mut AEDesc) -> OSStatus;
        fn AEDeterminePermissionToAutomateTarget(
            target: *const AEDesc,
            event_class: u32,
            event_id: u32,
            ask_user_if_needed: u8,
        ) -> OSStatus;
    }

    const TYPE_APPLICATION_BUNDLE_ID: u32 = u32::from_be_bytes(*b"bund");
    const TYPE_WILDCARD: u32 = u32::from_be_bytes(*b"****");
    /// errAEEventWouldRequireUserConsent — not yet asked.
    const WOULD_REQUIRE_CONSENT: OSStatus = -1744;
    /// procNotFound — the target app isn't running.
    const PROC_NOT_FOUND: OSStatus = -600;

    /// Probe (or, with `ask`, request) consent to automate `bundle_id`.
    /// With `ask: true` this BLOCKS until the user answers the dialog —
    /// callers must be off the main thread.
    pub fn determine(bundle_id: &str, ask: bool) -> super::AutomationConsent {
        let mut target = AEDesc {
            descriptor_type: 0,
            data_handle: std::ptr::null_mut(),
        };
        let status = unsafe {
            let created = AECreateDesc(
                TYPE_APPLICATION_BUNDLE_ID,
                bundle_id.as_ptr() as *const c_void,
                bundle_id.len() as isize,
                &mut target,
            );
            if created != 0 {
                return super::AutomationConsent::Denied;
            }
            let status = AEDeterminePermissionToAutomateTarget(
                &target,
                TYPE_WILDCARD,
                TYPE_WILDCARD,
                u8::from(ask),
            );
            AEDisposeDesc(&mut target);
            status
        };
        match status {
            0 => super::AutomationConsent::Granted,
            WOULD_REQUIRE_CONSENT => super::AutomationConsent::NotDetermined,
            PROC_NOT_FOUND => super::AutomationConsent::NotRunning,
            // errAEEventNotPermitted (-1743) and anything unexpected: treat
            // as denied — never a reason to script anyway.
            _ => super::AutomationConsent::Denied,
        }
    }
}

/// Consent state for automating `bundle_id`, WITHOUT prompting. Fast tccd
/// round trip; safe anywhere.
#[cfg(target_os = "macos")]
pub fn automation_consent(bundle_id: &str) -> AutomationConsent {
    automation::determine(bundle_id, false)
}

/// Fire macOS's own Automation consent dialog for `bundle_id` (no-op past
/// NotDetermined) and report the answer. Blocks until the user decides —
/// call from a blocking worker, never the main thread, and only ever from
/// an explicit user action (the settings "enable" button).
#[cfg(target_os = "macos")]
pub fn request_automation_consent(bundle_id: &str) -> AutomationConsent {
    automation::determine(bundle_id, true)
}

/// The cache/fetch host for a tab URL: http(s) only, lowercased, and
/// strictly `[a-z0-9.-]` so the host doubles as a safe cache filename.
/// Anything else — chrome:// pages, file://, about:blank, bracketed IPv6 —
/// resolves to None and the browser icon stays.
pub fn host_for_favicon(tab_url: &str) -> Option<String> {
    let url = url::Url::parse(tab_url.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // The url crate already lowercases and punycodes registrable domains.
    let host = url.host_str()?.to_ascii_lowercase();
    let safe = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    safe.then_some(host)
}

/// Ceiling on the AppleScript round trip. The task is off the hot path, but
/// an unbounded osascript (browser showing a modal, Automation consent
/// dialog pending) must not pin a zombie child for the whole session.
#[cfg(target_os = "macos")]
const SCRIPT_TIMEOUT: Duration = Duration::from_millis(1500);
/// Network budget for the one favicon request — short by design: the swap is
/// cosmetic, and a slow origin should cost nothing but "the browser icon
/// stayed".
#[cfg(target_os = "macos")]
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);
/// Hard ceiling on a fetched favicon. Real .ico/.png favicons run a few KB;
/// past this it's almost certainly not the glyph we asked for.
#[cfg(target_os = "macos")]
const MAX_ICON_BYTES: u64 = 512 * 1024;
/// How long a FAILED resolution is remembered before the host is retried.
/// Without this an offline stretch (or a site with no favicon.ico) would
/// re-pay the osascript + fetch on every single Fn-down; with a session-long
/// hold a transient failure would stick until restart. Successes are kept
/// for the whole run — the disk cache persists them across runs anyway.
const NEGATIVE_TTL: Duration = Duration::from_secs(10 * 60);

/// Session cache: host → resolution outcome.
enum MemEntry {
    /// Resolved `data:image/png;base64,…` URL, ready for the overlay.
    Icon(String),
    /// Resolution failed at this instant — don't retry until [`NEGATIVE_TTL`]
    /// has passed.
    FailedAt(Instant),
}

static MEM: Lazy<Mutex<HashMap<String, MemEntry>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// `~/Library/Application Support/PortBay/favicons` — mirrors the avatar
/// cache's `data_dir()/PortBay` layout. One small PNG per host.
fn cache_file(host: &str) -> Option<std::path::PathBuf> {
    Some(
        dirs::data_dir()?
            .join("PortBay")
            .join("favicons")
            .join(format!("{host}.png")),
    )
}

/// Resolve the favicon for `bundle_id`'s active tab as a small PNG data URL.
/// The whole chain is best-effort: None at any step (not a scriptable
/// browser, AppleScript failed/denied, non-web URL, no fetchable favicon)
/// means "keep the browser icon". macOS only — osascript + AppKit decode.
#[cfg(target_os = "macos")]
pub async fn active_tab_favicon(bundle_id: &str) -> Option<String> {
    let (family, canonical) = browser_script_target(bundle_id)?;
    // HARD GATE: only script a browser the user has ALREADY consented to.
    // Anything else — never asked, denied, probe hiccup — keeps the browser
    // icon silently. The dictation path must never be what pops the
    // Automation dialog (that moment belongs to the settings opt-in).
    if automation_consent(canonical) != AutomationConsent::Granted {
        tracing::debug!(
            bundle_id,
            "favicon: no Automation consent; keeping the app icon"
        );
        return None;
    }
    let tab_url = active_tab_url(family, canonical).await?;
    let host = host_for_favicon(&tab_url)?;

    // Memory first: a hit (positive or fresh-negative) costs one lock.
    {
        let mem = MEM.lock().unwrap_or_else(|e| e.into_inner());
        match mem.get(&host) {
            Some(MemEntry::Icon(url)) => return Some(url.clone()),
            Some(MemEntry::FailedAt(at)) if at.elapsed() < NEGATIVE_TTL => return None,
            _ => {}
        }
    }

    let resolved = resolve_host_icon(&host).await;
    let entry = match &resolved {
        Some(url) => MemEntry::Icon(url.clone()),
        None => MemEntry::FailedAt(Instant::now()),
    };
    MEM.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(host, entry);
    resolved
}

/// Ask the browser for its active tab's URL over AppleScript. A denial of
/// the per-target Automation consent (kTCCServiceAppleEvents is granted per
/// browser) surfaces as a non-zero osascript exit — silently None, like
/// every other failure here.
#[cfg(target_os = "macos")]
async fn active_tab_url(family: BrowserFamily, bundle_id: &'static str) -> Option<String> {
    let script = match family {
        BrowserFamily::Safari => {
            format!(r#"tell application id "{bundle_id}" to get URL of front document"#)
        }
        BrowserFamily::Chromium => {
            format!(r#"tell application id "{bundle_id}" to get URL of active tab of front window"#)
        }
    };
    let run = tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(SCRIPT_TIMEOUT, run).await {
        Ok(Ok(output)) => output,
        Ok(Err(detail)) => {
            tracing::debug!(bundle_id, %detail, "favicon: osascript failed to launch");
            return None;
        }
        Err(_) => {
            tracing::debug!(bundle_id, "favicon: osascript timed out");
            return None;
        }
    };
    if !output.status.success() {
        // Automation denied, no window, browser busy — all the same answer.
        tracing::debug!(bundle_id, "favicon: active-tab script returned an error");
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!url.is_empty()).then_some(url)
}

/// Disk cache → network, in that order. The disk copy is the ImageIO-decoded
/// PNG (not the raw .ico), so a hit is a tiny read + the standard downscale.
/// On a network success the PNG is persisted best-effort for next run.
#[cfg(target_os = "macos")]
async fn resolve_host_icon(host: &str) -> Option<String> {
    let path = cache_file(host);
    if let Some(path) = &path {
        if let Ok(bytes) = tokio::fs::read(path).await {
            if let Some(url) = crate::typing::png_bytes_to_data_url(&bytes) {
                return Some(url);
            }
            // Corrupt cache entry: fall through and re-fetch over it.
        }
    }

    let raw = fetch_favicon_ico(host).await?;
    // ImageIO decode (handles .ico natively) → PNG → the shared 64 px
    // data-url pipeline the app icon uses.
    let png = crate::typing::decode_image_to_png(&raw)?;
    let url = crate::typing::png_bytes_to_data_url(&png)?;
    if let Some(path) = &path {
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        // Best-effort persist — a write failure just means a re-fetch next
        // run, not a missing icon now.
        let _ = tokio::fs::write(path, &png).await;
    }
    Some(url)
}

/// Fetch `https://<host>/favicon.ico` directly from the site's own origin.
/// v1 is deliberately favicon.ico-only — parsing the page for
/// `<link rel="icon">` would mean downloading the page itself, which is
/// neither cheap nor necessary for the sites dictation lands in. Bounded by
/// [`FETCH_TIMEOUT`] / [`MAX_ICON_BYTES`]; None on any failure.
#[cfg(target_os = "macos")]
async fn fetch_favicon_ico(host: &str) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .ok()?;
    let resp = client
        .get(format!("https://{host}/favicon.ico"))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    // A 200 serving the SPA shell instead of an icon is common — reject
    // text up front rather than letting the image decode chew on HTML.
    if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
        if ct.to_str().map(|v| v.starts_with("text/")).unwrap_or(false) {
            return None;
        }
    }
    if resp
        .content_length()
        .map(|n| n > MAX_ICON_BYTES)
        .unwrap_or(false)
    {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    (!bytes.is_empty() && bytes.len() as u64 <= MAX_ICON_BYTES).then(|| bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Browser bundle-id classification ------------------------------

    #[test]
    fn classifies_safari_family() {
        assert_eq!(
            browser_family("com.apple.Safari"),
            Some(BrowserFamily::Safari)
        );
        assert_eq!(
            browser_family("com.kagi.kagimacOS"),
            Some(BrowserFamily::Safari)
        );
    }

    #[test]
    fn classifies_chromium_family() {
        for id in [
            "com.google.Chrome",
            "com.microsoft.edgemac",
            "com.brave.Browser",
            "company.thebrowser.Browser",
            "com.vivaldi.Vivaldi",
        ] {
            assert_eq!(
                browser_family(id),
                Some(BrowserFamily::Chromium),
                "{id} should classify as Chromium"
            );
        }
    }

    #[test]
    fn bundle_match_is_case_insensitive() {
        // NSWorkspace casing isn't guaranteed stable — same posture as the
        // terminal-bundle match in dictation_anywhere.
        assert_eq!(
            browser_family("COM.GOOGLE.CHROME"),
            Some(BrowserFamily::Chromium)
        );
    }

    #[test]
    fn firefox_and_non_browsers_are_none() {
        // Gecko has no AppleScript URL surface — must stay on the app icon.
        assert_eq!(browser_family("org.mozilla.firefox"), None);
        assert_eq!(browser_family("com.apple.Terminal"), None);
        assert_eq!(browser_family(""), None);
    }

    #[test]
    fn script_target_returns_the_canonical_id() {
        // The script interpolates OUR const, never the live bundle string —
        // a differently-cased live id must resolve to the canonical spelling.
        let (_, canonical) = browser_script_target("COM.GOOGLE.CHROME").unwrap();
        assert_eq!(canonical, "com.google.Chrome");
    }

    // --- Tab URL → favicon host ----------------------------------------

    #[test]
    fn extracts_host_from_web_urls() {
        assert_eq!(
            host_for_favicon("https://chatgpt.com/c/abc123"),
            Some("chatgpt.com".into())
        );
        assert_eq!(
            host_for_favicon("http://docs.example.co.uk:8080/page?q=1"),
            Some("docs.example.co.uk".into())
        );
        // Surrounding whitespace from the osascript stdout trim is tolerated.
        assert_eq!(
            host_for_favicon("  https://github.com/  "),
            Some("github.com".into())
        );
    }

    #[test]
    fn host_is_lowercased() {
        assert_eq!(
            host_for_favicon("https://ChatGPT.com/"),
            Some("chatgpt.com".into())
        );
    }

    #[test]
    fn rejects_non_web_schemes() {
        // Browser-internal and local pages have no fetchable favicon origin.
        assert_eq!(host_for_favicon("chrome://settings"), None);
        assert_eq!(host_for_favicon("about:blank"), None);
        assert_eq!(host_for_favicon("file:///Users/me/page.html"), None);
        assert_eq!(host_for_favicon("favorites://"), None);
    }

    #[test]
    fn rejects_unparseable_and_unsafe_hosts() {
        assert_eq!(host_for_favicon(""), None);
        assert_eq!(host_for_favicon("not a url"), None);
        // Bracketed IPv6 would not be a safe cache filename — skipped.
        assert_eq!(host_for_favicon("https://[::1]/page"), None);
    }

    #[test]
    fn plain_ip_hosts_pass() {
        // A LAN dashboard is a legitimate dictation target; dots and digits
        // are filename-safe.
        assert_eq!(
            host_for_favicon("http://192.168.1.10/admin"),
            Some("192.168.1.10".into())
        );
    }
}
