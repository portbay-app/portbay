//! Flutter runtime detector.
//!
//! Detect-first, never bundled. Flutter can arrive from Homebrew, asdf, mise,
//! FVM shims, or any other manager that exposes `flutter` on PATH. PortBay
//! treats the SDK binary as the runtime pin used by Flutter mobile projects.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::runtimes::env;
use crate::runtimes::{version_from, InstallSource, LanguageRuntime, RuntimeInstall};

pub struct FlutterRuntime;

impl LanguageRuntime for FlutterRuntime {
    fn id(&self) -> &'static str {
        "flutter"
    }
    fn display_name(&self) -> &'static str {
        "Flutter"
    }
    fn install_hint(&self) -> &'static str {
        "brew install --cask flutter"
    }
    fn brew_formula(&self) -> Option<String> {
        Some("--cask flutter".into())
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for (_, dir) in env::brew_formulae_matching("flutter") {
            push(
                &mut out,
                &mut seen,
                dir.join("bin").join("flutter"),
                InstallSource::Homebrew,
            );
        }

        if let Some(asdf) = env::asdf_root() {
            scan_children(
                &asdf.join("installs").join("flutter"),
                "bin/flutter",
                &mut out,
                &mut seen,
                InstallSource::Asdf,
            );
        }
        if let Some(mise) = env::mise_installs_root() {
            scan_children(
                &mise.join("flutter"),
                "bin/flutter",
                &mut out,
                &mut seen,
                InstallSource::Mise,
            );
        }

        if let Ok(p) = which::which("flutter") {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn write_fake_flutter(path: &std::path::Path, version: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("#!/bin/sh\necho Flutter {version}\n")).unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
    }

    #[test]
    fn scan_children_finds_flutter_sdk_version() {
        let tmp = tempfile::tempdir().unwrap();
        let installs = tmp.path().join("installs").join("flutter");
        write_fake_flutter(
            &installs.join("3.24.5").join("bin").join("flutter"),
            "3.24.5",
        );

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        scan_children(
            &installs,
            "bin/flutter",
            &mut out,
            &mut seen,
            InstallSource::Mise,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].version, "3.24.5");
        assert!(matches!(out[0].source, InstallSource::Mise));
    }

    #[test]
    fn flutter_runtime_identity_and_hint() {
        let r = FlutterRuntime;
        assert_eq!(r.id(), "flutter");
        assert_eq!(r.display_name(), "Flutter");
        assert!(r.install_hint().contains("flutter"));
        assert_eq!(r.brew_formula().as_deref(), Some("--cask flutter"));
    }
}
