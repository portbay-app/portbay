//! Python runtime detector.
//!
//! Probes:
//!   1. Homebrew `python@<ver>` formulae
//!   2. Homebrew bare `python` formula
//!   3. pyenv — `~/.pyenv/versions/<ver>/bin/python3`
//!   4. asdf — `~/.asdf/installs/python/<ver>/bin/python3`
//!   5. mise — `~/.local/share/mise/installs/python/<ver>/bin/python3`
//!   6. System `python3`

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::{
    homebrew_prefixes, version_from, InstallSource, LanguageRuntime, RuntimeInstall,
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
        "brew install python@3.12"
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for prefix in homebrew_prefixes() {
            if let Ok(entries) = std::fs::read_dir(&prefix) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let s = name.to_string_lossy();
                    if !s.starts_with("python@") {
                        continue;
                    }
                    // Pick the versioned binary if present, else fall back.
                    let dir = entry.path().join("bin");
                    let candidate = best_python_binary(&dir);
                    if let Some(bin) = candidate {
                        push(&mut out, &mut seen, bin, InstallSource::Homebrew);
                    }
                }
            }
            let bare = best_python_binary(&prefix.join("python").join("bin"));
            if let Some(bin) = bare {
                push(&mut out, &mut seen, bin, InstallSource::Homebrew);
            }
        }

        if let Some(home) = dirs::home_dir() {
            let pyenv = home.join(".pyenv").join("versions");
            if let Ok(entries) = std::fs::read_dir(&pyenv) {
                for entry in entries.flatten() {
                    if let Some(bin) = best_python_binary(&entry.path().join("bin")) {
                        push(&mut out, &mut seen, bin, InstallSource::Pyenv);
                    }
                }
            }
            let asdf = home.join(".asdf").join("installs").join("python");
            if let Ok(entries) = std::fs::read_dir(&asdf) {
                for entry in entries.flatten() {
                    if let Some(bin) = best_python_binary(&entry.path().join("bin")) {
                        push(&mut out, &mut seen, bin, InstallSource::Asdf);
                    }
                }
            }
            let mise = home
                .join(".local")
                .join("share")
                .join("mise")
                .join("installs")
                .join("python");
            if let Ok(entries) = std::fs::read_dir(&mise) {
                for entry in entries.flatten() {
                    if let Some(bin) = best_python_binary(&entry.path().join("bin")) {
                        push(&mut out, &mut seen, bin, InstallSource::Mise);
                    }
                }
            }
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
/// Prefers `python3.X` > `python3` > `python` because the more
/// specific symlink is less likely to drift across upgrades.
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
