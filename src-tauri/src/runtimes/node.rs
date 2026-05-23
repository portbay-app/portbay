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

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::env;
use crate::runtimes::{version_from, InstallSource, LanguageRuntime, RuntimeInstall};

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
            scan_children(&versions_dir, "bin/node", &mut out, &mut seen, InstallSource::Nvm);
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
