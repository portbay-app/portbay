//! Ruby runtime detector.
//!
//! Probes:
//!   1. Homebrew `ruby@<ver>` formula
//!   2. Homebrew bare `ruby` formula
//!   3. rbenv — `~/.rbenv/versions/<ver>/bin/ruby`
//!   4. asdf — `~/.asdf/installs/ruby/<ver>/bin/ruby`
//!   5. mise — `~/.local/share/mise/installs/ruby/<ver>/bin/ruby`
//!   6. System `ruby`

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::{
    homebrew_prefixes, version_from, InstallSource, LanguageRuntime, RuntimeInstall,
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

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for prefix in homebrew_prefixes() {
            if let Ok(entries) = std::fs::read_dir(&prefix) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let s = name.to_string_lossy();
                    if !s.starts_with("ruby@") {
                        continue;
                    }
                    push(
                        &mut out,
                        &mut seen,
                        entry.path().join("bin").join("ruby"),
                        InstallSource::Homebrew,
                    );
                }
            }
            push(
                &mut out,
                &mut seen,
                prefix.join("ruby").join("bin").join("ruby"),
                InstallSource::Homebrew,
            );
        }

        if let Some(home) = dirs::home_dir() {
            for (manager, src) in [
                (home.join(".rbenv").join("versions"), InstallSource::System),
                (home.join(".asdf").join("installs").join("ruby"), InstallSource::Asdf),
                (
                    home.join(".local")
                        .join("share")
                        .join("mise")
                        .join("installs")
                        .join("ruby"),
                    InstallSource::Mise,
                ),
            ] {
                if let Ok(entries) = std::fs::read_dir(&manager) {
                    for entry in entries.flatten() {
                        push(
                            &mut out,
                            &mut seen,
                            entry.path().join("bin").join("ruby"),
                            src,
                        );
                    }
                }
            }
        }

        if let Ok(p) = which::which("ruby") {
            push(&mut out, &mut seen, p, InstallSource::System);
        }

        out
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
