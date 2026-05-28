//! Best-effort project icon detection.
//!
//! Given a registered project, find the favicon / web-clip / app icon that
//! already lives in the project's source tree and return it as raw bytes plus
//! a mime type. The UI shows this in the project's avatar so a project reads
//! by its own identity rather than a generated initial. Returns `None` when
//! nothing suitable is found — the frontend then falls back to the project's
//! stack glyph.
//!
//! Everything here is read-only and bounded: a fixed, ordered list of
//! candidate relative paths under the project root, a single capped pass over
//! `index.html`, a small web-manifest parse, and — only for native app kinds
//! — a shallow directory walk that skips heavy build/vendor dirs. Hrefs that
//! point outside the project tree (remote URLs, `../` escapes) are rejected.

use std::path::{Path, PathBuf};

use base64::Engine;

use crate::registry::{Project, ProjectType};

/// A detected icon: raw file bytes plus the mime to advertise in the data URL.
pub struct IconData {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

impl IconData {
    /// `data:{mime};base64,{…}` — directly usable as an `<img src>`.
    pub fn to_data_url(&self) -> String {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&self.bytes);
        format!("data:{};base64,{b64}", self.mime)
    }
}

/// Files larger than this are skipped. A project avatar never needs a 1 MB
/// asset, and the bytes are embedded as base64 in a data URL held in memory.
const MAX_ICON_BYTES: u64 = 512 * 1024;

/// Directories never worth walking when locating a native app-icon set.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "Pods",
    "build",
    "DerivedData",
    ".next",
    "dist",
    "out",
    "vendor",
    "target",
    ".svelte-kit",
];

/// Detect the best on-disk icon for a project, if any.
pub fn detect_icon(project: &Project) -> Option<IconData> {
    // Monorepo / workspace projects keep `path` at the repo root, but the
    // actual app — and its favicon — lives in the workspace subdirectory
    // (e.g. `apps/web`). Search that first, then fall back to the repo root
    // for a shared, repo-level icon.
    let app_dir = match project.workspace.as_ref() {
        Some(ws) => project.path.join(&ws.rel_dir),
        None => project.path.clone(),
    };
    detect_in_dirs(
        &app_dir,
        &project.path,
        project.kind,
        project.document_root.as_deref(),
    )
}

/// Search `app_dir` for an icon, falling back to `repo_root` when they differ
/// (the monorepo case). `document_root` is relative to the repo root, so it's
/// only meaningful for the root search.
fn detect_in_dirs(
    app_dir: &Path,
    repo_root: &Path,
    kind: ProjectType,
    document_root: Option<&str>,
) -> Option<IconData> {
    let same = app_dir == repo_root;
    match kind {
        ProjectType::Xcode => detect_apple_icon(app_dir).or_else(|| {
            if same {
                None
            } else {
                detect_apple_icon(repo_root)
            }
        }),
        ProjectType::Android => detect_android_icon(app_dir).or_else(|| {
            if same {
                None
            } else {
                detect_android_icon(repo_root)
            }
        }),
        // Flutter carries both an iOS asset catalogue and an Android res tree.
        ProjectType::Flutter => detect_apple_icon(app_dir).or_else(|| detect_android_icon(app_dir)),
        // Everything web-ish shares the same favicon search.
        _ => detect_web_icon(app_dir, if same { document_root } else { None }).or_else(|| {
            if same {
                None
            } else {
                detect_web_icon(repo_root, document_root)
            }
        }),
    }
}

/// Test shim: search a single directory (no separate workspace root).
#[cfg(test)]
fn detect_in(root: &Path, kind: ProjectType, document_root: Option<&str>) -> Option<IconData> {
    detect_in_dirs(root, root, kind, document_root)
}

// ---------------------------------------------------------------------------
// Web favicon / logo detection
// ---------------------------------------------------------------------------

/// Explicit icon files, in priority order. SVG first (crisp at any size),
/// then the conventional public/static favicon names, then project root.
const WEB_CANDIDATES: &[&str] = &[
    // Next.js app-router conventions — these are the project's real favicon.
    "app/icon.svg",
    "app/icon.png",
    "app/apple-icon.png",
    "app/favicon.ico",
    "src/app/icon.svg",
    "src/app/icon.png",
    "src/app/apple-icon.png",
    "src/app/favicon.ico",
    // public/ (CRA, Vite, Next pages-router, most bundlers).
    "public/favicon.svg",
    "public/icon.svg",
    "public/logo.svg",
    "public/apple-touch-icon.png",
    "public/icon.png",
    "public/logo.png",
    "public/favicon.png",
    "public/favicon.ico",
    // static/ (SvelteKit, Astro).
    "static/favicon.svg",
    "static/icon.svg",
    "static/apple-touch-icon.png",
    "static/favicon.png",
    "static/favicon.ico",
    // Project root (plain static sites).
    "favicon.svg",
    "logo.svg",
    "apple-touch-icon.png",
    "favicon.png",
    "logo.png",
    "favicon.ico",
];

/// Favicon names searched inside a PHP project's document root.
const DOCROOT_FAVICONS: &[&str] = &[
    "favicon.svg",
    "apple-touch-icon.png",
    "favicon.png",
    "favicon.ico",
    "logo.svg",
    "logo.png",
];

fn detect_web_icon(root: &Path, document_root: Option<&str>) -> Option<IconData> {
    // 1. Explicit, conventional candidate files.
    for rel in WEB_CANDIDATES {
        if let Some(found) = load_icon_file(&root.join(rel)) {
            return Some(found);
        }
    }
    // PHP document root (e.g. `public`, `web`).
    if let Some(dr) = document_root.filter(|s| !s.trim().is_empty()) {
        let dr_path = root.join(dr);
        for name in DOCROOT_FAVICONS {
            if let Some(found) = load_icon_file(&dr_path.join(name)) {
                return Some(found);
            }
        }
    }
    // 2. `<link rel="icon">` declared in an index.html.
    let mut html_dirs: Vec<PathBuf> = vec![
        root.join("index.html"),
        root.join("public/index.html"),
        root.join("src/index.html"),
    ];
    if let Some(dr) = document_root.filter(|s| !s.trim().is_empty()) {
        html_dirs.push(root.join(dr).join("index.html"));
    }
    for html in &html_dirs {
        if let Some(found) = icon_from_html(html) {
            return Some(found);
        }
    }
    // 3. Web manifest `icons[]`.
    for man in &[
        "public/site.webmanifest",
        "public/manifest.json",
        "public/manifest.webmanifest",
        "site.webmanifest",
        "manifest.json",
    ] {
        if let Some(found) = icon_from_manifest(&root.join(man)) {
            return Some(found);
        }
    }
    None
}

/// Read a file as an icon if it exists, is a non-empty image under the size
/// cap, and has a recognised image extension.
fn load_icon_file(path: &Path) -> Option<IconData> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_ICON_BYTES {
        return None;
    }
    let mime = mime_for(path)?;
    let bytes = std::fs::read(path).ok()?;
    Some(IconData { mime, bytes })
}

fn mime_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "ico" => Some("image/x-icon"),
        "webp" => Some("image/webp"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn icon_from_html(html_path: &Path) -> Option<IconData> {
    let raw = std::fs::read_to_string(html_path).ok()?;
    let html = truncate_at_boundary(&raw, 64 * 1024);
    let href = extract_icon_href(html)?;
    let base = html_path.parent()?;
    resolve_local_href(base, &href).and_then(|p| load_icon_file(&p))
}

/// Find the href of the best `<link rel="…icon…">` in an HTML document.
/// `apple-touch-icon` wins when present (it's the highest-quality mark);
/// otherwise the first icon link with a usable href.
fn extract_icon_href(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut best: Option<String> = None;
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find("<link") {
        let start = cursor + rel;
        let end = lower[start..]
            .find('>')
            .map(|e| start + e)
            .unwrap_or(lower.len());
        let tag_lower = &lower[start..end];
        let tag_orig = &html[start..end];
        cursor = end + 1;

        if !rel_is_icon(tag_lower) {
            continue;
        }
        if let Some(href) = attr_value(tag_orig, "href") {
            if tag_lower.contains("apple-touch-icon") {
                return Some(href);
            }
            best.get_or_insert(href);
        }
    }
    best
}

fn rel_is_icon(tag_lower: &str) -> bool {
    attr_value(tag_lower, "rel")
        .map(|v| v.contains("icon"))
        .unwrap_or(false)
}

/// Extract an HTML attribute's value (quoted or unquoted) from a single tag.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let idx = lower.find(&key)?;
    let after = tag[idx + key.len()..].trim_start();
    let bytes = after.as_bytes();
    match bytes.first()? {
        b'"' | b'\'' => {
            let quote = after.as_bytes()[0] as char;
            let rest = &after[1..];
            let end = rest.find(quote)?;
            Some(rest[..end].to_string())
        }
        _ => {
            let end = after
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .unwrap_or(after.len());
            Some(after[..end].to_string())
        }
    }
}

/// Resolve an HTML/manifest href to a local file inside `base_dir`. Rejects
/// remote (`http(s)://`, `//`) and `data:` hrefs, and anything that escapes
/// the base directory.
fn resolve_local_href(base_dir: &Path, href: &str) -> Option<PathBuf> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("//")
        || lower.starts_with("data:")
    {
        return None;
    }
    // Drop query / fragment; treat a leading slash as web-root-relative
    // (best effort: the directory holding index.html).
    let clean = href.split(['?', '#']).next().unwrap_or(href);
    let rel = clean.trim_start_matches('/');
    let candidate = base_dir.join(rel);

    let canon = candidate.canonicalize().ok()?;
    let base_canon = base_dir.canonicalize().ok()?;
    if !canon.starts_with(&base_canon) {
        return None;
    }
    Some(canon)
}

fn icon_from_manifest(man_path: &Path) -> Option<IconData> {
    let raw = std::fs::read_to_string(man_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let icons = json.get("icons")?.as_array()?;

    // Pick the largest declared icon, falling back to the first with a src.
    let mut best: Option<(u32, String)> = None;
    for icon in icons {
        let Some(src) = icon.get("src").and_then(|s| s.as_str()) else {
            continue;
        };
        let size = icon
            .get("sizes")
            .and_then(|s| s.as_str())
            .map(largest_size)
            .unwrap_or(0);
        if best.as_ref().map(|(b, _)| size >= *b).unwrap_or(true) {
            best = Some((size, src.to_string()));
        }
    }
    let (_, src) = best?;
    let base = man_path.parent()?;
    resolve_local_href(base, &src).and_then(|p| load_icon_file(&p))
}

/// Largest pixel dimension from a manifest `sizes` string like `"48x48 96x96"`.
fn largest_size(sizes: &str) -> u32 {
    sizes
        .split_whitespace()
        .filter_map(|tok| {
            tok.split(['x', 'X'])
                .next()
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
}

/// Truncate a string to at most `max` bytes, on a char boundary.
fn truncate_at_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---------------------------------------------------------------------------
// Native app icons (Xcode / Android / Flutter)
// ---------------------------------------------------------------------------

/// Locate an `AppIcon.appiconset` and pick a mid-size raster from it.
fn detect_apple_icon(root: &Path) -> Option<IconData> {
    let set = find_dir_named(root, "AppIcon.appiconset", 6)?;
    pick_appiconset_image(&set)
}

/// Choose a ~120px-class PNG from an asset catalogue (avoiding the 1024px
/// marketing icon), driven by `Contents.json` when present, else by file size.
fn pick_appiconset_image(set: &Path) -> Option<IconData> {
    if let Ok(raw) = std::fs::read_to_string(set.join("Contents.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(images) = json.get("images").and_then(|v| v.as_array()) {
                let mut best: Option<(i64, String)> = None; // (distance-to-120, filename)
                for img in images {
                    let Some(file) = img.get("filename").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let dim = img
                        .get("size")
                        .and_then(|v| v.as_str())
                        .map(largest_size)
                        .unwrap_or(0);
                    let scale = img
                        .get("scale")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.trim_end_matches('x').parse::<u32>().ok())
                        .unwrap_or(1);
                    let px = (dim * scale) as i64;
                    let dist = (px - 120).abs();
                    if best.as_ref().map(|(d, _)| dist < *d).unwrap_or(true) {
                        best = Some((dist, file.to_string()));
                    }
                }
                if let Some((_, file)) = best {
                    if let Some(found) = load_icon_file(&set.join(file)) {
                        return Some(found);
                    }
                }
            }
        }
    }
    // Fallback: the largest PNG that still fits under the size cap.
    largest_image_in_dir(set)
}

/// Locate an Android `res` tree and pick `ic_launcher` from a mid-density
/// bucket (x/hdpi first to keep the embedded payload small).
fn detect_android_icon(root: &Path) -> Option<IconData> {
    let res = find_dir_named(root, "res", 6)?;
    const DENSITIES: &[&str] = &[
        "mipmap-xhdpi",
        "mipmap-hdpi",
        "mipmap-xxhdpi",
        "mipmap-mdpi",
        "mipmap-xxxhdpi",
    ];
    const NAMES: &[&str] = &[
        "ic_launcher.png",
        "ic_launcher.webp",
        "ic_launcher_round.png",
        "ic_launcher_round.webp",
    ];
    for d in DENSITIES {
        for n in NAMES {
            if let Some(found) = load_icon_file(&res.join(d).join(n)) {
                return Some(found);
            }
        }
    }
    None
}

/// Largest image file (under the size cap) directly inside `dir`.
fn largest_image_in_dir(dir: &Path) -> Option<IconData> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_ICON_BYTES {
            continue;
        }
        if mime_for(&path).is_none() {
            continue;
        }
        if best.as_ref().map(|(s, _)| meta.len() > *s).unwrap_or(true) {
            best = Some((meta.len(), path));
        }
    }
    best.and_then(|(_, path)| load_icon_file(&path))
}

/// Depth-limited search for a directory by exact name, skipping heavy dirs.
fn find_dir_named(root: &Path, name: &str, max_depth: usize) -> Option<PathBuf> {
    fn walk(dir: &Path, name: &str, depth: usize, max_depth: usize) -> Option<PathBuf> {
        if depth > max_depth {
            return None;
        }
        let mut subdirs = Vec::new();
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            if SKIP_DIRS.contains(&fname.as_ref()) {
                continue;
            }
            if fname == name {
                return Some(entry.path());
            }
            subdirs.push(entry.path());
        }
        for sub in subdirs {
            if let Some(found) = walk(&sub, name, depth + 1, max_depth) {
                return Some(found);
            }
        }
        None
    }
    walk(root, name, 0, max_depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    // A 1x1 transparent PNG — enough bytes to be a "real" file.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn picks_public_favicon_svg() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("public/favicon.svg"), b"<svg/>");
        let icon = detect_in(dir.path(), ProjectType::Vite, None).unwrap();
        assert_eq!(icon.mime, "image/svg+xml");
    }

    #[test]
    fn next_app_router_icon_beats_public() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("app/icon.png"), PNG_1X1);
        write(&dir.path().join("public/favicon.ico"), b"\0\0\x01\0");
        let icon = detect_in(dir.path(), ProjectType::Next, None).unwrap();
        assert_eq!(icon.mime, "image/png");
    }

    #[test]
    fn sveltekit_static_favicon_ico() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("static/favicon.ico"), b"\0\0\x01\0icon");
        let icon = detect_in(dir.path(), ProjectType::Vite, None).unwrap();
        assert_eq!(icon.mime, "image/x-icon");
    }

    #[test]
    fn php_document_root_favicon() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("web/favicon.png"), PNG_1X1);
        let icon = detect_in(dir.path(), ProjectType::Php, Some("web")).unwrap();
        assert_eq!(icon.mime, "image/png");
    }

    #[test]
    fn index_html_link_rel_icon_is_resolved() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("assets/brand.png"), PNG_1X1);
        write(
            &dir.path().join("index.html"),
            br#"<!doctype html><html><head>
                <link rel="icon" type="image/png" href="/assets/brand.png">
            </head></html>"#,
        );
        let icon = detect_in(dir.path(), ProjectType::Static, None).unwrap();
        assert_eq!(icon.mime, "image/png");
    }

    #[test]
    fn remote_link_href_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("index.html"),
            br#"<link rel="icon" href="https://cdn.example.com/favicon.ico">"#,
        );
        assert!(detect_in(dir.path(), ProjectType::Static, None).is_none());
    }

    #[test]
    fn manifest_icons_largest_is_chosen() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("public/pwa-192.png"), PNG_1X1);
        write(
            &dir.path().join("public/manifest.json"),
            br#"{ "icons": [
                { "src": "pwa-64.png", "sizes": "64x64" },
                { "src": "pwa-192.png", "sizes": "192x192" }
            ] }"#,
        );
        let icon = detect_in(dir.path(), ProjectType::Vite, None).unwrap();
        assert_eq!(icon.mime, "image/png");
    }

    #[test]
    fn android_mipmap_xhdpi_launcher() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path()
                .join("app/src/main/res/mipmap-xhdpi/ic_launcher.png"),
            PNG_1X1,
        );
        let icon = detect_in(dir.path(), ProjectType::Android, None).unwrap();
        assert_eq!(icon.mime, "image/png");
    }

    #[test]
    fn xcode_appiconset_uses_contents_json() {
        let dir = tempfile::tempdir().unwrap();
        let set = dir.path().join("MyApp/Assets.xcassets/AppIcon.appiconset");
        write(&set.join("icon-60@2x.png"), PNG_1X1);
        write(&set.join("icon-1024.png"), PNG_1X1);
        write(
            &set.join("Contents.json"),
            br#"{ "images": [
                { "size": "60x60", "scale": "2x", "filename": "icon-60@2x.png" },
                { "size": "1024x1024", "scale": "1x", "filename": "icon-1024.png" }
            ] }"#,
        );
        let icon = detect_in(dir.path(), ProjectType::Xcode, None).unwrap();
        assert_eq!(icon.mime, "image/png");
    }

    #[test]
    fn workspace_app_subdir_favicon_is_found() {
        // The BookSlash case: repo root has no icon, the app lives in
        // `apps/web` and its favicon sits in `apps/web/public`.
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("apps/web/public/apple-touch-icon.png"),
            PNG_1X1,
        );
        let app_dir = dir.path().join("apps/web");
        let icon = detect_in_dirs(&app_dir, dir.path(), ProjectType::Node, None).unwrap();
        assert_eq!(icon.mime, "image/png");
    }

    #[test]
    fn repo_root_icon_is_used_when_workspace_subdir_has_none() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("public/favicon.svg"), b"<svg/>");
        let app_dir = dir.path().join("apps/web"); // empty / nonexistent
        let icon = detect_in_dirs(&app_dir, dir.path(), ProjectType::Node, None).unwrap();
        assert_eq!(icon.mime, "image/svg+xml");
    }

    #[test]
    fn nothing_present_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("README.md"), b"# hi");
        assert!(detect_in(dir.path(), ProjectType::Node, None).is_none());
    }

    #[test]
    fn data_url_is_base64_encoded() {
        let icon = IconData {
            mime: "image/png",
            bytes: vec![0, 1, 2, 3],
        };
        assert_eq!(icon.to_data_url(), "data:image/png;base64,AAECAw==");
    }
}
