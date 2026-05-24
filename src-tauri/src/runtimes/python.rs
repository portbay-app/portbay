//! Python runtime detector.
//!
//! Same discovery model as node.rs — every source is queried via
//! `runtimes::env`. No hardcoded prefixes or version lists.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::registry::RuntimeSettings;
use crate::runtimes::env;
use crate::runtimes::{
    version_from, ApplyResult, ConfigTab, InstallSource, KvRow, LanguageRuntime, RuntimeInstall,
};

pub struct PythonRuntime;

impl LanguageRuntime for PythonRuntime {
    fn id(&self) -> &'static str {
        "python"
    }
    fn display_name(&self) -> &'static str {
        "Python"
    }
    fn install_hint(&self) -> &'static str {
        "brew install python"
    }

    fn tabs(&self, _install: &RuntimeInstall, _settings: &RuntimeSettings) -> Vec<ConfigTab> {
        // Python config is system-owned: pip's user-level pip.conf `[global]`
        // section. We edit it directly (shared across versions — pip has no
        // per-interpreter user config).
        let path_hint = pip_conf_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(unknown)".into());

        let tab = ConfigTab::editable(
            "index",
            "Package index",
            vec![
                KvRow::text(
                    "index-url",
                    "Index URL",
                    read_pip_index_url().unwrap_or_default(),
                )
                .with_hint(
                    "pip's package index (pip.conf [global] index-url). Blank \
                     uses the default PyPI (https://pypi.org/simple).",
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
        let value = validate_pip_patch(tab_id, patches)?;
        write_pip_index_url(value.as_deref())?;
        Ok(ApplyResult::default()) // Python has no daemon.
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for (_, dir) in env::brew_formulae_matching("python") {
            if let Some(bin) = best_python_binary(&dir.join("bin")) {
                push(&mut out, &mut seen, bin, InstallSource::Homebrew);
            }
        }

        if let Some(pyenv) = env::pyenv_root() {
            scan_children(
                &pyenv.join("versions"),
                |dir| best_python_binary(&dir.join("bin")),
                &mut out,
                &mut seen,
                InstallSource::Pyenv,
            );
        }
        if let Some(asdf) = env::asdf_root() {
            scan_children(
                &asdf.join("installs").join("python"),
                |dir| best_python_binary(&dir.join("bin")),
                &mut out,
                &mut seen,
                InstallSource::Asdf,
            );
        }
        if let Some(mise) = env::mise_installs_root() {
            scan_children(
                &mise.join("python"),
                |dir| best_python_binary(&dir.join("bin")),
                &mut out,
                &mut seen,
                InstallSource::Mise,
            );
        }

        for cmd in ["python3", "python"] {
            if let Ok(p) = which::which(cmd) {
                push(&mut out, &mut seen, p, InstallSource::System);
            }
        }

        out
    }
}

/// Pick the most specific python binary in a `bin/` directory.
/// Prefers `python3.X` > `python3` > `python` — the more specific
/// symlink is less likely to drift across upgrades.
fn best_python_binary(dir: &std::path::Path) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    let mut versioned: Option<PathBuf> = None;
    let mut three: Option<PathBuf> = None;
    let mut plain: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("python3.") && versioned.is_none() {
            versioned = Some(entry.path());
        } else if name == "python3" && three.is_none() {
            three = Some(entry.path());
        } else if name == "python" && plain.is_none() {
            plain = Some(entry.path());
        }
    }
    versioned.or(three).or(plain)
}

/// Walk every direct child of `root`, calling `pick` to produce the
/// binary path for that child. Pushes if the result exists.
fn scan_children<F>(
    root: &std::path::Path,
    pick: F,
    out: &mut Vec<RuntimeInstall>,
    seen: &mut HashSet<PathBuf>,
    source: InstallSource,
) where
    F: Fn(&std::path::Path) -> Option<PathBuf>,
{
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if let Some(bin) = pick(&entry.path()) {
            push(out, seen, bin, source);
        }
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
// pip.conf — pip's user-level config. Unlike the flat key=value files, this is
// sectioned INI, so `index-url` is set under `[global]` with a small
// section-aware writer that preserves every other section/key.
// ---------------------------------------------------------------------------

fn pip_conf_path() -> Option<PathBuf> {
    // On macOS dirs::config_dir() is ~/Library/Application Support (which pip
    // reads when that dir exists); on Linux it's ~/.config. Both are valid pip
    // user-config locations.
    dirs::config_dir().map(|c| c.join("pip").join("pip.conf"))
}

fn read_pip_index_url() -> Option<String> {
    let path = pip_conf_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let mut section = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            continue;
        }
        if section == "global" {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == "index-url" {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

fn validate_pip_patch(
    tab_id: &str,
    patches: &BTreeMap<String, String>,
) -> Result<Option<String>, String> {
    if tab_id != "index" {
        return Err(format!("Python has no editable tab `{tab_id}`"));
    }
    let mut value: Option<String> = None;
    for (key, raw) in patches {
        if key != "index-url" {
            return Err(format!("unknown Python setting `{key}`"));
        }
        let val = raw.trim();
        if val.is_empty() {
            value = None;
        } else if !(val.starts_with("http://") || val.starts_with("https://")) {
            return Err("index URL must be an http(s) URL".into());
        } else {
            value = Some(val.to_string());
        }
    }
    Ok(value)
}

/// Set (or clear) `index-url` under the `[global]` section of pip.conf text,
/// preserving every other section and key. `None` removes the key.
fn apply_pip_global_index(existing: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut wrote = false;
    let mut seen_global = false;

    for line in existing.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            // Leaving a section — if it was [global] and we still owe an
            // insert (key was absent), add it before the next header.
            if section == "global" && !wrote {
                if let Some(v) = value {
                    out.push(format!("index-url = {v}"));
                }
                wrote = true;
            }
            section = t[1..t.len() - 1].trim().to_string();
            if section == "global" {
                seen_global = true;
            }
            out.push(line.to_string());
            continue;
        }
        if section == "global"
            && t.split_once('=')
                .map(|(k, _)| k.trim() == "index-url")
                .unwrap_or(false)
        {
            // First match: emit the new value (or drop when removing); skip
            // any further duplicate lines.
            if !wrote {
                if let Some(v) = value {
                    out.push(format!("index-url = {v}"));
                }
                wrote = true;
            }
            continue;
        }
        out.push(line.to_string());
    }

    // EOF still inside [global] with the key absent → insert now.
    if section == "global" && !wrote {
        if let Some(v) = value {
            out.push(format!("index-url = {v}"));
            wrote = true;
        }
    }
    // No [global] at all → append the section when setting.
    if !seen_global && !wrote {
        if let Some(v) = value {
            out.push("[global]".to_string());
            out.push(format!("index-url = {v}"));
        }
    }

    let mut body = out.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    body
}

fn write_pip_index_url(value: Option<&str>) -> Result<(), String> {
    let path = pip_conf_path().ok_or("could not resolve pip config path")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("couldn't create {}: {e}", parent.display()))?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let body = apply_pip_global_index(&existing, value);
    std::fs::write(&path, body).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn sets_index_url_in_existing_global_and_preserves_other_sections() {
        let existing = "[global]\ntimeout = 60\n\n[install]\nuser = true\n";
        let out = apply_pip_global_index(existing, Some("https://mirror/simple"));
        assert!(out.contains("timeout = 60")); // sibling key kept
        assert!(out.contains("[install]")); // other section kept
        assert!(out.contains("user = true"));
        assert!(out.contains("index-url = https://mirror/simple"));
    }

    #[test]
    fn replaces_existing_index_url_once() {
        let existing = "[global]\nindex-url = https://old/simple\n";
        let out = apply_pip_global_index(existing, Some("https://new/simple"));
        assert_eq!(out.matches("index-url").count(), 1);
        assert!(out.contains("https://new/simple"));
        assert!(!out.contains("old"));
    }

    #[test]
    fn appends_global_section_when_absent() {
        let out = apply_pip_global_index("", Some("https://mirror/simple"));
        assert!(out.contains("[global]"));
        assert!(out.contains("index-url = https://mirror/simple"));
    }

    #[test]
    fn removes_index_url_when_clearing() {
        let existing = "[global]\nindex-url = https://x/simple\ntimeout = 5\n";
        let out = apply_pip_global_index(existing, None);
        assert!(!out.contains("index-url"));
        assert!(out.contains("timeout = 5"));
    }

    #[test]
    fn validate_rejects_unknown_tab_key_and_scheme() {
        assert!(validate_pip_patch("nope", &patch(&[("index-url", "https://x/")])).is_err());
        assert!(validate_pip_patch("index", &patch(&[("cache", "/x")])).is_err());
        assert!(validate_pip_patch("index", &patch(&[("index-url", "ftp://x/")])).is_err());
        assert_eq!(
            validate_pip_patch("index", &patch(&[("index-url", "  ")])).unwrap(),
            None
        );
    }
}
