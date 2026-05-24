//! Node.js runtime detector.
//!
//! Discovery is entirely driven by the user's actual environment —
//! no hardcoded version lists, no hardcoded prefixes. Sources:
//!
//!   1. Every `node` / `node@<ver>` formula under the user's brew prefix
//!      (discovered via `brew --prefix`, see `env::brew_opt_prefixes`).
//!   2. `~/.nvm/versions/node/<ver>/bin/node` (NVM_DIR honoured).
//!   3. Every install under `<asdf-root>/installs/nodejs/<ver>`.
//!   4. Every install under `<mise-installs>/node/<ver>`.
//!   5. Anything else on `PATH` via `which::which`.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::registry::RuntimeSettings;
use crate::runtimes::env;
use crate::runtimes::{
    version_from, ApplyResult, ConfigTab, InstallSource, KvRow, LanguageRuntime, RuntimeInstall,
};

pub struct NodeRuntime;

impl LanguageRuntime for NodeRuntime {
    fn id(&self) -> &'static str {
        "node"
    }
    fn display_name(&self) -> &'static str {
        "Node.js"
    }
    fn install_hint(&self) -> &'static str {
        "brew install node"
    }

    fn tabs(&self, _install: &RuntimeInstall, _settings: &RuntimeSettings) -> Vec<ConfigTab> {
        // Node config is system-owned, not PortBay-owned: these tabs read and
        // write the user-level `~/.npmrc` directly (shared across every Node
        // version — npm has no per-version user config). No registry storage.
        let npmrc = read_npmrc();
        let path_hint = npmrc_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "~/.npmrc".into());

        let registry = ConfigTab::editable(
            "registry",
            "Registry",
            vec![
                KvRow::text(
                    "registry",
                    "Registry URL",
                    npmrc.get("registry").cloned().unwrap_or_default(),
                )
                .with_hint(
                    "Read by npm and pnpm from ~/.npmrc (user-level, shared across \
                     Node versions). Leave blank for the default registry.npmjs.org.",
                ),
                KvRow::path("Config file", path_hint),
            ],
        );

        let cache = ConfigTab::editable(
            "cache",
            "Cache",
            vec![KvRow::text(
                "cache",
                "Cache directory",
                npmrc.get("cache").cloned().unwrap_or_default(),
            )
            .with_hint(
                "npm cache location (~/.npmrc `cache`). Blank uses npm's default (~/.npm).",
            )],
        );

        vec![registry, cache]
    }

    fn apply_config(
        &self,
        _version: &str,
        tab_id: &str,
        patches: &BTreeMap<String, String>,
        _settings: &mut RuntimeSettings,
    ) -> Result<ApplyResult, String> {
        let updates = validate_npmrc_patch(tab_id, patches)?;
        let refs: Vec<(&str, Option<String>)> = updates
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        write_npmrc_keys(&refs)?;
        // Node has no daemon — nothing to restart; the next process that reads
        // ~/.npmrc picks the change up.
        Ok(ApplyResult::default())
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for (_, dir) in env::brew_formulae_matching("node") {
            push(
                &mut out,
                &mut seen,
                dir.join("bin").join("node"),
                InstallSource::Homebrew,
            );
        }

        if let Some(nvm) = env::nvm_root() {
            let versions_dir = nvm.join("versions").join("node");
            scan_children(
                &versions_dir,
                "bin/node",
                &mut out,
                &mut seen,
                InstallSource::Nvm,
            );
        }
        if let Some(asdf) = env::asdf_root() {
            scan_children(
                &asdf.join("installs").join("nodejs"),
                "bin/node",
                &mut out,
                &mut seen,
                InstallSource::Asdf,
            );
        }
        if let Some(mise) = env::mise_installs_root() {
            scan_children(
                &mise.join("node"),
                "bin/node",
                &mut out,
                &mut seen,
                InstallSource::Mise,
            );
        }

        // Anything on the user's (now login-shell-expanded) PATH.
        if let Ok(p) = which::which("node") {
            push(&mut out, &mut seen, p, InstallSource::System);
        }
        out
    }
}

fn scan_children(
    root: &std::path::Path,
    rel: &str,
    out: &mut Vec<RuntimeInstall>,
    seen: &mut HashSet<PathBuf>,
    source: InstallSource,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        push(out, seen, entry.path().join(rel), source);
    }
}

fn push(
    out: &mut Vec<RuntimeInstall>,
    seen: &mut HashSet<PathBuf>,
    bin: PathBuf,
    source: InstallSource,
) {
    if !bin.exists() {
        return;
    }
    let canonical = bin.canonicalize().unwrap_or_else(|_| bin.clone());
    if !seen.insert(canonical) {
        return;
    }
    let Some(version) = version_from(&bin, "--version") else {
        return;
    };
    out.push(RuntimeInstall {
        version,
        binary: bin,
        source,
        config_dir: None,
    });
}

// ---------------------------------------------------------------------------
// ~/.npmrc — read for the Registry/Cache tabs, written by apply_config.
//
// The file may hold auth tokens and settings PortBay doesn't surface, so edits
// are surgical: only the targeted keys are rewritten, every other line is kept
// verbatim. The validation + text transform are pure (and unit-tested); only
// `write_npmrc_keys` touches the filesystem.
// ---------------------------------------------------------------------------

fn npmrc_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".npmrc"))
}

/// Parse `~/.npmrc` into a flat key→value map for display. Comments, sections,
/// and lines without an `=` are ignored.
fn read_npmrc() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(path) = npmrc_path() else {
        return out;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return out;
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with(';') || t.starts_with('#') || t.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

/// Validate an editable-tab patch into `(key, Some(value)|None)` updates.
/// `None` clears the key. Rejects unknown tabs/keys and malformed values so a
/// buggy frontend can't write garbage into `~/.npmrc`.
fn validate_npmrc_patch(
    tab_id: &str,
    patches: &BTreeMap<String, String>,
) -> Result<Vec<(String, Option<String>)>, String> {
    let allowed: &[&str] = match tab_id {
        "registry" => &["registry"],
        "cache" => &["cache"],
        other => return Err(format!("Node has no editable tab `{other}`")),
    };
    let mut out = Vec::new();
    for (key, raw) in patches {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown Node setting `{key}`"));
        }
        let val = raw.trim();
        if val.is_empty() {
            out.push((key.clone(), None));
            continue;
        }
        if val.contains(['\n', '\r']) {
            return Err(format!("`{key}` contains an illegal character"));
        }
        if key == "registry" && !(val.starts_with("http://") || val.starts_with("https://")) {
            return Err("registry must be an http(s) URL".into());
        }
        out.push((key.clone(), Some(val.to_string())));
    }
    Ok(out)
}

/// Apply key updates to npmrc text (flat `key=value`), preserving every
/// unrelated line. Thin wrapper over the shared [`crate::runtimes`] helper.
fn apply_npmrc_text(existing: &str, updates: &[(&str, Option<String>)]) -> String {
    crate::runtimes::apply_flat_config(existing, '=', "=", updates)
}

fn write_npmrc_keys(updates: &[(&str, Option<String>)]) -> Result<(), String> {
    let path = npmrc_path().ok_or("could not resolve home directory")?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let body = apply_npmrc_text(&existing, updates);
    std::fs::write(&path, body).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn apply_npmrc_text_replaces_in_place_and_preserves_other_lines() {
        let existing = "; my npmrc\n//registry.npmjs.org/:_authToken=secret\nregistry=https://old.example/\nsave-exact=true\n";
        let out = apply_npmrc_text(
            existing,
            &[("registry", Some("https://new.example/".to_string()))],
        );
        // Auth token + unrelated setting + comment survive.
        assert!(out.contains("_authToken=secret"));
        assert!(out.contains("save-exact=true"));
        assert!(out.contains("; my npmrc"));
        // Old registry gone, new one present, exactly once.
        assert!(!out.contains("old.example"));
        assert_eq!(out.matches("registry=").count(), 1);
        assert!(out.contains("registry=https://new.example/"));
    }

    #[test]
    fn apply_npmrc_text_removes_key_when_value_is_none() {
        let existing = "registry=https://x/\ncache=/tmp/c\n";
        let out = apply_npmrc_text(existing, &[("cache", None)]);
        assert!(!out.contains("cache="));
        assert!(out.contains("registry=https://x/"));
    }

    #[test]
    fn apply_npmrc_text_collapses_duplicate_keys() {
        let existing = "registry=a\nregistry=b\n";
        let out = apply_npmrc_text(existing, &[("registry", Some("https://c/".to_string()))]);
        assert_eq!(out.matches("registry=").count(), 1);
        assert!(out.contains("registry=https://c/"));
    }

    #[test]
    fn validate_rejects_unknown_tab_key_and_bad_registry() {
        assert!(validate_npmrc_patch("nope", &patch(&[("registry", "https://x/")])).is_err());
        assert!(validate_npmrc_patch("registry", &patch(&[("cache", "/tmp")])).is_err());
        assert!(validate_npmrc_patch("registry", &patch(&[("registry", "ftp://x/")])).is_err());
    }

    #[test]
    fn validate_blank_value_clears_key() {
        let updates = validate_npmrc_patch("cache", &patch(&[("cache", "  ")])).unwrap();
        assert_eq!(updates, vec![("cache".to_string(), None)]);
    }

    #[test]
    fn validate_accepts_https_registry() {
        let updates =
            validate_npmrc_patch("registry", &patch(&[("registry", "https://r.example/")]))
                .unwrap();
        assert_eq!(
            updates,
            vec![(
                "registry".to_string(),
                Some("https://r.example/".to_string())
            )]
        );
    }
}
