//! Platform resolver install / uninstall / status.
//!
//! macOS routes DNS queries for a specific suffix to a configured
//! nameserver via a small file under `/etc/resolver/`. The file lives
//! at `/etc/resolver/<suffix>` and contains lines like:
//!
//! ```text
//! nameserver 127.0.0.1
//! port 53053
//! ```
//!
//! Writing into `/etc/` requires root. PortBay drives the install via
//! `osascript -e 'do shell script "…" with administrator privileges'`,
//! which surfaces the standard macOS sudo dialog. One prompt, one
//! file written; afterwards every `.<suffix>` query routes to the
//! local dnsmasq port we picked at boot.
//!
//! Linux routes DNS queries through `systemd-resolved` using a drop-in under
//! `/etc/systemd/resolved.conf.d/`. PortBay supports that as the first Linux
//! tier; other resolver managers remain documented best-effort.

use std::path::PathBuf;
use std::process::Command;

use crate::dnsmasq::error::{DnsmasqError, Result};

/// Path on disk for a given suffix. macOS uses the suffix verbatim
/// (no `.` prefix), e.g. `/etc/resolver/test`.
#[cfg(target_os = "macos")]
pub fn resolver_file_path(suffix: &str) -> PathBuf {
    PathBuf::from("/etc/resolver").join(suffix.trim_start_matches('.'))
}

#[cfg(target_os = "linux")]
pub fn resolver_file_path(suffix: &str) -> PathBuf {
    PathBuf::from("/etc/systemd/resolved.conf.d").join(format!(
        "portbay-{}.conf",
        suffix.trim_matches('.').replace('.', "-")
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn resolver_file_path(suffix: &str) -> PathBuf {
    PathBuf::from("/etc/portbay/resolver").join(suffix.trim_start_matches('.'))
}

/// Content the file should contain. Caller picks the dnsmasq port.
///
/// This legacy helper is suffix-less because macOS stores the suffix in the
/// filename and the file body only needs the loopback nameserver and port. New
/// code that needs platform-specific content should call
/// `resolver_file_content_for_suffix`.
#[cfg(target_os = "macos")]
pub fn resolver_file_content(port: u16) -> String {
    format!("nameserver 127.0.0.1\nport {port}\n")
}

#[cfg(target_os = "linux")]
pub fn resolver_file_content(port: u16) -> String {
    resolver_file_content_for_suffix("test", port)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn resolver_file_content(port: u16) -> String {
    format!("nameserver 127.0.0.1\nport {port}\n")
}

pub fn resolver_file_content_for_suffix(suffix: &str, port: u16) -> String {
    #[cfg(target_os = "linux")]
    {
        return linux_resolver_file_content(suffix, port);
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = suffix;
        resolver_file_content(port)
    }
}

#[cfg(target_os = "linux")]
pub fn linux_resolver_file_content(suffix: &str, port: u16) -> String {
    let suffix = suffix.trim_matches('.');
    format!(
        "# PortBay-managed wildcard resolver. Do not edit by hand.\n[Resolve]\nDNS=127.0.0.1:{port}\nDomains=~{suffix}\n"
    )
}

/// True iff the file exists and points at `127.0.0.1` on the given
/// port. Used by the reconciler's hosts sub-step to decide whether
/// it can skip writing `/etc/hosts` (the file does the routing).
pub fn is_installed(suffix: &str, port: u16) -> bool {
    let path = resolver_file_path(suffix);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return false;
    };
    (contents.contains("nameserver 127.0.0.1") && contents.contains(&format!("port {port}")))
        || (contents.contains(&format!("DNS=127.0.0.1:{port}")) && contents.contains("Domains=~"))
}

/// Returns the file's full contents if it exists, for diagnostic
/// display. None when the file is missing.
pub fn read_installed(suffix: &str) -> Option<String> {
    std::fs::read_to_string(resolver_file_path(suffix)).ok()
}

/// The port the installed resolver file currently points at, parsed from its
/// `port <N>` line. None if the file is missing or malformed. Used at boot to
/// prefer re-binding the SAME port, so wildcard DNS keeps working across
/// restarts even when the privileged helper isn't available to re-point the
/// file (the drift the resolver guard otherwise has to repair).
pub fn read_installed_port(suffix: &str) -> Option<u16> {
    let contents = read_installed(suffix)?;
    contents
        .lines()
        .find_map(|l| {
            let line = l.trim();
            line.strip_prefix("port ")
                .map(str::trim)
                .or_else(|| line.strip_prefix("DNS=127.0.0.1:").map(str::trim))
        })
        .and_then(|p| p.split_whitespace().next()?.parse::<u16>().ok())
}

/// Root-side install used by the privileged helper. Non-root callers should use
/// `install_via_osascript`, whose name is kept for API compatibility with the
/// earlier macOS-only implementation.
pub fn install_as_root(suffix: &str, port: u16) -> Result<()> {
    let suffix = sanitise_suffix(suffix)?;
    let path = resolver_file_path(&suffix);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DnsmasqError::SpawnFailed(e.to_string()))?;
    }
    let body = resolver_file_content_for_suffix(&suffix, port);
    std::fs::write(&path, body).map_err(|e| DnsmasqError::SpawnFailed(e.to_string()))?;
    reload_resolver_manager()
}

pub fn uninstall_as_root(suffix: &str) -> Result<()> {
    let suffix = sanitise_suffix(suffix)?;
    let path = resolver_file_path(&suffix);
    match std::fs::remove_file(&path) {
        Ok(()) => reload_resolver_manager(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(DnsmasqError::SpawnFailed(e.to_string())),
    }
}

/// Write `/etc/resolver/<suffix>` via osascript-with-admin. Blocks
/// until the user dismisses the macOS auth dialog. On cancel,
/// returns a `PermissionDenied`-equivalent error so the GUI can
/// distinguish "user said no" from "shell exec failed."
#[cfg(target_os = "macos")]
pub fn install_via_osascript(suffix: &str, port: u16) -> Result<()> {
    let suffix = sanitise_suffix(suffix)?;
    let path = resolver_file_path(&suffix);
    let body = resolver_file_content(port);

    // Build the shell command we want osascript to run as root.
    // We pre-escape every interpolated value so the user can't pass a
    // suffix containing shell metachars — even though `sanitise_suffix`
    // already restricts the alphabet, defense in depth.
    let shell_cmd = format!(
        "/bin/mkdir -p /etc/resolver && /usr/bin/printf %s {} > {}",
        shell_quote(&body),
        shell_quote(&path.to_string_lossy()),
    );

    run_osascript_admin(
        &shell_cmd,
        "Allow PortBay to route .test queries to its local DNS resolver?",
    )
}

/// Remove `/etc/resolver/<suffix>` via osascript-with-admin.
#[cfg(target_os = "macos")]
pub fn uninstall_via_osascript(suffix: &str) -> Result<()> {
    let suffix = sanitise_suffix(suffix)?;
    let path = resolver_file_path(&suffix);
    let shell_cmd = format!("/bin/rm -f {}", shell_quote(&path.to_string_lossy()));
    run_osascript_admin(&shell_cmd, "Allow PortBay to remove its DNS resolver file?")
}

#[cfg(not(target_os = "macos"))]
#[cfg(target_os = "linux")]
pub fn install_via_osascript(suffix: &str, port: u16) -> Result<()> {
    let suffix = sanitise_suffix(suffix)?;
    let path = resolver_file_path(&suffix);
    let body = resolver_file_content_for_suffix(&suffix, port);
    let parent = path
        .parent()
        .ok_or_else(|| DnsmasqError::SpawnFailed("invalid resolver path".into()))?;
    let shell_cmd = format!(
        "/usr/bin/install -d -m 755 {parent} && /usr/bin/printf %s {body} > {path} && /bin/systemctl restart systemd-resolved",
        parent = shell_quote(&parent.to_string_lossy()),
        body = shell_quote(&body),
        path = shell_quote(&path.to_string_lossy()),
    );
    run_pkexec_shell(&shell_cmd)
}

#[cfg(not(target_os = "macos"))]
#[cfg(target_os = "linux")]
pub fn uninstall_via_osascript(suffix: &str) -> Result<()> {
    let suffix = sanitise_suffix(suffix)?;
    let path = resolver_file_path(&suffix);
    let shell_cmd = format!(
        "/bin/rm -f {path} && /bin/systemctl restart systemd-resolved",
        path = shell_quote(&path.to_string_lossy()),
    );
    run_pkexec_shell(&shell_cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install_via_osascript(_suffix: &str, _port: u16) -> Result<()> {
    Err(DnsmasqError::SpawnFailed(
        "resolver install is not supported on this platform".into(),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall_via_osascript(_suffix: &str) -> Result<()> {
    Err(DnsmasqError::SpawnFailed(
        "resolver uninstall is not supported on this platform".into(),
    ))
}

#[cfg(target_os = "macos")]
fn run_osascript_admin(shell_cmd: &str, prompt: &str) -> Result<()> {
    // The `do shell script` AppleScript form runs the command via
    // `/bin/sh -c <cmd>` with admin privileges. The `with prompt`
    // string is what the user sees in the auth dialog title.
    let escaped_cmd = applescript_escape(shell_cmd);
    let escaped_prompt = applescript_escape(prompt);
    let script = format!(
        r#"do shell script "{escaped_cmd}" with prompt "{escaped_prompt}" with administrator privileges"#
    );

    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| DnsmasqError::SpawnFailed(format!("osascript: {e}")))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    // The user-cancelled-the-dialog AppleScript error is `(-128)`. Map
    // it to a recognisable variant so the GUI can stay quiet about it.
    if stderr.contains("(-128)") || stderr.contains("User canceled") {
        return Err(DnsmasqError::SpawnFailed(
            "cancelled — keychain prompt was dismissed".into(),
        ));
    }
    Err(DnsmasqError::SpawnFailed(format!(
        "osascript failed: {}",
        stderr.trim()
    )))
}

#[cfg(target_os = "linux")]
fn run_pkexec_shell(shell_cmd: &str) -> Result<()> {
    let output = Command::new("pkexec")
        .arg("/bin/sh")
        .arg("-c")
        .arg(shell_cmd)
        .output()
        .map_err(|e| DnsmasqError::SpawnFailed(format!("pkexec: {e}")))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("dismissed") || stderr.contains("Not authorized") {
        return Err(DnsmasqError::SpawnFailed(
            "cancelled — authorization dialog was dismissed".into(),
        ));
    }
    Err(DnsmasqError::SpawnFailed(format!(
        "pkexec failed: {}",
        stderr.trim()
    )))
}

fn reload_resolver_manager() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("systemctl")
            .args(["restart", "systemd-resolved"])
            .status()
            .map_err(|e| DnsmasqError::SpawnFailed(format!("systemctl: {e}")))?;
        if !status.success() {
            return Err(DnsmasqError::SpawnFailed(format!(
                "systemctl restart systemd-resolved exited {status}"
            )));
        }
    }
    Ok(())
}

/// Reject suffixes containing anything other than `[a-zA-Z0-9._-]`.
/// The legitimate set is small (test, local, dev, etc.); anything
/// fancier risks shell injection even inside `osascript`.
fn sanitise_suffix(suffix: &str) -> Result<String> {
    let trimmed = suffix.trim_start_matches('.').to_string();
    if trimmed.is_empty() {
        return Err(DnsmasqError::SpawnFailed(
            "domain suffix cannot be empty".into(),
        ));
    }
    if trimmed
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'))
    {
        return Err(DnsmasqError::SpawnFailed(format!(
            "invalid domain suffix `{trimmed}`"
        )));
    }
    Ok(trimmed)
}

/// POSIX shell single-quote a string. Replaces existing single quotes
/// with `'\''` and wraps the whole thing in single quotes.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// AppleScript-quote a string: escape `\` and `"`. Newlines stay as
/// literal `\n` inside the script source; osascript handles them.
#[cfg(any(target_os = "macos", test))]
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_path_strips_leading_dot() {
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                resolver_file_path(".test"),
                PathBuf::from("/etc/resolver/test")
            );
            assert_eq!(
                resolver_file_path("local"),
                PathBuf::from("/etc/resolver/local")
            );
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                resolver_file_path(".test"),
                PathBuf::from("/etc/systemd/resolved.conf.d/portbay-test.conf")
            );
            assert_eq!(
                resolver_file_path("dev.local"),
                PathBuf::from("/etc/systemd/resolved.conf.d/portbay-dev-local.conf")
            );
        }
    }

    #[test]
    fn resolver_content_contains_loopback_and_port() {
        let c = resolver_file_content(53053);
        #[cfg(target_os = "macos")]
        {
            assert!(c.contains("nameserver 127.0.0.1"));
            assert!(c.contains("port 53053"));
        }
        #[cfg(target_os = "linux")]
        {
            assert!(c.contains("DNS=127.0.0.1:53053"));
            assert!(c.contains("Domains=~test"));
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            assert!(c.contains("nameserver 127.0.0.1"));
            assert!(c.contains("port 53053"));
        }
    }

    #[test]
    fn resolver_content_for_suffix_contains_requested_suffix() {
        let c = resolver_file_content_for_suffix("dev.local", 53053);
        #[cfg(target_os = "linux")]
        {
            assert!(c.contains("DNS=127.0.0.1:53053"));
            assert!(c.contains("Domains=~dev.local"));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(c.contains("nameserver 127.0.0.1"));
            assert!(c.contains("port 53053"));
        }
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("hello"), "'hello'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("/tmp/dir name"), "'/tmp/dir name'");
    }

    #[test]
    fn applescript_escape_handles_quotes_and_backslashes() {
        assert_eq!(applescript_escape(r#"foo "bar""#), r#"foo \"bar\""#);
        assert_eq!(applescript_escape(r#"C:\path"#), r#"C:\\path"#);
    }

    #[test]
    fn sanitise_rejects_shell_metachars() {
        assert!(sanitise_suffix("test").is_ok());
        assert!(sanitise_suffix(".local").is_ok());
        assert!(sanitise_suffix("dev-01").is_ok());
        assert!(sanitise_suffix("test;rm -rf").is_err());
        assert!(sanitise_suffix("test`echo").is_err());
        assert!(sanitise_suffix("test$X").is_err());
        assert!(sanitise_suffix("").is_err());
        assert!(sanitise_suffix(".").is_err());
    }

    #[test]
    fn is_installed_returns_false_when_file_missing() {
        // Use a suffix that almost certainly isn't installed on the
        // test host. If you run this and your test host happens to
        // have PortBay's platform resolver file for this suffix, the assertion
        // still tells the truth.
        assert!(!is_installed("portbay-unit-test-suffix", 53053));
    }
}
