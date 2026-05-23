//! Go runtime detector.
//!
//! Discovery via `runtimes::env` — no hardcoded paths or versions.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::env;
use crate::runtimes::{version_from, InstallSource, LanguageRuntime, RuntimeInstall};

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
    fn probe_version(&self, binary: &std::path::Path) -> Option<String> {
        // `go --version` is not valid; Go reports via `go version`.
        version_from(binary, "version")
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for (_, dir) in env::brew_formulae_matching("go") {
            push(
                &mut out,
                &mut seen,
                dir.join("bin").join("go"),
                InstallSource::Homebrew,
            );
        }

        if let Some(asdf) = env::asdf_root() {
            // asdf-golang installs land under <root>/installs/golang/<ver>/go/bin/go
            let golang = asdf.join("installs").join("golang");
            if let Ok(entries) = std::fs::read_dir(&golang) {
                for entry in entries.flatten() {
                    push(
                        &mut out,
                        &mut seen,
                        entry.path().join("go").join("bin").join("go"),
                        InstallSource::Asdf,
                    );
                }
            }
        }
        if let Some(mise) = env::mise_installs_root() {
            let go_dir = mise.join("go");
            if let Ok(entries) = std::fs::read_dir(&go_dir) {
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
