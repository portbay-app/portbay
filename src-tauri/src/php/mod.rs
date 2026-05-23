//! PHP version detection + per-version metadata.
//!
//! v1 scope: detect what's already installed on the user's machine
//! (Homebrew formulae, standard prefixes) and surface the needed paths
//! for the reconciler to wire each version into Process Compose. We do
//! not bundle a compiler; missing versions are surfaced with a
//! `brew install php@x.y` hint that the GUI renders directly.
//!
//! Detection sources, in priority order:
//!   1. Homebrew Apple Silicon: `/opt/homebrew/opt/php@<ver>/`
//!   2. Homebrew Intel:         `/usr/local/opt/php@<ver>/`
//!   3. Homebrew main `php` formula (typically the current major).
//!
//! Each detected version yields a [`PhpInstall`] with the binary,
//! php-fpm path, php.ini path, and the loaded extensions parsed from
//! `php -m`. The reconciler reads the list to spawn one FPM child per
//! version that any registered project actually uses.

pub mod error;
pub mod lifecycle;

pub use error::{PhpError, Result};

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Versions we know how to detect via Homebrew. Each entry maps to a
/// `php@<ver>` formula; the bare `php` formula handles the rolling
/// latest version that Homebrew points at.
pub const KNOWN_VERSIONS: &[&str] = &["7.4", "8.0", "8.1", "8.2", "8.3", "8.4"];

/// One detected PHP install. Fully serialisable so the Tauri command
/// surface can hand it straight to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpInstall {
    /// Semantic version label, e.g. "8.3". May be a longer string
    /// for the bare `php` formula (we resolve it to the major.minor).
    pub version: String,

    /// Path to the `php` CLI binary.
    pub php_bin: PathBuf,

    /// Path to the `php-fpm` binary, when present. Pure-CLI installs
    /// can omit FPM, in which case this is `None` and PortBay can't
    /// serve sites with that version.
    pub php_fpm_bin: Option<PathBuf>,

    /// The php.ini file PHP loads by default. Parsed from
    /// `php --ini` output.
    pub php_ini: Option<PathBuf>,

    /// Directory where additional `.ini` files are loaded from.
    /// Useful for telling the user where to drop `xdebug.ini` etc.
    pub additional_ini_dir: Option<PathBuf>,

    /// Where to look for compiled extension `.so` files.
    pub extension_dir: Option<PathBuf>,

    /// Loaded extensions as reported by `php -m`. Sorted, deduped.
    pub loaded_extensions: Vec<String>,

    /// Source of the install — Homebrew formula name or "system".
    pub source: PhpSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PhpSource {
    Homebrew,
    System,
}

/// Probe the machine for installed PHPs. Returns one entry per
/// detected version. Detection is best-effort — a probe failure on
/// one path is logged and skipped, not propagated.
pub fn detect_all() -> Vec<PhpInstall> {
    let mut out: Vec<PhpInstall> = Vec::new();
    let mut seen_versions: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ver in KNOWN_VERSIONS {
        for prefix in homebrew_prefixes() {
            let candidate = prefix.join(format!("php@{ver}"));
            if !candidate.exists() {
                continue;
            }
            let bin = candidate.join("bin").join("php");
            if !bin.exists() {
                continue;
            }
            if let Some(install) = probe(&bin, ver, PhpSource::Homebrew) {
                if seen_versions.insert(install.version.clone()) {
                    out.push(install);
                }
            }
        }
    }

    // The bare `php` formula resolves to the current Homebrew major.
    // Probe it last so a versioned formula with the same major wins.
    for prefix in homebrew_prefixes() {
        let bin = prefix.join("php").join("bin").join("php");
        if !bin.exists() {
            continue;
        }
        if let Some(install) = probe(&bin, "", PhpSource::Homebrew) {
            if seen_versions.insert(install.version.clone()) {
                out.push(install);
            }
        }
    }

    // Plain `php` on PATH as a final fallback.
    if let Ok(path) = which::which("php") {
        if let Some(install) = probe(&path, "", PhpSource::System) {
            if seen_versions.insert(install.version.clone()) {
                out.push(install);
            }
        }
    }

    out.sort_by(|a, b| a.version.cmp(&b.version));
    out
}

/// Probe a specific PHP binary and return its [`PhpInstall`]. The
/// `version_hint` is the formula's nominal version (`"8.3"`) when we
/// derived this candidate from a known prefix; for the bare `php`
/// formula or a PATH probe it's `""` and we parse the version from
/// `php --version`.
fn probe(bin: &Path, version_hint: &str, source: PhpSource) -> Option<PhpInstall> {
    let version = if version_hint.is_empty() {
        version_from_bin(bin)?
    } else {
        version_hint.to_string()
    };

    let ini_out = Command::new(bin).arg("--ini").output().ok()?;
    let ini_text = String::from_utf8_lossy(&ini_out.stdout);
    let php_ini = parse_ini_path(&ini_text, "Loaded Configuration File");
    let additional_ini_dir = parse_ini_path(&ini_text, "Scan this dir for additional .ini files");

    let ext_dir_out = Command::new(bin)
        .args(["-r", "echo ini_get('extension_dir');"])
        .output()
        .ok()?;
    let extension_dir = if ext_dir_out.status.success() {
        let s = String::from_utf8_lossy(&ext_dir_out.stdout)
            .trim()
            .to_string();
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    } else {
        None
    };

    let modules_out = Command::new(bin).arg("-m").output().ok()?;
    let loaded_extensions = parse_modules(&String::from_utf8_lossy(&modules_out.stdout));

    let php_fpm_bin = locate_fpm(bin);

    Some(PhpInstall {
        version,
        php_bin: bin.to_path_buf(),
        php_fpm_bin,
        php_ini,
        additional_ini_dir,
        extension_dir,
        loaded_extensions,
        source,
    })
}

/// macOS Homebrew install prefixes — Apple Silicon and Intel both
/// exist in the wild; we probe each.
fn homebrew_prefixes() -> Vec<PathBuf> {
    let mut prefixes = Vec::new();
    let candidates = ["/opt/homebrew/opt", "/usr/local/opt"];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            prefixes.push(p);
        }
    }
    prefixes
}

/// Locate php-fpm next to a php binary. Homebrew lays it out as
/// `<prefix>/sbin/php-fpm` while some other distributions co-locate
/// it in `bin/`. We probe both.
fn locate_fpm(php_bin: &Path) -> Option<PathBuf> {
    let prefix = php_bin.parent()?.parent()?;
    let candidates = [
        prefix.join("sbin").join("php-fpm"),
        prefix.join("bin").join("php-fpm"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

fn version_from_bin(bin: &Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?;
    // Typical output: "PHP 8.3.13 (cli) (built: ...)". Pull out the
    // major.minor and return that, since our registry shape is
    // "8.3"-style (third-component drift is irrelevant for routing).
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let mut version_parts = parts[1].split('.');
    let major = version_parts.next()?;
    let minor = version_parts.next()?;
    Some(format!("{major}.{minor}"))
}

/// Pull a value out of `php --ini` / `php -i` output. Both forms
/// label their lines the same way but use different separators —
/// `php --ini` uses `:` while `php -i` uses `=>`. We accept either.
pub(crate) fn parse_ini_path(text: &str, label: &str) -> Option<PathBuf> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix(label) else {
            continue;
        };
        let sep_pos = rest.find("=>").or_else(|| rest.find(':'))?;
        let after = &rest[sep_pos..];
        let path = after
            .trim_start_matches("=>")
            .trim_start_matches(':')
            .trim();
        if path.is_empty() || path == "(none)" {
            return None;
        }
        return Some(PathBuf::from(path));
    }
    None
}

/// Parse the output of `php -m` into a sorted, deduped extension list.
/// The output has a `[PHP Modules]` header, then one extension per
/// line, then a `[Zend Modules]` section. We collapse both into one
/// list since they're functionally identical from the user's POV.
pub(crate) fn parse_modules(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('[') && !l.starts_with("Zend"))
        .map(|l| l.to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modules_strips_headers_and_dedupes() {
        let sample = "\
[PHP Modules]
Core
date
json
openssl

[Zend Modules]
Zend OPcache
opcache
";
        let mods = parse_modules(sample);
        assert!(mods.contains(&"Core".to_string()));
        assert!(mods.contains(&"json".to_string()));
        assert!(mods.contains(&"opcache".to_string()));
        // headers stripped
        assert!(!mods.iter().any(|m| m.starts_with('[')));
        // Zend OPcache line filtered, opcache deduped survives
        assert!(!mods.iter().any(|m| m.contains("Zend OPcache")));
    }

    #[test]
    fn parse_ini_path_handles_none_and_present() {
        let text = "\
Configuration File (php.ini) Path: /usr/local/etc/php/8.3
Loaded Configuration File:         /usr/local/etc/php/8.3/php.ini
Scan this dir for additional .ini files: /usr/local/etc/php/8.3/conf.d
Additional .ini files parsed:      (none)
";
        assert_eq!(
            parse_ini_path(text, "Loaded Configuration File").unwrap(),
            PathBuf::from("/usr/local/etc/php/8.3/php.ini")
        );
        assert_eq!(
            parse_ini_path(text, "Scan this dir for additional .ini files").unwrap(),
            PathBuf::from("/usr/local/etc/php/8.3/conf.d")
        );
        assert!(parse_ini_path(text, "Missing label").is_none());
    }

    #[test]
    fn known_versions_lists_at_least_one_recent_release() {
        // Sanity — Homebrew currently ships 8.3 as the latest stable
        // tap. If this fails after a Homebrew bump, expand
        // KNOWN_VERSIONS first.
        assert!(KNOWN_VERSIONS.contains(&"8.3"));
    }
}
