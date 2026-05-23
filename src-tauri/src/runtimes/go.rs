//! Go runtime detector.
//!
//! Probes:
//!   1. Homebrew `go@<ver>` formula
//!   2. Homebrew bare `go` formula
//!   3. asdf — `~/.asdf/installs/golang/<ver>/go/bin/go`
//!   4. mise — `~/.local/share/mise/installs/go/<ver>/bin/go`
//!   5. System `go`

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::{
    homebrew_prefixes, version_from, InstallSource, LanguageRuntime, RuntimeInstall,
};

pub struct GoRuntime;

impl LanguageRuntime for GoRuntime {
    fn id(&self) -> &'static str {
        "go"
    }
    fn display_name(&self) -> &'static str {
        "Go"
    }
    fn install_hint(&self) -> &'static str {
        "brew install go"
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for prefix in homebrew_prefixes() {
            if let Ok(entries) = std::fs::read_dir(&prefix) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let s = name.to_string_lossy();
                    if !s.starts_with("go@") {
                        continue;
                    }
                    push(
                        &mut out,
                        &mut seen,
                        entry.path().join("bin").join("go"),
                        InstallSource::Homebrew,
                    );
                }
            }
            push(
                &mut out,
                &mut seen,
                prefix.join("go").join("bin").join("go"),
                InstallSource::Homebrew,
            );
        }

        if let Some(home) = dirs::home_dir() {
            let asdf = home.join(".asdf").join("installs").join("golang");
            if let Ok(entries) = std::fs::read_dir(&asdf) {
                for entry in entries.flatten() {
                    push(
                        &mut out,
                        &mut seen,
                        entry.path().join("go").join("bin").join("go"),
                        InstallSource::Asdf,
                    );
                }
            }
            let mise = home
                .join(".local")
                .join("share")
                .join("mise")
                .join("installs")
                .join("go");
            if let Ok(entries) = std::fs::read_dir(&mise) {
                for entry in entries.flatten() {
                    push(
                        &mut out,
                        &mut seen,
                        entry.path().join("bin").join("go"),
                        InstallSource::Mise,
                    );
                }
            }
        }

        if let Ok(p) = which::which("go") {
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
    let Some(version) = version_from(&bin, "version") else {
        return;
    };
    out.push(RuntimeInstall {
        version,
        binary: bin,
        source,
        config_dir: None,
    });
}
