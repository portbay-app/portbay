//! Ruby runtime detector.
//!
//! Discovery via `runtimes::env` — no hardcoded paths or versions.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::registry::RuntimeSettings;
use crate::runtimes::env;
use crate::runtimes::{
    version_from, ApplyResult, ConfigTab, InstallSource, KvRow, LanguageRuntime, RuntimeInstall,
};

pub struct RubyRuntime;

impl LanguageRuntime for RubyRuntime {
    fn id(&self) -> &'static str {
        "ruby"
    }
    fn display_name(&self) -> &'static str {
        "Ruby"
    }
    fn install_hint(&self) -> &'static str {
        "brew install ruby"
    }

    fn tabs(&self, _install: &RuntimeInstall, _settings: &RuntimeSettings) -> Vec<ConfigTab> {
        // Ruby config is system-owned: `~/.gemrc` (a YAML doc). We edit only the
        // top-level scalar `gem:` key (default flags applied to every gem
        // command); the `:sources:` list is left untouched (editing it safely
        // needs list semantics — deferred to `gem sources`).
        let gemrc = read_gemrc();
        let path_hint = gemrc_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "~/.gemrc".into());

        let tab = ConfigTab::editable(
            "config",
            "RubyGems",
            vec![
                KvRow::text(
                    "gem",
                    "Default gem flags",
                    gemrc.get("gem").cloned().unwrap_or_default(),
                )
                .with_hint(
                    "Flags applied to every `gem` command via ~/.gemrc (e.g. \
                     --no-document). Blank removes the override.",
                ),
                KvRow::path("Config file", path_hint),
            ],
        );
        vec![tab]
    }

    fn apply_config(
        &self,
        _version: &str,
        tab_id: &str,
        patches: &BTreeMap<String, String>,
        _settings: &mut RuntimeSettings,
    ) -> Result<ApplyResult, String> {
        let updates = validate_gemrc_patch(tab_id, patches)?;
        let refs: Vec<(&str, Option<String>)> =
            updates.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        write_gemrc(&refs)?;
        Ok(ApplyResult::default()) // Ruby has no daemon.
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for (_, dir) in env::brew_formulae_matching("ruby") {
            push(
                &mut out,
                &mut seen,
                dir.join("bin").join("ruby"),
                InstallSource::Homebrew,
            );
        }

        if let Some(rbenv) = env::rbenv_root() {
            scan_children(
                &rbenv.join("versions"),
                "bin/ruby",
                &mut out,
                &mut seen,
                InstallSource::System,
            );
        }
        if let Some(asdf) = env::asdf_root() {
            scan_children(
                &asdf.join("installs").join("ruby"),
                "bin/ruby",
                &mut out,
                &mut seen,
                InstallSource::Asdf,
            );
        }
        if let Some(mise) = env::mise_installs_root() {
            scan_children(
                &mise.join("ruby"),
                "bin/ruby",
                &mut out,
                &mut seen,
                InstallSource::Mise,
            );
        }

        if let Ok(p) = which::which("ruby") {
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
// ~/.gemrc — a YAML doc. We surgically edit only the top-level scalar `gem:`
// key (default gem-command flags); the `:sources:` list and any other keys are
// preserved verbatim by the shared flat writer (which only matches the `gem`
// key before the first `:`).
// ---------------------------------------------------------------------------

fn gemrc_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gemrc"))
}

fn read_gemrc() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(path) = gemrc_path() else {
        return out;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return out;
    };
    for line in text.lines() {
        let t = line.trim();
        // Only flat top-level scalars; skip comments, the `:sources:` list, and
        // its `- item` entries.
        if t.is_empty() || t.starts_with('#') || t.starts_with('-') || t.starts_with(':') {
            continue;
        }
        if let Some((k, v)) = t.split_once(':') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn validate_gemrc_patch(
    tab_id: &str,
    patches: &BTreeMap<String, String>,
) -> Result<Vec<(String, Option<String>)>, String> {
    if tab_id != "config" {
        return Err(format!("Ruby has no editable tab `{tab_id}`"));
    }
    let mut out = Vec::new();
    for (key, raw) in patches {
        if key != "gem" {
            return Err(format!("unknown Ruby setting `{key}`"));
        }
        let val = raw.trim();
        if val.is_empty() {
            out.push((key.clone(), None));
            continue;
        }
        if val.contains(['\n', '\r']) {
            return Err(format!("`{key}` contains an illegal character"));
        }
        out.push((key.clone(), Some(val.to_string())));
    }
    Ok(out)
}

fn write_gemrc(updates: &[(&str, Option<String>)]) -> Result<(), String> {
    let path = gemrc_path().ok_or("could not resolve home directory")?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let body = crate::runtimes::apply_flat_config(&existing, ':', ": ", updates);
    std::fs::write(&path, body).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn editing_gem_key_preserves_sources_list() {
        // The shared flat writer with ':' sep must touch only the `gem` scalar,
        // never the `:sources:` block.
        let existing = ":sources:\n- https://rubygems.org\ngem: --old\n";
        let out = crate::runtimes::apply_flat_config(
            existing,
            ':',
            ": ",
            &[("gem", Some("--no-document".into()))],
        );
        assert!(out.contains(":sources:"));
        assert!(out.contains("- https://rubygems.org"));
        assert!(out.contains("gem: --no-document"));
        assert!(!out.contains("--old"));
    }

    #[test]
    fn validate_rejects_unknown_tab_and_key() {
        assert!(validate_gemrc_patch("nope", &patch(&[("gem", "--no-document")])).is_err());
        assert!(validate_gemrc_patch("config", &patch(&[("sources", "x")])).is_err());
    }

    #[test]
    fn validate_blank_clears_gem_flags() {
        let updates = validate_gemrc_patch("config", &patch(&[("gem", "  ")])).unwrap();
        assert_eq!(updates, vec![("gem".to_string(), None)]);
    }
}
