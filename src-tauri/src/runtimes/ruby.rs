//! Ruby runtime detector.
//!
//! Discovery via `runtimes::env` — no hardcoded paths or versions.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::env;
use crate::runtimes::{version_from, InstallSource, LanguageRuntime, RuntimeInstall};

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
