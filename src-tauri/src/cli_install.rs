//! Install the bundled `portbay` CLI onto the user's PATH.
//!
//! A packaged PortBay.app ships the CLI at `Contents/MacOS/portbay`, right next
//! to the GUI binary (`portbay-app`) — but nothing puts it on `$PATH`, so a
//! fresh install can't run `portbay` from a terminal. This module symlinks the
//! bundled binary into a PATH directory (default `/usr/local/bin/portbay`,
//! VS Code's "Install 'code' command" model).
//!
//! A symlink (not a copy) means app updates are picked up automatically, and
//! creating one never reads the target, so it sidesteps the TCC file-access
//! walls that force the privileged-helper install to stage under `/private/tmp`.
//!
//! We try the write unprivileged first — on Homebrew machines the admin user
//! already owns `/usr/local/bin`, so there's no password prompt. Only when the
//! direct write fails (clean machine where `/usr/local/bin` is missing or root
//! owned) do we fall back to a single `osascript … with administrator
//! privileges` prompt.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};

/// What the Advanced settings CLI row renders.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    /// The bundled CLI exists and we can resolve where it lives.
    pub bundle_found: bool,
    /// Absolute path to the bundled `portbay` binary, when found.
    pub bundle_path: Option<String>,
    /// A `portbay` is installed at `install_path` (symlink or file present).
    pub installed: bool,
    /// The installed entry resolves to *this* app's bundled binary (i.e. not a
    /// stale link to an old/dev build).
    pub points_to_bundle: bool,
    /// The install directory is on the current `$PATH` (so `portbay` is
    /// actually runnable). False means we should tell the user to add it.
    pub on_path: bool,
}

/// Resolve the bundled CLI: the `portbay` binary sitting beside the running
/// executable. Works for a packaged app (`Contents/MacOS/portbay`) and for the
/// dev build (`target/debug/portbay`).
pub fn bundled_cli_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("portbay");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

/// Is `dir` listed in the process `$PATH`?
fn dir_on_path(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|entry| entry == dir))
        .unwrap_or(false)
}

/// Report the current state of the CLI install at `install_path`.
pub fn status(install_path: &Path) -> CliStatus {
    let bundle = bundled_cli_path();
    // `canonicalize` follows the symlink to its real target; compare against the
    // canonical bundle path so a correct link reads as `points_to_bundle`.
    let installed = install_path.symlink_metadata().is_ok();
    let points_to_bundle = match (install_path.canonicalize(), bundle.as_deref()) {
        (Ok(resolved), Some(b)) => b.canonicalize().map(|cb| cb == resolved).unwrap_or(false),
        _ => false,
    };
    let on_path = install_path.parent().map(dir_on_path).unwrap_or(false);

    CliStatus {
        bundle_found: bundle.is_some(),
        bundle_path: bundle.map(|p| p.to_string_lossy().into_owned()),
        installed,
        points_to_bundle,
        on_path,
    }
}

/// Single-quote a string for safe interpolation into a `/bin/sh` script.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Escape a string for embedding inside an AppleScript double-quoted literal.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', r#"\""#)
}

/// Reject install targets that aren't a plain absolute path to a file named
/// `portbay`, so we never hand something surprising to an elevated shell.
fn validate_target(install_path: &Path) -> Result<(), String> {
    if !install_path.is_absolute() {
        return Err("The install path must be absolute.".into());
    }
    if install_path.file_name().and_then(|n| n.to_str()) != Some("portbay") {
        return Err("The install path must end in `/portbay`.".into());
    }
    if install_path.parent().is_none() {
        return Err("The install path has no parent directory.".into());
    }
    Ok(())
}

/// Create (or refresh) the symlink at `install_path` pointing at the bundled
/// CLI. Tries unprivileged first; escalates with one OS auth prompt if the
/// directory isn't writable. Returns the resolved bundle path on success.
#[cfg(target_os = "macos")]
pub fn install(install_path: &Path) -> Result<String, String> {
    validate_target(install_path)?;
    let src = bundled_cli_path()
        .ok_or_else(|| "Couldn't find the bundled portbay binary next to the app.".to_string())?;
    let parent = install_path.parent().expect("validated has parent");

    // Fast path: write it ourselves. On Homebrew machines /usr/local/bin is
    // already owned by the admin user, so this succeeds with no prompt.
    if try_symlink_unprivileged(&src, install_path).is_ok() {
        return Ok(src.to_string_lossy().into_owned());
    }

    // Slow path: one elevated shell does mkdir + relink atomically.
    let script = format!(
        "#!/bin/sh\nset -e\n/bin/mkdir -p {parent}\n/bin/rm -f {dst}\n/bin/ln -s {src} {dst}\n",
        parent = sh_quote(&parent.to_string_lossy()),
        dst = sh_quote(&install_path.to_string_lossy()),
        src = sh_quote(&src.to_string_lossy()),
    );
    run_elevated(
        &script,
        "PortBay needs administrator access to install the portbay command-line tool.",
    )?;

    // Confirm the link actually resolves to our binary before claiming success.
    let st = status(install_path);
    if st.points_to_bundle {
        Ok(src.to_string_lossy().into_owned())
    } else {
        Err("The command-line tool didn't install correctly.".into())
    }
}

/// Remove the installed symlink at `install_path`. Unprivileged first, then
/// elevated if needed.
#[cfg(target_os = "macos")]
pub fn uninstall(install_path: &Path) -> Result<(), String> {
    validate_target(install_path)?;
    if install_path.symlink_metadata().is_err() {
        return Ok(()); // already gone — idempotent
    }
    if std::fs::remove_file(install_path).is_ok() {
        return Ok(());
    }
    let script = format!(
        "#!/bin/sh\nset -e\n/bin/rm -f {dst}\n",
        dst = sh_quote(&install_path.to_string_lossy()),
    );
    run_elevated(
        &script,
        "PortBay needs administrator access to remove the portbay command-line tool.",
    )?;
    if install_path.symlink_metadata().is_ok() {
        return Err("Couldn't remove the command-line tool.".into());
    }
    Ok(())
}

/// Try to (re)create the symlink without elevation. Replaces any existing entry
/// at the destination.
#[cfg(target_os = "macos")]
fn try_symlink_unprivileged(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::symlink;
    let parent = dst.parent().expect("validated has parent");
    if !parent.exists() {
        std::fs::create_dir_all(parent)?;
    }
    // Replace whatever's there (stale symlink, old copy) so reinstall is clean.
    if dst.symlink_metadata().is_ok() {
        std::fs::remove_file(dst)?;
    }
    symlink(src, dst)
}

/// Run a `/bin/sh` script as root via a single macOS authorization prompt.
#[cfg(target_os = "macos")]
fn run_elevated(script: &str, prompt: &str) -> Result<(), String> {
    use std::process::Command;

    // Stage the script under /private/tmp (system-local, outside TCC walls) in a
    // 0700 dir so no other local user can swap what root runs.
    let work = stage_dir()?;
    let script_path = work.join("install-cli.sh");
    std::fs::write(&script_path, script).map_err(|e| {
        let _ = std::fs::remove_dir_all(&work);
        format!("Couldn't stage the install script: {e}")
    })?;

    let apple = format!(
        r#"do shell script "/bin/sh {}" with prompt "{}" with administrator privileges"#,
        applescript_escape(&script_path.to_string_lossy()),
        applescript_escape(prompt),
    );
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&apple)
        .output();
    let _ = std::fs::remove_dir_all(&work);

    let output = output.map_err(|e| format!("Couldn't run the authorization prompt: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("(-128)") || stderr.contains("User canceled") {
        return Err("Cancelled — the authorization prompt was dismissed.".into());
    }
    Err(format!("Install failed: {}", stderr.trim()))
}

/// A 0700 working dir under /private/tmp owned by the current user.
#[cfg(target_os = "macos")]
fn stage_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    // PID + a monotonic-ish suffix keeps it unique without needing rand.
    let pid = std::process::id();
    let dir = PathBuf::from(format!("/private/tmp/portbay-cli-install.{pid}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(&dir)
        .map_err(|e| format!("Couldn't create a staging directory: {e}"))?;
    Ok(dir)
}

#[cfg(not(target_os = "macos"))]
pub fn install(_install_path: &Path) -> Result<String, String> {
    Err("Installing the CLI is only supported on macOS.".into())
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall(_install_path: &Path) -> Result<(), String> {
    Err("Installing the CLI is only supported on macOS.".into())
}
