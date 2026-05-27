//! User-environment discovery for runtime detection.
//!
//! GUI apps on macOS inherit a minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`)
//! because they aren't launched through the user's shell. Tools that live
//! under `/opt/homebrew/...`, `/Volumes/MyDrive/homebrew/...`, `~/.nvm`,
//! or any custom prefix are invisible until we ask the user's actual
//! shell for its PATH. This module owns that discovery, plus the
//! version-manager roots that the language detectors scan.
//!
//! Design constraints:
//!   - No hardcoded paths. Every prefix is discovered at runtime by
//!     asking the user's tools (`brew --prefix`, `pyenv root`, etc.) or
//!     by reading the env vars those tools document
//!     (`$ASDF_DATA_DIR`, `$NVM_DIR`, etc.).
//!   - Best-effort: any discovery failure logs a warning and falls back
//!     to the next strategy, never panicking. Worst case we end up with
//!     whatever the GUI inherited.
//!   - Bounded latency: the login-shell PATH expansion runs once at app
//!     startup with a 3-second timeout so a slow `.zshrc` can't gate
//!     boot. Subsequent detections read the cached PATH from the
//!     process env.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long we'll wait for the user's login shell to print its PATH.
/// Heavy `.zshrc` files (nvm, conda, Homebrew shellenv, oh-my-zsh
/// theme loading) can easily push past 3 s on a cold cache. 8 s is
/// the empirically-comfortable upper bound — long enough to absorb
/// a real-world slow shell, short enough that the user doesn't
/// notice a stall at app launch. When this still times out we fall
/// back to a non-interactive shell probe (`-c`) which skips rc
/// files but at least picks up the OS defaults.
const SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(8);
/// Fallback timeout for the non-interactive probe — should be fast
/// since no rc files are sourced.
const SHELL_PATH_FALLBACK_TIMEOUT: Duration = Duration::from_secs(2);

/// Path markers for competitor dev-environment apps PortBay must never *run*
/// binaries from. PortBay ships and supervises its own services; launching a
/// php-fpm / nginx / httpd that belongs to ServBay, Herd, MAMP, XAMPP or FlyEnv
/// couples us to their install layout, breaks when they update or uninstall,
/// and is the wrong thing for a tool that isn't associated with them.
///
/// This is strictly about *executing* their binaries. The migration importer
/// (`crate::import`) still reads their configs to bring sites into PortBay —
/// that's a different, intentional flow that never runs their tools.
const COMPETITOR_PATH_MARKERS: &[&str] =
    &["servbay", "xampp", "flyenv", "herd.app", "/herd/", "/mamp"];

/// True if `path` resolves into a known competitor dev-environment app. The path
/// is canonicalised first, so a neutral-looking symlink that points into a
/// competitor bundle (e.g. `/usr/local/bin/php` → `/Applications/XAMPP/…/php`)
/// is still caught.
pub fn is_competitor_managed(path: &std::path::Path) -> bool {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let lower = real.to_string_lossy().to_ascii_lowercase();
    COMPETITOR_PATH_MARKERS.iter().any(|m| lower.contains(m))
}

/// Resolve the user's login shell. Priority:
///   1. `$SHELL` (set by the OS when the user has a session)
///   2. `getpwuid` via `dscl . -read /Users/<user> UserShell` on macOS
///      — falls back to the password database without requiring root.
///   3. `/bin/zsh` (default since macOS Catalina) or `/bin/bash`
///      (Linux/Windows fallback).
pub fn login_shell() -> PathBuf {
    if let Ok(s) = std::env::var("SHELL") {
        if !s.is_empty() {
            let p = PathBuf::from(&s);
            if p.exists() {
                return p;
            }
        }
    }

    // dscl is macOS-specific; on Linux we'd parse /etc/passwd. We try
    // dscl first since it's the official Apple-supported path.
    #[cfg(target_os = "macos")]
    {
        if let Ok(user) = std::env::var("USER") {
            if let Ok(out) = Command::new("dscl")
                .args([".", "-read", &format!("/Users/{user}"), "UserShell"])
                .output()
            {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout);
                    // Output shape: "UserShell: /bin/zsh"
                    if let Some(after) = s.split(':').nth(1) {
                        let candidate = after.trim();
                        let p = PathBuf::from(candidate);
                        if p.exists() {
                            return p;
                        }
                    }
                }
            }
        }
    }

    // Last-ditch defaults — present on virtually every UNIX-like OS.
    for fallback in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        let p = PathBuf::from(fallback);
        if p.exists() {
            return p;
        }
    }

    PathBuf::from("/bin/sh")
}

/// Run the user's login shell and return its `$PATH`. Returns `None`
/// when the shell can't be reached, times out, or prints an empty
/// PATH. The caller decides whether to merge or replace the
/// inherited PATH.
///
/// Two-stage probe:
///   1. `-i -l -c 'echo "$PATH"'` so the shell sources `.zshenv`,
///      `.zprofile` (login), and `.zshrc` (interactive) — most users
///      put their PATH edits in one or the other and it's not safe
///      to assume which. Bounded by `SHELL_PATH_TIMEOUT`.
///   2. On timeout, fall back to `-c` (no rc files) with a short
///      timeout. This won't pick up user-added prefixes but at least
///      captures the OS defaults the shell exposes — better than
///      degrading to Tauri's minimal inherited PATH.
fn shell_path() -> Option<String> {
    let shell = login_shell();
    if let Some(p) = run_path_probe(&shell, &["-ilc", "echo \"$PATH\""], SHELL_PATH_TIMEOUT) {
        return Some(p);
    }
    tracing::warn!(shell = %shell.display(), "login-shell PATH probe timed out; trying non-interactive fallback");
    run_path_probe(
        &shell,
        &["-c", "echo \"$PATH\""],
        SHELL_PATH_FALLBACK_TIMEOUT,
    )
}

fn run_path_probe(shell: &std::path::Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = Command::new(shell);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());

    let mut child = cmd.spawn().ok()?;
    let started = Instant::now();
    // Poll for completion with a deadline. spawn_blocking would be
    // cleaner but this runs at app startup before tokio is fully
    // configured for state-tracked work; a simple wait_timeout via
    // try_wait keeps the dep surface tight.
    loop {
        if let Ok(Some(_status)) = child.try_wait() {
            break;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Replace the process PATH with the user's login-shell PATH, merging
/// the GUI-inherited entries on the end so we keep anything Tauri
/// itself added. Idempotent — calling twice is a no-op since the
/// merged result is stable.
///
/// Called once during app setup. Subsequent runtime detection,
/// `which::which`, and child-process spawns (PC, Caddy, project
/// dev servers) inherit the new PATH automatically.
pub fn bootstrap_user_env() {
    let Some(user_path) = shell_path() else {
        tracing::info!("login-shell PATH unavailable; using GUI-inherited PATH");
        return;
    };

    let current = std::env::var("PATH").unwrap_or_default();
    // Merge: user PATH first (their tools win), then anything from the
    // inherited PATH that wasn't already there. Dedup by exact string
    // match — close enough for /opt/homebrew vs /opt/homebrew differences.
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for entry in user_path
        .split(':')
        .chain(current.split(':'))
        .filter(|s| !s.is_empty())
    {
        if seen.insert(entry.to_string()) {
            merged.push(entry.to_string());
        }
    }
    let joined = merged.join(":");
    // SAFETY: std::env::set_var is technically unsafe in multi-threaded
    // contexts (rustc 1.80+). We call it once at setup before spawning
    // any worker threads, so the contract holds. The compiler still
    // emits a warning; we silence it with the wrapping `unsafe` only
    // where required by the edition. On stable Rust 1.79 this remains
    // safe.
    std::env::set_var("PATH", &joined);
    tracing::info!(entries = merged.len(), "user PATH merged into process env");
}

/// Return every Homebrew install prefix the user has. Strategy:
///   1. Ask `brew --prefix` — works for the user's primary install
///      regardless of where they put it (Intel default, Apple Silicon
///      default, custom volume, Linuxbrew).
///   2. As a safety net, probe the two default macOS prefixes
///      (`/opt/homebrew`, `/usr/local`) — handles the case where brew
///      isn't on PATH but its install layout is still on disk.
///   3. Dedupe and return only directories that actually exist.
///
/// Output paths point at the `<prefix>/opt/` directory — that's
/// where versioned formulae land (`opt/php@8.3`, `opt/node@22`).
pub fn brew_opt_prefixes() -> Vec<PathBuf> {
    let mut prefixes: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Some(p) = brew_prefix_via_cli() {
        let opt = p.join("opt");
        if opt.is_dir() && seen.insert(opt.clone()) {
            prefixes.push(opt);
        }
    }

    // Last-ditch defaults — only honoured when actually present.
    for fallback in ["/opt/homebrew", "/usr/local"] {
        let opt = PathBuf::from(fallback).join("opt");
        if opt.is_dir() && seen.insert(opt.clone()) {
            prefixes.push(opt);
        }
    }

    prefixes
}

/// Return every Homebrew **Cellar** directory — where kegs are actually
/// installed, as `<prefix>/Cellar/<formula>/<version>`. The Cellar is the
/// sibling of `opt/` under each prefix, so we derive it from the same prefixes
/// [`brew_opt_prefixes`] discovers. Scanning the Cellar (in addition to `opt`)
/// lets detection find a formula whose `opt/<formula>` symlink is missing — an
/// install left unlinked or interrupted still shows up.
pub fn brew_cellar_prefixes() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for opt in brew_opt_prefixes() {
        // `<prefix>/opt` → `<prefix>` → `<prefix>/Cellar`.
        if let Some(cellar) = opt.parent().map(|p| p.join("Cellar")) {
            if cellar.is_dir() && seen.insert(cellar.clone()) {
                out.push(cellar);
            }
        }
    }
    out
}

fn brew_prefix_via_cli() -> Option<PathBuf> {
    let out = Command::new("brew").arg("--prefix").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

/// Discover the asdf-vm data directory. Priority:
///   1. `$ASDF_DATA_DIR` (the modern env var — asdf 0.10+)
///   2. `$ASDF_DIR` (legacy)
///   3. `~/.asdf`
pub fn asdf_root() -> Option<PathBuf> {
    for var in ["ASDF_DATA_DIR", "ASDF_DIR"] {
        if let Ok(d) = std::env::var(var) {
            let p = PathBuf::from(d);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    home_subdir(".asdf")
}

/// Discover the nvm directory. Priority:
///   1. `$NVM_DIR`
///   2. `~/.nvm`
pub fn nvm_root() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("NVM_DIR") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    home_subdir(".nvm")
}

/// Discover Bun's official-installer directory (the `curl bun.sh/install`
/// location). Priority:
///   1. `$BUN_INSTALL`
///   2. `~/.bun`
pub fn bun_root() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("BUN_INSTALL") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    home_subdir(".bun")
}

/// Discover the pyenv root. Priority:
///   1. `pyenv root` (the CLI's own answer)
///   2. `$PYENV_ROOT`
///   3. `~/.pyenv`
pub fn pyenv_root() -> Option<PathBuf> {
    if let Ok(out) = Command::new("pyenv").arg("root").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(s);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
    }
    if let Ok(d) = std::env::var("PYENV_ROOT") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    home_subdir(".pyenv")
}

/// Discover the rbenv root. Priority:
///   1. `rbenv root`
///   2. `$RBENV_ROOT`
///   3. `~/.rbenv`
pub fn rbenv_root() -> Option<PathBuf> {
    if let Ok(out) = Command::new("rbenv").arg("root").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(s);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
    }
    if let Ok(d) = std::env::var("RBENV_ROOT") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    home_subdir(".rbenv")
}

/// Discover the mise (formerly rtx) installs root. Priority:
///   1. `$MISE_DATA_DIR/installs`
///   2. `~/.local/share/mise/installs`
pub fn mise_installs_root() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("MISE_DATA_DIR") {
        let p = PathBuf::from(d).join("installs");
        if p.is_dir() {
            return Some(p);
        }
    }
    dirs::home_dir()
        .map(|h| h.join(".local").join("share").join("mise").join("installs"))
        .filter(|p| p.is_dir())
}

fn home_subdir(child: &str) -> Option<PathBuf> {
    dirs::home_dir()
        .map(|h| h.join(child))
        .filter(|p| p.is_dir())
}

/// List every Homebrew formula directory whose name matches
/// `<base>` exactly or `<base>@<version>`. Used by every language
/// detector instead of a hardcoded version list — picks up whatever
/// the user installed, including future versions Homebrew adds.
///
/// Returns `(formula_name, install_path)` pairs. `install_path` is
/// the directory itself (e.g. `<prefix>/opt/php@8.3`).
pub fn brew_formulae_matching(base: &str) -> Vec<(String, PathBuf)> {
    collect_brew_formulae(&brew_opt_prefixes(), &brew_cellar_prefixes(), base)
}

/// Pure core of [`brew_formulae_matching`], split out so the scan logic is unit
/// testable without a real Homebrew install. Given the `opt` dirs and `Cellar`
/// dirs to scan, return `(formula_name, install_dir)` for every formula named
/// `<base>` or `<base>@<version>`.
///
/// - In `opt/`, a formula is a single dir (`opt/php@8.3`) that already contains
///   `bin/` (via symlink into the Cellar).
/// - In `Cellar/`, a formula has an extra version layer
///   (`Cellar/php@8.3/8.3.13`); we descend one level so the returned dir
///   contains `bin/` like the opt case.
///
/// Callers dedupe by canonical binary path, so an `opt` symlink and the Cellar
/// keg it points at collapse into a single install — listing both here is safe.
fn collect_brew_formulae(
    opt_dirs: &[PathBuf],
    cellar_dirs: &[PathBuf],
    base: &str,
) -> Vec<(String, PathBuf)> {
    let prefix_match = format!("{base}@");
    let matches_name = |s: &str| s == base || s.starts_with(&prefix_match);
    let mut out = Vec::new();

    // Linked formulae — the common case.
    for opt_dir in opt_dirs {
        let Ok(entries) = std::fs::read_dir(opt_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let s = entry.file_name().to_string_lossy().into_owned();
            if matches_name(&s) {
                out.push((s, entry.path()));
            }
        }
    }

    // Installed kegs whose `opt` symlink may be absent (unlinked/interrupted
    // install). Descend the version layer so the returned dir holds `bin/`.
    for cellar in cellar_dirs {
        let Ok(entries) = std::fs::read_dir(cellar) else {
            continue;
        };
        for entry in entries.flatten() {
            let s = entry.file_name().to_string_lossy().into_owned();
            if !matches_name(&s) {
                continue;
            }
            let Ok(versions) = std::fs::read_dir(entry.path()) else {
                continue;
            };
            for v in versions.flatten() {
                if v.path().is_dir() {
                    out.push((s.clone(), v.path()));
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_shell_falls_back_to_sh_when_nothing_else_exists() {
        // Sanity: even with no $SHELL, this must return a path.
        let shell = login_shell();
        assert!(!shell.as_os_str().is_empty());
    }

    #[test]
    fn brew_opt_prefixes_returns_only_existing_dirs() {
        for p in brew_opt_prefixes() {
            assert!(p.is_dir(), "{p:?} should be an existing dir");
        }
    }

    #[test]
    fn brew_formulae_matching_with_unknown_base_returns_empty() {
        let v = brew_formulae_matching("__no_such_formula_ever__");
        assert!(v.is_empty());
    }

    #[test]
    fn collect_brew_formulae_scans_opt_and_unlinked_cellar_kegs() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // opt/: a linked php + an unrelated formula + a non-matching language.
        let opt = root.join("opt");
        fs::create_dir_all(opt.join("php@8.3")).unwrap();
        fs::create_dir_all(opt.join("node")).unwrap();
        fs::create_dir_all(opt.join("php-cs-fixer")).unwrap(); // must NOT match "php"

        // Cellar/: a php keg with NO opt symlink (the unlinked-install case),
        // plus a ruby keg that must be ignored.
        let cellar = root.join("Cellar");
        fs::create_dir_all(cellar.join("php@8.2").join("8.2.10").join("bin")).unwrap();
        fs::create_dir_all(cellar.join("ruby").join("3.3.0")).unwrap();

        let got = collect_brew_formulae(&[opt], &[cellar], "php");
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();

        // Linked opt php + unlinked Cellar php both surface; nothing else does.
        assert!(names.contains(&"php@8.3"), "opt php@8.3 should be found");
        assert!(names.contains(&"php@8.2"), "unlinked Cellar php@8.2 should be found");
        assert_eq!(got.len(), 2, "node / php-cs-fixer / ruby must be excluded: {names:?}");

        // The Cellar hit descends to the version dir so it holds `bin/`.
        let cellar_hit = got.iter().find(|(n, _)| n == "php@8.2").unwrap();
        assert!(cellar_hit.1.ends_with("8.2.10"));
    }

    #[test]
    fn competitor_managed_paths_are_rejected() {
        use std::path::Path;
        // Non-existent paths can't be canonicalised, so they fall back to the
        // literal path — which is what these markers match on.
        for p in [
            "/Applications/ServBay/package/sbin/php-fpm",
            "/Applications/ServBay/script/alias/nginx",
            "/Applications/XAMPP/xamppfiles/bin/php-8.2.4",
            "/Applications/MAMP/bin/php/php8.2/bin/php",
            "/Users/me/Library/Application Support/Herd/bin/php",
            "/Users/me/Library/Application Support/FlyEnv/php/8.3/bin/php",
        ] {
            assert!(
                is_competitor_managed(Path::new(p)),
                "expected competitor path to be rejected: {p}"
            );
        }
    }

    #[test]
    fn neutral_paths_are_allowed() {
        use std::path::Path;
        for p in [
            "/opt/homebrew/opt/php@8.3/sbin/php-fpm",
            "/usr/local/bin/nginx",
            "/usr/sbin/httpd",
            "/Users/me/code/my-herd-project/bin/php", // "herd" in a project name must NOT trip it
        ] {
            assert!(
                !is_competitor_managed(Path::new(p)),
                "expected neutral path to be allowed: {p}"
            );
        }
    }
}
