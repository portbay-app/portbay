//! ServBay importer.
//!
//! ServBay stores its main control state in encrypted files
//! (`config.data` and friends), but the actual site definitions land
//! as NGINX vhost configs on disk. We scan those vhost directories
//! and pull out `server_name`, `root`, and TLS state from each
//! `server { … }` block.
//!
//! Common locations:
//!
//! - `~/Library/Application Support/ServBay/disabled-vhosts/` — sites
//!   the user has explicitly turned off (kept for re-enabling).
//! - `/Applications/ServBay/etc/nginx/sites/` — enabled sites on a
//!   stock install (may vary per ServBay version; we probe a small
//!   list).
//!
//! NGINX config is brace-delimited and order-independent inside a
//! block; the lightweight parser below extracts the three directives
//! we care about and ignores everything else.

use std::path::{Path, PathBuf};

use crate::import::error::{ImportError, Result};
use crate::import::{DetectedSource, ImportSource, ImportedSite};
use crate::registry::ProjectType;

/// Conventional document-root sub-directory names. When a vhost's `root` ends
/// in one of these, the *project* is the parent directory and the leaf is its
/// document root — so a site at `…/tribal-house-cms/public` imports as the
/// project "tribal-house-cms" with document root "public", not as "public".
const DOC_ROOT_DIRS: &[&str] = &[
    "public",
    "web",
    "html",
    "public_html",
    "www",
    "dist",
    "build",
    "out",
];

/// Directories the importer scans for ServBay vhost files. All existing ones
/// are scanned (not just the first) so user-added and auto-generated sites both
/// come through.
fn candidate_vhost_dirs() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(mut home_data) = dirs::data_dir() {
        home_data.push("ServBay");
        paths.push(home_data.join("vhosts"));
        paths.push(home_data.join("disabled-vhosts"));
    }
    // ServBay's real on-disk layout (stock install): `manual-vhosts` holds the
    // sites the user added, `enabled-dev-vhosts` the auto-generated dev mirrors.
    // The older `sites` / `sites-enabled` names are kept as probes for other
    // ServBay versions. Without `manual-vhosts` the importer found nothing on a
    // current install — the cause of the missing Tribal House CMS import.
    let nginx = PathBuf::from("/Applications/ServBay/etc/nginx");
    paths.push(nginx.join("manual-vhosts"));
    paths.push(nginx.join("enabled-dev-vhosts"));
    paths.push(nginx.join("sites"));
    paths.push(nginx.join("sites-enabled"));
    paths
}

pub fn detect() -> DetectedSource {
    let dirs: Vec<PathBuf> = candidate_vhost_dirs()
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    let present = !dirs.is_empty();
    let site_count = if present {
        read_sites()
            .map(|v| crate::import::dedupe_sites(v).len())
            .unwrap_or(0)
    } else {
        0
    };
    DetectedSource {
        source: ImportSource::ServBay,
        label: ImportSource::ServBay.label(),
        present,
        site_count,
        note: Some("uses NGINX vhost format".into()),
    }
}

pub fn read_sites() -> Result<Vec<ImportedSite>> {
    let dirs: Vec<PathBuf> = candidate_vhost_dirs()
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    if dirs.is_empty() {
        return Err(ImportError::SourceMissing(PathBuf::from("ServBay vhosts")));
    }

    let mut out: Vec<ImportedSite> = Vec::new();
    for dir in &dirs {
        let entries = std::fs::read_dir(dir).map_err(|e| ImportError::io(dir, e))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // .conf or .conf.disabled — match either.
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if !name.contains(".conf") {
                continue;
            }
            let contents = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            for site in parse_vhost(&contents) {
                let hostname = site.server_name;
                let root = site.root.unwrap_or_default();
                // Skip vhosts we can't turn into a real PortBay project:
                //   - no `root` → a reverse-proxy dev site (proxy_pass); PortBay
                //     runs the dev server itself, so there's nothing to import.
                //   - wildcard `server_name` (e.g. `*.servbay.demo`) → ServBay's
                //     catch-all, not a project.
                if hostname.is_empty() || root.is_empty() || hostname.contains('*') {
                    continue;
                }
                let (project_path, document_root) = split_document_root(&root);
                let kind_hint = Some(if site.is_php {
                    ProjectType::Php
                } else {
                    ProjectType::Static
                });
                let mut imported = ImportedSite::from_parts(
                    ImportSource::ServBay,
                    project_path,
                    hostname,
                    None,
                    site.https,
                );
                imported.document_root = document_root;
                imported.kind_hint = kind_hint;
                out.push(imported);
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Default)]
struct ParsedVhost {
    server_name: String,
    root: Option<String>,
    https: bool,
    /// The vhost includes a PHP-FPM config or routes through a `.php` front
    /// controller — i.e. it's a PHP app, not a plain static site.
    is_php: bool,
}

/// Split a conventional document-root sub-dir off a vhost `root`. Returns
/// `(project_path, document_root)`: when `root` ends in a known doc-root name
/// (`public`, `web`, …) the project is the parent and the leaf becomes the
/// document root; otherwise the project is served straight from `root`.
fn split_document_root(root: &str) -> (String, Option<String>) {
    let trimmed = root.trim_end_matches('/');
    let path = Path::new(trimmed);
    if let Some(leaf) = path.file_name().and_then(|s| s.to_str()) {
        if DOC_ROOT_DIRS.contains(&leaf) {
            if let Some(parent) = path.parent().and_then(|p| p.to_str()) {
                if !parent.is_empty() {
                    return (parent.to_string(), Some(leaf.to_string()));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}

/// Strip a trailing `;` and a single pair of surrounding single/double quotes.
/// ServBay quotes roots that contain spaces (e.g. `'…/Tribal House/…'`); left
/// unquoted the literal quotes would become part of the path.
fn unquote(s: &str) -> String {
    let s = s.trim().trim_end_matches(';').trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// Lightweight NGINX vhost parser. Walks each `server { … }` block and
/// pulls `server_name`, `root`, and `listen 443 ssl` (presence implies
/// HTTPS). Comments are stripped first.
fn parse_vhost(input: &str) -> Vec<ParsedVhost> {
    let stripped = strip_comments(input);
    let mut out: Vec<ParsedVhost> = Vec::new();

    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Find the next `server` keyword followed by `{` (skipping
        // `server_name`, which has an underscore).
        if let Some(start) = find_server_block(&stripped[i..]) {
            let block_start = i + start;
            // Find the matching closing brace from the opening one.
            let open = match stripped[block_start..].find('{') {
                Some(o) => block_start + o,
                None => break,
            };
            let close = match matching_brace(&stripped, open) {
                Some(c) => c,
                None => break,
            };
            let block = &stripped[open + 1..close];
            out.push(parse_block(block));
            i = close + 1;
        } else {
            break;
        }
    }

    out
}

fn find_server_block(s: &str) -> Option<usize> {
    let mut idx = 0;
    while idx < s.len() {
        let rest = &s[idx..];
        let pos = rest.find("server")?;
        let after = idx + pos + "server".len();
        // Reject `server_name` and `server_tokens` etc.
        let ch = s.as_bytes().get(after).copied();
        if matches!(ch, Some(b'_')) {
            idx = after;
            continue;
        }
        // Must be followed (after optional whitespace) by `{`.
        let mut j = after;
        while j < s.len() && matches!(s.as_bytes()[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if j < s.len() && s.as_bytes()[j] == b'{' {
            return Some(idx + pos);
        }
        idx = after;
    }
    None
}

fn matching_brace(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0;
    for (off, b) in bytes[open_idx..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open_idx + off);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_block(block: &str) -> ParsedVhost {
    let mut out = ParsedVhost::default();
    for raw_line in block.split(';') {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("server_name") {
            out.server_name = rest.split_whitespace().next().unwrap_or("").to_string();
        } else if let Some(rest) = line.strip_prefix("root") {
            out.root = Some(unquote(rest));
        } else if line.contains("listen") && line.contains("ssl") {
            out.https = true;
        }
        // A PHP-FPM include or a `.php` front controller (try_files/index)
        // marks this as a PHP app rather than a static site.
        if (line.starts_with("include") && line.contains("php-fpm")) || line.contains(".php") {
            out.is_php = true;
        }
    }
    out
}

/// Strip `#` line comments. Not exhaustive (won't handle `#` inside
/// quoted strings) but adequate for the small directive set we parse.
fn strip_comments(input: &str) -> String {
    input
        .lines()
        .map(|line| match line.find('#') {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# A ServBay-style vhost.
server {
    listen 80;
    server_name myapp.test;
    root /Users/x/Sites/myapp/public;

    access_log /var/log/myapp.log;
    location / {
        try_files $uri $uri/ /index.php?$query_string;
    }
}

server {
    listen 443 ssl;
    server_name secure.test;
    root /Users/x/Sites/secure;
}
"#;

    #[test]
    fn parse_vhost_extracts_two_servers() {
        let blocks = parse_vhost(SAMPLE);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].server_name, "myapp.test");
        assert_eq!(
            blocks[0].root.as_deref(),
            Some("/Users/x/Sites/myapp/public")
        );
        assert!(!blocks[0].https);
        assert_eq!(blocks[1].server_name, "secure.test");
        assert!(blocks[1].https);
    }

    #[test]
    fn parse_skips_server_name_keyword_collision() {
        let input =
            "server_name should_not_open_a_block;\nserver {\n  server_name x.test;\n  root /p;\n}";
        let blocks = parse_vhost(input);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].server_name, "x.test");
    }

    #[test]
    fn strip_comments_drops_hash_lines() {
        let s = strip_comments("server { # inline\n  root /p; # trailing\n}");
        assert!(!s.contains("inline"));
        assert!(!s.contains("trailing"));
        assert!(s.contains("/p"));
    }

    // The exact shape of a ServBay `manual-vhosts/*.conf` PHP site (Tribal
    // House CMS): single-quoted root with a space, php-fpm include, router.php.
    const TRIBAL: &str = r#"
server {
    listen 80;
    server_name tribal-house.localhost;
    access_log /Applications/ServBay/logs/nginx/tribal-house.localhost.log;
    index index.php index.html index.htm;
    root '/Volumes/DevSSD/Projects/Clients/Tribal House/tribal-house-cms/public';
    include enable-php-fpm-default-pathinfo.conf;
    location / {
        try_files $uri /router.php?$query_string;
    }
}
"#;

    #[test]
    fn parses_quoted_root_with_spaces_and_detects_php() {
        let blocks = parse_vhost(TRIBAL);
        assert_eq!(blocks.len(), 1);
        let b = &blocks[0];
        assert_eq!(b.server_name, "tribal-house.localhost");
        // Quotes stripped, space preserved.
        assert_eq!(
            b.root.as_deref(),
            Some("/Volumes/DevSSD/Projects/Clients/Tribal House/tribal-house-cms/public")
        );
        assert!(b.is_php, "php-fpm include + router.php should flag PHP");
        assert!(!b.https);
    }

    #[test]
    fn split_document_root_peels_public() {
        let (path, doc) = split_document_root(
            "/Volumes/DevSSD/Projects/Clients/Tribal House/tribal-house-cms/public",
        );
        assert_eq!(
            path,
            "/Volumes/DevSSD/Projects/Clients/Tribal House/tribal-house-cms"
        );
        assert_eq!(doc.as_deref(), Some("public"));
    }

    #[test]
    fn split_document_root_leaves_plain_root_alone() {
        let (path, doc) = split_document_root("/Users/x/Sites/plain-static");
        assert_eq!(path, "/Users/x/Sites/plain-static");
        assert!(doc.is_none());
    }

    #[test]
    fn unquote_strips_single_and_double_quotes() {
        assert_eq!(unquote("'/a/b c'"), "/a/b c");
        assert_eq!(unquote("\"/a/b\""), "/a/b");
        assert_eq!(unquote("/a/b"), "/a/b");
        assert_eq!(unquote(" '/a' "), "/a");
    }

    #[test]
    fn wildcard_and_proxy_vhosts_are_excluded_by_field_shape() {
        // Catch-all: wildcard server_name, no root → would be filtered in
        // read_sites (hostname.contains('*') || root.is_empty()).
        let catchall = "server {\n  listen 80;\n  server_name *.servbay.demo;\n  return 404;\n}";
        let b = &parse_vhost(catchall)[0];
        assert!(b.server_name.contains('*'));
        assert!(b.root.is_none());

        // Reverse-proxy dev site: has a server_name but no root.
        let proxy = "server {\n  server_name bookslash.localhost;\n  location / { proxy_pass http://127.0.0.1:3000; }\n}";
        let pb = &parse_vhost(proxy)[0];
        assert_eq!(pb.server_name, "bookslash.localhost");
        assert!(pb.root.is_none());
    }
}
