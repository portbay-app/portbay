//! MAMP importer.
//!
//! MAMP serves sites via Apache vhost blocks defined in
//! `/Applications/MAMP/conf/apache/extra/httpd-vhosts.conf`. The
//! parser here is intentionally narrow: it recognises
//! `<VirtualHost …>` blocks and pulls `ServerName` and `DocumentRoot`
//! from each. Apache's full grammar is rich; we ignore everything
//! we don't recognise.

use std::path::PathBuf;

use crate::import::error::{ImportError, Result};
use crate::import::{DetectedSource, ImportSource, ImportedSite};

const DEFAULT_VHOSTS_PATH: &str = "/Applications/MAMP/conf/apache/extra/httpd-vhosts.conf";

pub fn detect() -> DetectedSource {
    let path = PathBuf::from(DEFAULT_VHOSTS_PATH);
    let present = path.exists();
    let site_count = if present {
        read_sites()
            .map(|v| crate::import::dedupe_sites(v).len())
            .unwrap_or(0)
    } else {
        0
    };
    DetectedSource {
        source: ImportSource::Mamp,
        label: ImportSource::Mamp.label(),
        present,
        site_count,
        note: Some("uses Apache httpd-vhosts.conf".into()),
    }
}

pub fn read_sites() -> Result<Vec<ImportedSite>> {
    let path = PathBuf::from(DEFAULT_VHOSTS_PATH);
    if !path.exists() {
        return Err(ImportError::SourceMissing(path));
    }
    let contents = std::fs::read_to_string(&path).map_err(|e| ImportError::io(&path, e))?;
    let blocks = parse_vhosts(&contents);
    let mut out: Vec<ImportedSite> = Vec::new();
    for block in blocks {
        if block.server_name.is_empty() || block.document_root.is_empty() {
            continue;
        }
        out.push(ImportedSite::from_parts(
            ImportSource::Mamp,
            block.document_root,
            block.server_name,
            None,
            block.https,
        ));
    }
    Ok(out)
}

#[derive(Debug, Default)]
struct ParsedVhost {
    server_name: String,
    document_root: String,
    https: bool,
}

/// Locate `<VirtualHost …>` … `</VirtualHost>` blocks (case-insensitive)
/// and pull `ServerName` + `DocumentRoot` from each.
fn parse_vhosts(input: &str) -> Vec<ParsedVhost> {
    let mut out: Vec<ParsedVhost> = Vec::new();
    let lc = input.to_ascii_lowercase();
    let bytes = lc.as_bytes();
    let mut i = 0;
    while let Some(start_off) = find_substr(&lc[i..], "<virtualhost") {
        let start_lt = i + start_off;
        // The block spans from `<VirtualHost …>` up to and including
        // `</VirtualHost>`. We need to use the original input slice so
        // case is preserved on the directive values.
        let close_open = match find_substr(&lc[start_lt..], ">") {
            Some(o) => start_lt + o,
            None => break,
        };
        let close_tag = match find_substr(&lc[close_open..], "</virtualhost>") {
            Some(c) => close_open + c,
            None => break,
        };

        let header = &input[start_lt..=close_open];
        let body = &input[close_open + 1..close_tag];

        let https = header.to_ascii_lowercase().contains(":443");
        let mut block = parse_block_body(body);
        if block.https {
            // Already detected via directives — leave it; otherwise
            // fall back to the header's port hint.
        } else {
            block.https = https;
        }
        out.push(block);

        i = close_tag + "</virtualhost>".len();
        if i >= bytes.len() {
            break;
        }
    }
    out
}

fn parse_block_body(body: &str) -> ParsedVhost {
    let mut out = ParsedVhost::default();
    for raw_line in body.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = strip_prefix_ci(line, "servername") {
            out.server_name = rest.trim().to_string();
        } else if let Some(rest) = strip_prefix_ci(line, "documentroot") {
            // Apache config quotes paths with spaces in double quotes.
            let s = rest.trim().trim_matches('"');
            out.document_root = s.to_string();
        } else if let Some(rest) = strip_prefix_ci(line, "sslengine") {
            if rest.trim().eq_ignore_ascii_case("on") {
                out.https = true;
            }
        }
    }
    out
}

fn find_substr(hay: &str, needle: &str) -> Option<usize> {
    hay.find(needle)
}

fn strip_prefix_ci<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    if line.len() < prefix.len() {
        return None;
    }
    let head = &line[..prefix.len()];
    if head.eq_ignore_ascii_case(prefix) {
        Some(&line[prefix.len()..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# MAMP vhosts.
<VirtualHost *:80>
    ServerName myapp.test
    DocumentRoot "/Users/x/Sites/myapp"
</VirtualHost>

<VirtualHost *:443>
    ServerName secure.test
    DocumentRoot "/Users/x/Sites/secure"
    SSLEngine on
</VirtualHost>
"#;

    #[test]
    fn parses_two_vhosts() {
        let blocks = parse_vhosts(SAMPLE);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].server_name, "myapp.test");
        assert_eq!(blocks[0].document_root, "/Users/x/Sites/myapp");
        assert!(!blocks[0].https);
        assert_eq!(blocks[1].server_name, "secure.test");
        assert!(blocks[1].https);
    }

    #[test]
    fn handles_lowercase_directives() {
        let s = r#"<virtualhost *:80>
            servername x.test
            documentroot /p
        </virtualhost>"#;
        let blocks = parse_vhosts(s);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].server_name, "x.test");
        assert_eq!(blocks[0].document_root, "/p");
    }
}
