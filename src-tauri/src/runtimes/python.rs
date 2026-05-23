//! Python runtime detector.
//!
//! Same discovery model as node.rs — every source is queried via
//! `runtimes::env`. No hardcoded prefixes or version lists.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::env;
use crate::runtimes::{version_from, InstallSource, LanguageRuntime, RuntimeInstall};

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
