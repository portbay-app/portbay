//! Migration import from other local-dev tools.
//!
//! Three source tools (`Herd`, `ServBay`, `MAMP`), each with a small
//! detect-and-parse module. Detection probes filesystem paths the tool
//! is known to use; parsing returns a uniform `ImportedSite` list the
//! GUI can show before commit.
//!
//! Best-effort mapping. Where the source tool stores something
//! PortBay doesn't carry (e.g. an explicit Apache document index),
//! we drop it on the floor; the user edits in the detail panel after
//! import.

pub mod error;
pub mod herd;
pub mod mamp;
pub mod servbay;

use serde::{Deserialize, Serialize};

pub use error::{ImportError, Result};

use crate::registry::ProjectType;

/// Stable identifier for each known source tool. Surface in the GUI and
/// in the registry's `tags` on imported projects (`source:herd` etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    Herd,
    ServBay,
    Mamp,
}

impl ImportSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Herd => "Laravel Herd",
            Self::ServBay => "ServBay",
            Self::Mamp => "MAMP",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::Herd => "source:herd",
            Self::ServBay => "source:servbay",
            Self::Mamp => "source:mamp",
        }
    }
}

/// One site extracted from a source tool. The fields are deliberately
/// the same names the registry's `Project` struct uses so the import
/// command can construct a `Project` straight from this shape.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedSite {
    pub source: ImportSource,
    /// Absolute filesystem path to the project root.
    pub path: String,
    /// Hostname as the source tool serves it (e.g. `myapp.test`).
    pub hostname: String,
    /// PHP version string the source tool records (`8.3`, `7.4`, …)
    /// when available. `None` for non-PHP projects.
    pub php_version: Option<String>,
    /// True if the source tool serves this site over HTTPS.
    pub https: bool,
    /// Document root relative to `path`, when the source serves files out of a
    /// sub-directory (e.g. `public` for a Laravel/PHP front-controller app).
    /// `None` when the project is served straight from `path`.
    pub document_root: Option<String>,
    /// Project type the source clearly implies (e.g. PHP for an FPM vhost,
    /// Static for a plain file-server vhost). `None` lets the import command
    /// fall back to its php-version heuristic.
    pub kind_hint: Option<ProjectType>,
    /// Suggested project id derived from the path's final component.
    pub suggested_id: String,
    /// Suggested project name (human-readable; falls back to id).
    pub suggested_name: String,
}

impl ImportedSite {
    pub(crate) fn from_parts(
        source: ImportSource,
        path: String,
        hostname: String,
        php_version: Option<String>,
        https: bool,
    ) -> Self {
        let id = derive_id(&path);
        Self {
            source,
            suggested_name: derive_name(&path).unwrap_or_else(|| id.clone()),
            suggested_id: id,
            path,
            hostname,
            php_version,
            https,
            document_root: None,
            kind_hint: None,
        }
    }
}

/// What a `detect_sources` call returns per source — whether anything
/// was found and how many sites are recoverable. The GUI uses
/// `site_count` to gate the "Import N sites from X" button.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSource {
    pub source: ImportSource,
    pub label: &'static str,
    /// True iff the source tool's config or vhost dir is present.
    pub present: bool,
    /// Number of sites that parsed without error.
    pub site_count: usize,
    /// Free-form note for the GUI (e.g. "uses NGINX vhost format").
    pub note: Option<String>,
}

/// Run every detector and return the per-source summary.
pub fn detect_all() -> Vec<DetectedSource> {
    vec![herd::detect(), servbay::detect(), mamp::detect()]
}

/// Parse all sites from the given source. Errors per site are logged
/// and skipped — the caller gets every site the parser could recover.
pub fn read_all(source: ImportSource) -> Result<Vec<ImportedSite>> {
    match source {
        ImportSource::Herd => herd::read_sites(),
        ImportSource::ServBay => servbay::read_sites(),
        ImportSource::Mamp => mamp::read_sites(),
    }
}

fn derive_id(path: &str) -> String {
    let last = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let mut out = String::with_capacity(last.len());
    let mut last_dash = true;
    for ch in last.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "imported".to_string()
    } else {
        trimmed
    }
}

fn derive_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_id_lowercases_and_hyphenates() {
        assert_eq!(derive_id("/Users/x/Sites/API Gateway"), "api-gateway");
        assert_eq!(derive_id("/Users/x/MyApp"), "myapp");
        assert_eq!(derive_id("/Users/x/__weird__"), "weird");
    }

    #[test]
    fn derive_id_falls_back_to_imported_for_empty() {
        assert_eq!(derive_id("/"), "imported");
        assert_eq!(derive_id(""), "imported");
    }

    #[test]
    fn imported_site_has_id_and_name_derived_from_path() {
        let s = ImportedSite::from_parts(
            ImportSource::Herd,
            "/Users/x/MyApp".into(),
            "myapp.test".into(),
            Some("8.3".into()),
            true,
        );
        assert_eq!(s.suggested_id, "myapp");
        assert_eq!(s.suggested_name, "MyApp");
        assert!(s.https);
    }

    #[test]
    fn detect_all_returns_one_entry_per_source() {
        let result = detect_all();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].source, ImportSource::Herd);
        assert_eq!(result[1].source, ImportSource::ServBay);
        assert_eq!(result[2].source, ImportSource::Mamp);
    }
}
