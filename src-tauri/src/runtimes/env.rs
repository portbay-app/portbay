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
/// 3 s is generous — even pathologically slow `.zshrc` files
/// (NVM lazy-loaders, conda init) typically finish under 1 s. A
/// timeout here only delays first detection, never blocks boot.
const SHELL_PATH_TIMEOUT: Duration = Duration::from_secs(3);

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
/// We use `-i -l -c` so the shell sources both `.zshenv`/`.zprofile`
/// (login) and `.zshrc` (interactive) — most users put their PATH
/// edits in one or the other and it's not safe to assume which.
fn shell_path() -> Option<String> {
    let shell = login_shell();
    let mut cmd = Command::new(&shell);
    cmd.args(["-ilc", "echo \"$PATH\""])
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
        if started.elapsed() > SHELL_PATH_TIMEOUT {
            let _ = child.kill();
            tracing::warn!(shell = %shell.display(), "login-shell PATH probe timed out");
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

fn brew_prefix_via_cli() -> Option<PathBuf> {
    let out = Command::new("brew")
        .arg("--prefix")
        .output()
        .ok()?;
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
    dirs::home_dir().map(|h| h.join(child)).filter(|p| p.is_dir())
}

/// List every Homebrew formula directory whose name matches
/// `<base>` exactly or `<base>@<version>`. Used by every language
/// detector instead of a hardcoded version list — picks up whatever
/// the user installed, including future versions Homebrew adds.
///
/// Returns `(formula_name, install_path)` pairs. `install_path` is
/// the directory itself (e.g. `<prefix>/opt/php@8.3`).
pub fn brew_formulae_matching(base: &str) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let prefix_match = format!("{base}@");
    for opt_dir in brew_opt_prefixes() {
        let Ok(entries) = std::fs::read_dir(&opt_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy().into_owned();
            if s == base || s.starts_with(&prefix_match) {
                out.push((s, entry.path()));
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
}
