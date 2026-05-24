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

/// Directories the importer scans for ServBay vhost files, in order
/// of likelihood. The first one found wins.
fn candidate_vhost_dirs() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(mut home_data) = dirs::data_dir() {
        home_data.push("ServBay");
        paths.push(home_data.join("vhosts"));
        paths.push(home_data.join("disabled-vhosts"));
    }
    paths.push(PathBuf::from("/Applications/ServBay/etc/nginx/sites"));
    paths.push(PathBuf::from(
        "/Applications/ServBay/etc/nginx/sites-enabled",
    ));
    paths
}

pub fn detect() -> DetectedSource {
    let dirs: Vec<PathBuf> = candidate_vhost_dirs()
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    let present = !dirs.is_empty();
    let site_count = if present {
        read_sites().map(|v| v.len()).unwrap_or(0)
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
                if hostname.is_empty() || root.is_empty() {
                    continue;
                }
                out.push(ImportedSite::from_parts(
                    ImportSource::ServBay,
                    root,
                    hostname,
                    None,
                    site.https,
                ));
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
            out.root = Some(rest.trim().trim_matches(';').trim().to_string());
        } else if line.contains("listen") && line.contains("ssl") {
            out.https = true;
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

#[allow(dead_code)]
fn _typecheck_path_export() -> &'static Path {
    Path::new(".")
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
}
