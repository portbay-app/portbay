//! Node.js runtime detector.
//!
//! Probes (in order):
//!   1. Homebrew `node@<ver>` formula (`/opt/homebrew/opt/node@22/bin/node`)
//!   2. Homebrew bare `node` formula
//!   3. nvm — `~/.nvm/versions/node/<ver>/bin/node`
//!   4. asdf — `~/.asdf/installs/nodejs/<ver>/bin/node`
//!   5. mise — `~/.local/share/mise/installs/node/<ver>/bin/node`
//!   6. System `which node`
//!
//! Deduped by major.minor so the sidebar groups Node 22.11 and 22.12
//! into a single "22" row in a follow-up. For now we surface each
//! exact version detected.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::{
    homebrew_prefixes, version_from, InstallSource, LanguageRuntime, RuntimeInstall,
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
    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        // Homebrew versioned formulas (node@18, node@20, node@22, …).
        for prefix in homebrew_prefixes() {
            if let Ok(entries) = std::fs::read_dir(&prefix) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let s = name.to_string_lossy();
                    if !s.starts_with("node@") {
                        continue;
                    }
                    let bin = entry.path().join("bin").join("node");
                    push_if_present(&mut out, &mut seen, bin, InstallSource::Homebrew);
                }
            }
            let bare = prefix.join("node").join("bin").join("node");
            push_if_present(&mut out, &mut seen, bare, InstallSource::Homebrew);
        }

        // nvm — versions live under ~/.nvm/versions/node/v<ver>/bin/node
        if let Some(home) = dirs::home_dir() {
            let nvm = home.join(".nvm").join("versions").join("node");
            if let Ok(entries) = std::fs::read_dir(&nvm) {
                for entry in entries.flatten() {
                    let bin = entry.path().join("bin").join("node");
                    push_if_present(&mut out, &mut seen, bin, InstallSource::Nvm);
                }
            }
            // asdf
            let asdf = home.join(".asdf").join("installs").join("nodejs");
            scan_manager_installs(&asdf, "bin/node", &mut out, &mut seen, InstallSource::Asdf);
            // mise
            let mise = home
                .join(".local")
                .join("share")
                .join("mise")
                .join("installs")
                .join("node");
            scan_manager_installs(&mise, "bin/node", &mut out, &mut seen, InstallSource::Mise);
        }

        // System PATH (last so it loses dedup ties to package-manager hits).
        if let Ok(p) = which::which("node") {
            push_if_present(&mut out, &mut seen, p, InstallSource::System);
        }

        out
    }
}

/// Walk `<root>/<version>/<rel>` for every direct child of `root`,
/// pushing the resulting binary path when it exists.
fn scan_manager_installs(
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
        let bin = entry.path().join(rel);
        push_if_present(out, seen, bin, source);
    }
}

fn push_if_present(
    out: &mut Vec<RuntimeInstall>,
    seen: &mut HashSet<PathBuf>,
    bin: PathBuf,
    source: InstallSource,
) {
    if !bin.exists() {
        return;
    }
    // Canonicalise so symlinks (Homebrew + nvm both symlink) don't
    // produce two entries for the same install.
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
