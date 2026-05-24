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

pub mod bun;
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
    /// Key-value rows shown in this tab.
    pub rows: Vec<KvRow>,
    /// When true the tab has at least one editable row; the frontend shows
    /// a Save button that posts the dirty rows to `update_runtime_config`.
    #[serde(default)]
    pub editable: bool,
}

impl ConfigTab {
    /// A read-only info tab (no Save button). Used for metadata panes.
    pub fn readonly(id: impl Into<String>, label: impl Into<String>, rows: Vec<KvRow>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            rows,
            editable: false,
        }
    }

    /// An editable tab — its rows carry input field kinds and the frontend
    /// renders a Save button.
    pub fn editable(id: impl Into<String>, label: impl Into<String>, rows: Vec<KvRow>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            rows,
            editable: true,
        }
    }
}

/// How a [`KvRow`] renders and whether it accepts edits. Read-only rows use
/// [`FieldKind::Readonly`] (value + copy/reveal affordances, the historical
/// behaviour); the rest render as the matching input control and are sent
/// back on save keyed by [`KvRow::key`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldKind {
    /// Display only — value shown with copy/reveal, never edited.
    Readonly,
    /// Single-line free text.
    Text,
    /// Numeric input. Optional bounds clamp the stepper in the UI.
    #[serde(rename_all = "camelCase")]
    Number {
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// One-of a fixed option list (rendered as a `<select>`).
    Select { options: Vec<String> },
    /// Boolean toggle. `value` is `"true"` / `"false"`.
    Bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KvRow {
    /// Stable key edits are posted under. For read-only info rows this is a
    /// label slug and is ignored on save.
    pub key: String,
    pub label: String,
    pub value: String,
    /// Optional hint shown beneath the value (e.g. install path, doc link).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// When true, the value is rendered as a monospace path the user
    /// can click to reveal in Finder.
    #[serde(default)]
    pub is_path: bool,
    /// How this row renders + whether it's editable.
    pub field: FieldKind,
}

impl KvRow {
    fn slug(label: &str) -> String {
        label
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// Read-only metadata row (the default for info panes).
    pub fn info(label: impl Into<String>, value: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            key: Self::slug(&label),
            label,
            value: value.into(),
            hint: None,
            is_path: false,
            field: FieldKind::Readonly,
        }
    }

    /// Read-only row whose value is a filesystem path (gets a reveal button).
    pub fn path(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            is_path: true,
            ..Self::info(label, value)
        }
    }

    /// Editable free-text row.
    pub fn text(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
            hint: None,
            is_path: false,
            field: FieldKind::Text,
        }
    }

    /// Editable numeric row with optional bounds.
    pub fn number(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl std::fmt::Display,
        min: Option<i64>,
        max: Option<i64>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.to_string(),
            hint: None,
            is_path: false,
            field: FieldKind::Number { min, max },
        }
    }

    /// Editable single-choice row.
    pub fn select(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        options: Vec<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
            hint: None,
            is_path: false,
            field: FieldKind::Select { options },
        }
    }

    /// Attach a hint shown beneath the field (builder-style).
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// What applying a config patch implies for the running stack. Returned by
/// [`LanguageRuntime::apply_config`] so the IPC layer can restart only the
/// services the change actually affects.
#[derive(Debug, Clone, Default)]
pub struct ApplyResult {
    /// Process-compose process ids to restart so the change takes effect now
    /// (e.g. the version's FPM pool). Best-effort — the caller ignores
    /// restarts for processes that aren't currently running.
    pub restart_processes: Vec<String>,
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
///
/// `Send + Sync` because the boxed trait object is carried across `.await`
/// points in the async IPC commands (every impl is a stateless unit struct).
pub trait LanguageRuntime: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn install_hint(&self) -> &'static str;
    /// The Homebrew formula to `brew install` for this runtime's recommended
    /// version, or `None` if PortBay can't drive a brew install for it. The
    /// default derives it from `install_hint()` (everything after
    /// `"brew install "`), so the install action and the sidebar hint can't
    /// drift; a runtime whose hint isn't a `brew install …` line returns
    /// `None` and simply won't offer the one-click install.
    fn brew_formula(&self) -> Option<String> {
        self.install_hint()
            .strip_prefix("brew install ")
            .map(|f| f.trim().to_string())
    }
    /// Detect every install on this machine.
    fn detect(&self) -> Vec<RuntimeInstall>;
    /// Probe an arbitrary binary's version string, for the "add by path"
    /// flow. Default runs `<binary> --version`; runtimes whose flag differs
    /// (Go uses `version`) override this.
    fn probe_version(&self, binary: &std::path::Path) -> Option<String> {
        version_from(binary, "--version")
    }
    /// Per-version config tabs. Default: a single read-only "Info" tab that
    /// shows the binary path and source. PHP overrides this with editable
    /// FPM / php.ini / extensions tabs, reading saved values from `settings`.
    fn tabs(
        &self,
        install: &RuntimeInstall,
        _settings: &crate::registry::RuntimeSettings,
    ) -> Vec<ConfigTab> {
        vec![ConfigTab::readonly(
            "info",
            "Info",
            vec![
                KvRow::path("Binary", install.binary.to_string_lossy().into_owned()),
                KvRow::info("Source", source_label(install.source)),
            ],
        )]
    }

    /// Apply a patch from an editable tab, persisting into `settings`. Returns
    /// the services that must restart for the change to take effect. The
    /// default has no editable settings and rejects any patch — only runtimes
    /// that expose editable tabs override this.
    fn apply_config(
        &self,
        _version: &str,
        _tab_id: &str,
        _patches: &std::collections::BTreeMap<String, String>,
        _settings: &mut crate::registry::RuntimeSettings,
    ) -> Result<ApplyResult, String> {
        Err(format!("{} has no editable settings", self.display_name()))
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
        Box::new(bun::BunRuntime),
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
/// Per-version `tabs` are pre-computed (reading any saved per-version config
/// from `settings`) so the frontend renders the whole panel without an extra
/// round-trip.
///
/// A manual install whose binary the detector already surfaced is skipped (no
/// duplicate row).
pub fn list_all(settings: &crate::registry::RuntimeSettings) -> Vec<LanguageView> {
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
            for m in settings.manual.iter().filter(|m| m.lang == id) {
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
                    tabs: lang.tabs(&install, settings),
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
                default_version: settings.defaults.get(id).cloned(),
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

/// Surgically apply `key → value` updates to a flat, line-based config file's
/// text (e.g. `~/.npmrc`, Go's `env`, `~/.gemrc`). For each update every
/// existing line whose key (the text before the first `sep`) matches is
/// dropped, then a single canonical `key{joiner}value` line is appended when
/// setting (`None` removes the key entirely). Every unrelated line — comments,
/// other sections, settings PortBay doesn't surface — is preserved verbatim.
///
/// Shared by the system-owned runtime config tabs (Node/Go/Ruby) so each one
/// reuses the same well-tested, non-destructive write.
pub(crate) fn apply_flat_config(
    existing: &str,
    sep: char,
    joiner: &str,
    updates: &[(&str, Option<String>)],
) -> String {
    let mut lines: Vec<String> = existing.lines().map(|s| s.to_string()).collect();
    for (key, value) in updates {
        lines.retain(|line| {
            line.trim_start()
                .split_once(sep)
                .map(|(k, _)| k.trim() != *key)
                .unwrap_or(true)
        });
        if let Some(v) = value {
            lines.push(format!("{key}{joiner}{v}"));
        }
    }
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    body
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
        let views = list_all(&crate::registry::RuntimeSettings::default());
        let ids: Vec<&str> = views.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"php"));
        assert!(ids.contains(&"node"));
        assert!(ids.contains(&"bun"));
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
        let mut settings = crate::registry::RuntimeSettings::default();
        settings
            .defaults
            .insert("php".to_string(), "8.3".to_string());
        let views = list_all(&settings);
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
        let settings = crate::registry::RuntimeSettings {
            manual: vec![crate::registry::ManualRuntime {
                lang: "php".into(),
                version: "99.9".into(),
                binary: bin,
            }],
            ..Default::default()
        };
        let views = list_all(&settings);
        let php = views.iter().find(|v| v.id == "php").unwrap();
        assert!(php
            .versions
            .iter()
            .any(|v| v.install.version == "99.9"
                && matches!(v.install.source, InstallSource::Manual)));
    }

    #[test]
    fn default_runtime_has_no_editable_settings() {
        // Every shipped runtime now overrides apply_config, so exercise the
        // default impl through a minimal stand-in: it must reject any patch.
        struct NoConfig;
        impl LanguageRuntime for NoConfig {
            fn id(&self) -> &'static str {
                "noconfig"
            }
            fn display_name(&self) -> &'static str {
                "NoConfig"
            }
            fn install_hint(&self) -> &'static str {
                ""
            }
            fn detect(&self) -> Vec<RuntimeInstall> {
                Vec::new()
            }
        }
        let mut settings = crate::registry::RuntimeSettings::default();
        let err = NoConfig
            .apply_config(
                "1",
                "any",
                &std::collections::BTreeMap::new(),
                &mut settings,
            )
            .unwrap_err();
        assert!(err.contains("no editable settings"));
    }

    #[test]
    fn apply_flat_config_preserves_other_lines_and_dedupes() {
        let existing = "# comment\nGOPATH=/old\nGOFLAGS=-mod=vendor\nGOPATH=/dup\n";
        let out = apply_flat_config(
            existing,
            '=',
            "=",
            &[("GOPROXY", Some("https://proxy/".into())), ("GOPATH", None)],
        );
        assert!(out.contains("# comment"));
        assert!(out.contains("GOFLAGS=-mod=vendor")); // value with '=' survives
        assert!(out.contains("GOPROXY=https://proxy/"));
        assert!(!out.contains("GOPATH=")); // removed (both occurrences)
    }
}
