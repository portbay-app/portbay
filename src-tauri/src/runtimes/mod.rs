//! Language-runtime container — detect-first runtime management.
//!
//! Replaces the PHP-specific surface with a generic abstraction that
//! covers every dev runtime PortBay knows about (PHP, Node, Python,
//! Go, Ruby to start). The model is **detect, don't install**:
//! PortBay scans for runtimes that already exist on the user's
//! machine (Homebrew, asdf, mise, system PATH) and surfaces them. We
//! never bundle a compiler; installing a missing version is delegated
//! to the user's existing package manager via a follow-up "Add
//! version" flow (a separate kanban step).
//!
//! Design:
//! - One file per language under this module. Each implements a
//!   `LanguageRuntime` trait that returns its display name, a list
//!   of detected installs, and the declarative config-panel spec the
//!   frontend renders.
//! - The IPC surface (`commands/runtimes.rs`) iterates registered
//!   languages and concatenates them into a single `Vec<LanguageView>`
//!   the frontend can render in one pass.
//!
//! Scope of *this* commit:
//! - Detection + list_runtimes IPC + /languages route + ServBay-style
//!   sidebar UI. The "Add version" install flow, per-version PortBay
//!   config dirs, and the registry v1→v2 migration are deferred to
//!   follow-up commits on the same kanban card.

pub mod env;
pub mod go;
pub mod node;
pub mod php;
pub mod python;
pub mod ruby;

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a detected install came from. Drives the install-source pill
/// the frontend renders next to each version.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallSource {
    /// Homebrew formula (Apple Silicon or Intel prefix).
    Homebrew,
    /// asdf-vm — `~/.asdf/installs/<lang>/<ver>/`.
    Asdf,
    /// mise (formerly rtx) — `~/.local/share/mise/installs/<lang>/<ver>/`.
    Mise,
    /// nvm — `~/.nvm/versions/node/<ver>/`. Node only.
    Nvm,
    /// pyenv — `~/.pyenv/versions/<ver>/`. Python only.
    Pyenv,
    /// Found on `$PATH` without a recognised version manager.
    System,
    /// Added by hand via "Add by path" — a binary the detector didn't find.
    Manual,
}

/// One detected install of a particular runtime. Generic across
/// languages; per-language detail (e.g. PHP's loaded extensions) is
/// returned separately via `tabs` in the LanguageView.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInstall {
    /// Semantic version label, e.g. "8.3", "22.11.0", "3.12".
    pub version: String,
    /// Path to the primary binary (e.g. `php`, `node`, `python3`).
    pub binary: PathBuf,
    /// Where the install came from.
    pub source: InstallSource,
    /// PortBay-managed config dir for this version. None when the
    /// runtime has no config (Node, Go); Some for runtimes with
    /// daemon-side config (PHP-FPM). Deferred follow-up populates
    /// this on first use rather than at detect time.
    pub config_dir: Option<PathBuf>,
}

/// One detail tab inside a version's config panel. The frontend
/// renders the tabs declaratively — no per-language UI branches.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigTab {
    pub id: String,
    pub label: String,
    /// Key-value rows shown in this tab. For now everything is
    /// readonly metadata; editing surfaces ship in a follow-up.
    pub rows: Vec<KvRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KvRow {
    pub label: String,
    pub value: String,
    /// Optional hint shown beneath the value (e.g. install path, doc link).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// When true, the value is rendered as a monospace path the user
    /// can click to reveal in Finder.
    #[serde(default)]
    pub is_path: bool,
}

/// One detected version, ready for the frontend. Couples an
/// `RuntimeInstall` with its config tabs so the panel doesn't need a
/// second round-trip to populate the right pane.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionView {
    pub install: RuntimeInstall,
    pub tabs: Vec<ConfigTab>,
}

/// One language entry in the sidebar. The list of versions is
/// returned in priority order (newest first).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageView {
    /// Stable id, e.g. "php", "node", "python".
    pub id: String,
    /// Display label, e.g. "PHP", "Node.js", "Python".
    pub display_name: String,
    /// Family of detected versions on this machine. Empty list →
    /// the frontend shows an "install via Homebrew" hint.
    pub versions: Vec<VersionView>,
    /// Hint shown when `versions` is empty, e.g. "brew install php".
    pub install_hint: String,
    /// The version marked as this language's default (from the registry's
    /// runtime settings), or `None` when no default is set.
    pub default_version: Option<String>,
}

/// Trait every supported language implements. Pulling the surface
/// behind a trait makes adding a new language a one-file addition
/// without touching the IPC layer.
pub trait LanguageRuntime {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn install_hint(&self) -> &'static str;
    /// Detect every install on this machine.
    fn detect(&self) -> Vec<RuntimeInstall>;
    /// Probe an arbitrary binary's version string, for the "add by path"
    /// flow. Default runs `<binary> --version`; runtimes whose flag differs
    /// (Go uses `version`) override this.
    fn probe_version(&self, binary: &std::path::Path) -> Option<String> {
        version_from(binary, "--version")
    }
    /// Per-version config tabs. Default: a single "Info" tab that
    /// shows the binary path and source. PHP overrides this with
    /// FPM / php.ini / extensions tabs.
    fn tabs(&self, install: &RuntimeInstall) -> Vec<ConfigTab> {
        vec![ConfigTab {
            id: "info".into(),
            label: "Info".into(),
            rows: vec![
                KvRow {
                    label: "Binary".into(),
                    value: install.binary.to_string_lossy().into_owned(),
                    hint: None,
                    is_path: true,
                },
                KvRow {
                    label: "Source".into(),
                    value: source_label(install.source).into(),
                    hint: None,
                    is_path: false,
                },
            ],
        }]
    }
}

/// Human label for a source, used in tabs + the sidebar pill.
pub fn source_label(s: InstallSource) -> &'static str {
    match s {
        InstallSource::Homebrew => "Homebrew",
        InstallSource::Asdf => "asdf",
        InstallSource::Mise => "mise",
        InstallSource::Nvm => "nvm",
        InstallSource::Pyenv => "pyenv",
        InstallSource::System => "System",
        InstallSource::Manual => "Manual",
    }
}

/// The registry of every language PortBay knows about. Adding a new
/// language: add a file under `src/runtimes/`, implement the trait,
/// push it here.
fn registry() -> Vec<Box<dyn LanguageRuntime>> {
    vec![
        Box::new(php::PhpRuntime),
        Box::new(node::NodeRuntime),
        Box::new(python::PythonRuntime),
        Box::new(go::GoRuntime),
        Box::new(ruby::RubyRuntime),
    ]
}

/// Look up a single language by its stable id (for the add-by-path flow).
pub fn runtime_by_id(id: &str) -> Option<Box<dyn LanguageRuntime>> {
    registry().into_iter().find(|r| r.id() == id)
}

/// Top-level IPC entry point: scan every language, fold in the user's
/// manually-added installs, and mark each language's default version.
/// Per-version `tabs` are pre-computed so the frontend renders the whole
/// panel without an extra round-trip.
///
/// `manual` and `defaults` come from the registry's [`RuntimeSettings`]; a
/// manual install whose binary the detector already surfaced is skipped (no
/// duplicate row).
pub fn list_all(
    manual: &[crate::registry::ManualRuntime],
    defaults: &std::collections::BTreeMap<String, String>,
) -> Vec<LanguageView> {
    registry()
        .into_iter()
        .map(|lang| {
            let id = lang.id();
            let mut installs = lang.detect();

            // Fold in manual installs for this language, skipping any whose
            // binary the detector already found (dedup by canonical path).
            let detected: HashSet<PathBuf> = installs
                .iter()
                .map(|i| i.binary.canonicalize().unwrap_or_else(|_| i.binary.clone()))
                .collect();
            for m in manual.iter().filter(|m| m.lang == id) {
                let canon = m.binary.canonicalize().unwrap_or_else(|_| m.binary.clone());
                if detected.contains(&canon) {
                    continue;
                }
                installs.push(RuntimeInstall {
                    version: m.version.clone(),
                    binary: m.binary.clone(),
                    source: InstallSource::Manual,
                    config_dir: None,
                });
            }

            let mut versions = installs
                .into_iter()
                .map(|install| VersionView {
                    tabs: lang.tabs(&install),
                    install,
                })
                .collect::<Vec<_>>();
            // Newest first — string compare works for our semver-ish
            // labels (8.4 > 8.3 lexicographically); good enough until
            // a project ships >9.x.
            versions.sort_by(|a, b| b.install.version.cmp(&a.install.version));
            LanguageView {
                id: id.into(),
                display_name: lang.display_name().into(),
                install_hint: lang.install_hint().into(),
                default_version: defaults.get(id).cloned(),
                versions,
            }
        })
        .collect()
}

// -----------------------------------------------------------------------
// Shared helpers — used by multiple language detectors below.
// -----------------------------------------------------------------------

/// Run `<binary> <arg>` and return the first whitespace-separated
/// token that looks like a semver. Used by Node / Python / Go / Ruby
/// detectors that all conform to "X.Y.Z" output.
pub fn version_from(bin: &std::path::Path, arg: &str) -> Option<String> {
    let out = std::process::Command::new(bin).arg(arg).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let combined = if text.trim().is_empty() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        text
    };
    for token in combined.split(|c: char| c.is_whitespace() || c == 'v') {
        let cleaned = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        if cleaned.split('.').count() >= 2
            && cleaned
                .split('.')
                .next()
                .map(|s| s.parse::<u32>().is_ok())
                .unwrap_or(false)
        {
            return Some(cleaned.to_string());
        }
    }
    None
}

/// Truncate a "1.22.3" → "1.22" so the sidebar groups by major.minor.
/// Keeps the full version available in the binary's actual install.
pub fn major_minor(version: &str) -> String {
    let parts: Vec<&str> = version.splitn(3, '.').collect();
    match parts.as_slice() {
        [a, b, ..] => format!("{a}.{b}"),
        _ => version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn major_minor_truncates_three_part_versions() {
        assert_eq!(major_minor("1.22.3"), "1.22");
        assert_eq!(major_minor("3.12.7"), "3.12");
        assert_eq!(major_minor("22"), "22");
        assert_eq!(major_minor("8.3"), "8.3");
    }

    #[test]
    fn source_label_covers_every_variant() {
        for s in [
            InstallSource::Homebrew,
            InstallSource::Asdf,
            InstallSource::Mise,
            InstallSource::Nvm,
            InstallSource::Pyenv,
            InstallSource::System,
        ] {
            assert!(!source_label(s).is_empty());
        }
    }

    #[test]
    fn list_all_returns_one_view_per_registered_language() {
        let views = list_all(&[], &std::collections::BTreeMap::new());
        let ids: Vec<&str> = views.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"php"));
        assert!(ids.contains(&"node"));
        assert!(ids.contains(&"python"));
        assert!(ids.contains(&"go"));
        assert!(ids.contains(&"ruby"));
    }

    #[test]
    fn empty_versions_still_surface_install_hint() {
        // Detection on a fresh machine returns empty lists; the
        // install_hint must still be present so the UI can prompt
        // the user to install via brew.
        let runtime = node::NodeRuntime;
        let lang = LanguageView {
            id: runtime.id().into(),
            display_name: runtime.display_name().into(),
            versions: vec![],
            install_hint: runtime.install_hint().into(),
            default_version: None,
        };
        assert!(!lang.install_hint.is_empty());
        let _ = PathBuf::from(""); // suppress unused import lint
    }

    #[test]
    fn default_version_is_surfaced_from_settings() {
        let mut defaults = std::collections::BTreeMap::new();
        defaults.insert("php".to_string(), "8.3".to_string());
        let views = list_all(&[], &defaults);
        let php = views.iter().find(|v| v.id == "php").unwrap();
        assert_eq!(php.default_version.as_deref(), Some("8.3"));
    }

    #[test]
    fn manual_install_is_merged_into_its_language() {
        // A manual binary the detector wouldn't surface (version 99.9) must
        // appear under its language with the Manual source.
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("php");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        let manual = vec![crate::registry::ManualRuntime {
            lang: "php".into(),
            version: "99.9".into(),
            binary: bin,
        }];
        let views = list_all(&manual, &std::collections::BTreeMap::new());
        let php = views.iter().find(|v| v.id == "php").unwrap();
        assert!(php.versions.iter().any(|v| v.install.version == "99.9"
            && matches!(v.install.source, InstallSource::Manual)));
    }
}
