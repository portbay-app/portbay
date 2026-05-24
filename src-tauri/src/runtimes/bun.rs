//! Bun runtime detector.
//!
//! Detect-first, never bundled — same model as every other runtime. Bun is
//! both a JS runtime and a package manager, shipped as a single binary, so
//! discovery is a flat scan of the places a `bun` binary lands:
//!
//!   1. Every `bun` / `bun@<ver>` formula under the user's brew prefix
//!      (`env::brew_formulae_matching`).
//!   2. `~/.bun/bin/bun` — the official `bun.sh/install` location
//!      (`$BUN_INSTALL` honoured via `env::bun_root`).
//!   3. Every install under `<asdf-root>/installs/bun/<ver>`.
//!   4. Every install under `<mise-installs>/bun/<ver>`.
//!   5. Anything else on `PATH` via `which::which` — this also covers
//!      proto/version-manager shims, which resolve through `PATH`.
//!
//! Detection-only for now: the `~/.bunfig.toml` registry tab is a deferred
//! follow-up (see the kanban card). With no override, `tabs()` falls back to
//! the honest Binary + Source info pane — no fake config fields.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::env;
use crate::runtimes::{version_from, InstallSource, LanguageRuntime, RuntimeInstall};

pub struct BunRuntime;

impl LanguageRuntime for BunRuntime {
    fn id(&self) -> &'static str {
        "bun"
    }
    fn display_name(&self) -> &'static str {
        "Bun"
    }
    fn install_hint(&self) -> &'static str {
        "brew install oven-sh/bun/bun"
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for (_, dir) in env::brew_formulae_matching("bun") {
            push(
                &mut out,
                &mut seen,
                dir.join("bin").join("bun"),
                InstallSource::Homebrew,
            );
        }

        // The official installer drops a single global bun under `~/.bun`.
        if let Some(bun) = env::bun_root() {
            push(
                &mut out,
                &mut seen,
                bun.join("bin").join("bun"),
                InstallSource::System,
            );
        }

        if let Some(asdf) = env::asdf_root() {
            scan_children(
                &asdf.join("installs").join("bun"),
                "bin/bun",
                &mut out,
                &mut seen,
                InstallSource::Asdf,
            );
        }
        if let Some(mise) = env::mise_installs_root() {
            scan_children(
                &mise.join("bun"),
                "bin/bun",
                &mut out,
                &mut seen,
                InstallSource::Mise,
            );
        }

        // Anything on the user's (login-shell-expanded) PATH — also catches a
        // proto/version-manager shim, which is always on PATH when active.
        if let Ok(p) = which::which("bun") {
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
    // `bun --version` prints a bare `1.x.y` (no `v` prefix); version_from copes.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Write an executable stub that prints `version` to stdout, mimicking
    /// `bun --version` (bare `1.x.y`). Lets the fixture exercise the real
    /// `version_from` probe without a real Bun install.
    fn write_fake_bun(path: &std::path::Path, version: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("#!/bin/sh\necho {version}\n")).unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
    }

    #[test]
    fn scan_children_finds_versioned_bun_and_reads_its_version() {
        let tmp = tempfile::tempdir().unwrap();
        let installs = tmp.path().join("installs").join("bun");
        write_fake_bun(&installs.join("1.1.30").join("bin").join("bun"), "1.1.30");
        // A version dir missing the binary must be skipped, not error out.
        fs::create_dir_all(installs.join("broken")).unwrap();

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        scan_children(
            &installs,
            "bin/bun",
            &mut out,
            &mut seen,
            InstallSource::Asdf,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].version, "1.1.30");
        assert!(matches!(out[0].source, InstallSource::Asdf));
    }

    #[test]
    fn push_dedupes_same_binary_by_canonical_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin").join("bun");
        write_fake_bun(&bin, "1.2.0");

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        push(&mut out, &mut seen, bin.clone(), InstallSource::System);
        push(&mut out, &mut seen, bin, InstallSource::Homebrew); // same path → ignored

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].version, "1.2.0");
    }

    #[test]
    fn scan_of_missing_root_is_empty_not_error() {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        scan_children(
            std::path::Path::new("/no/such/bun/root"),
            "bin/bun",
            &mut out,
            &mut seen,
            InstallSource::Mise,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn bun_runtime_identity_and_hint() {
        let r = BunRuntime;
        assert_eq!(r.id(), "bun");
        assert_eq!(r.display_name(), "Bun");
        assert!(!r.install_hint().is_empty());
    }
}
