//! Laravel Herd importer.
//!
//! Herd persists its sites under `~/Library/Application Support/Herd/config.json`.
//! The file is plain JSON; the keys we care about are documented at
//! <https://herd.laravel.com> and observed on real Herd installs:
//!
//! ```json
//! {
//!   "sites": [
//!     { "path": "/Users/x/Sites/myapp", "tld": "test",
//!       "php_version": "8.3", "secure": true }
//!   ],
//!   "parked_paths": ["/Users/x/Herd"]
//! }
//! ```
//!
//! `parked_paths` is a list of directories Herd treats as auto-
//! registered (every child dir becomes a site). We expand each one
//! into individual `ImportedSite` entries by listing the directory
//! and applying the parked tld + php_version defaults.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::import::error::{ImportError, Result};
use crate::import::{DetectedSource, ImportSource, ImportedSite};

const CONFIG_FILENAME: &str = "config.json";

#[derive(Debug, Deserialize)]
struct HerdConfig {
    #[serde(default)]
    sites: Vec<HerdSite>,
    #[serde(default)]
    parked_paths: Vec<String>,
    #[serde(default = "default_tld")]
    tld: String,
    #[serde(default)]
    php_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HerdSite {
    path: String,
    #[serde(default)]
    tld: Option<String>,
    #[serde(default)]
    php_version: Option<String>,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    alias: Option<String>,
}

fn default_tld() -> String {
    "test".into()
}

pub fn detect() -> DetectedSource {
    let path = config_path();
    let present = path.as_ref().map(|p| p.exists()).unwrap_or(false);
    let site_count = if present {
        read_sites().map(|v| v.len()).unwrap_or(0)
    } else {
        0
    };
    DetectedSource {
        source: ImportSource::Herd,
        label: ImportSource::Herd.label(),
        present,
        site_count,
        note: None,
    }
}

pub fn read_sites() -> Result<Vec<ImportedSite>> {
    let path = config_path().ok_or_else(|| {
        ImportError::SourceMissing(PathBuf::from("~/Library/Application Support/Herd"))
    })?;
    if !path.exists() {
        return Err(ImportError::SourceMissing(path));
    }

    let bytes = std::fs::read(&path).map_err(|e| ImportError::io(&path, e))?;
    let cfg: HerdConfig =
        serde_json::from_slice(&bytes).map_err(|e| ImportError::malformed(&path, e.to_string()))?;

    let mut out: Vec<ImportedSite> = Vec::new();

    // Explicit sites list — one entry per known site.
    for site in &cfg.sites {
        let tld = site
            .tld
            .as_deref()
            .unwrap_or(&cfg.tld)
            .trim_start_matches('.')
            .to_string();
        let hostname = build_hostname(site.alias.as_deref(), &site.path, &tld);
        let php_version = site.php_version.clone().or_else(|| cfg.php_version.clone());
        out.push(ImportedSite::from_parts(
            ImportSource::Herd,
            site.path.clone(),
            hostname,
            php_version,
            site.secure,
        ));
    }

    // Parked paths — every directory inside is an auto-registered site.
    for parked in &cfg.parked_paths {
        let dir = expand_tilde(parked);
        if !dir.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let path_str = p.to_string_lossy().into_owned();
            let hostname = build_hostname(None, &path_str, &cfg.tld);
            out.push(ImportedSite::from_parts(
                ImportSource::Herd,
                path_str,
                hostname,
                cfg.php_version.clone(),
                false, // parked sites default to HTTP — Herd's "secure"
                       // toggle is per-site and we don't track parked
                       // overrides today.
            ));
        }
    }

    Ok(out)
}

fn config_path() -> Option<PathBuf> {
    let mut p = dirs::config_dir().or_else(dirs::data_dir)?;
    // dirs::config_dir on macOS → ~/Library/Application Support
    p.push("Herd");
    p.push(CONFIG_FILENAME);
    Some(p)
}

fn build_hostname(alias: Option<&str>, path: &str, tld: &str) -> String {
    if let Some(alias) = alias {
        if !alias.is_empty() {
            return alias.to_string();
        }
    }
    let stem = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("imported")
        .to_ascii_lowercase();
    format!("{stem}.{tld}")
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let json = serde_json::json!({
            "sites": [
                { "path": "/Users/x/MyApp", "php_version": "8.3", "secure": true }
            ]
        });
        let cfg: HerdConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.sites.len(), 1);
        assert_eq!(cfg.sites[0].path, "/Users/x/MyApp");
        assert_eq!(cfg.sites[0].php_version.as_deref(), Some("8.3"));
        assert!(cfg.sites[0].secure);
        assert_eq!(cfg.tld, "test");
    }

    #[test]
    fn falls_back_to_global_tld_and_php() {
        let json = serde_json::json!({
            "tld": "local",
            "php_version": "8.2",
            "sites": [
                { "path": "/Users/x/A" },
                { "path": "/Users/x/B", "tld": "test" }
            ]
        });
        let cfg: HerdConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.tld, "local");
        assert_eq!(cfg.php_version.as_deref(), Some("8.2"));
        assert!(cfg.sites[0].tld.is_none());
        assert_eq!(cfg.sites[1].tld.as_deref(), Some("test"));
    }

    #[test]
    fn build_hostname_uses_alias_when_set() {
        let h = build_hostname(Some("api.myapp.test"), "/p/myapp", "test");
        assert_eq!(h, "api.myapp.test");
    }

    #[test]
    fn build_hostname_lowercases_and_appends_tld() {
        let h = build_hostname(None, "/Users/x/MyApp", "test");
        assert_eq!(h, "myapp.test");
    }

    #[test]
    fn build_hostname_handles_empty_alias() {
        let h = build_hostname(Some(""), "/Users/x/MyApp", "test");
        assert_eq!(h, "myapp.test");
    }
}
