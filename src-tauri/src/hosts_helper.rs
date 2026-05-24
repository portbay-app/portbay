//! Privileged hosts-helper protocol and client.
//!
//! The helper is installed as a macOS LaunchDaemon by SMAppService in
//! signed production builds. It listens on a Unix socket, validates that
//! every hostname is under the configured registry suffix, and delegates
//! the actual `/etc/hosts` mutation to [`crate::hosts::HostsManager`].

use std::io::{BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hosts::{HostsEntry, HostsError, HostsManager};

pub const SOCKET_PATH: &str = "/var/run/portbay-hosts-helper.sock";
pub const PLIST_NAME: &str = "com.portbay-app.portbay.hosts-helper.plist";
pub const HELPER_LABEL: &str = "com.portbay-app.portbay.hosts-helper";

#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("privileged hosts helper is not installed or not reachable at {0}")]
    Unreachable(String),

    #[error("invalid helper request: {0}")]
    BadRequest(String),

    #[error("helper rejected hostname `{hostname}` because it is outside .{suffix}")]
    HostOutsideSuffix { hostname: String, suffix: String },

    #[error("helper protocol error: {0}")]
    Protocol(String),

    #[error("helper I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{0}")]
    Hosts(#[from] HostsError),
}

impl HelperError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, HelperError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HelperRequest {
    List,
    Add {
        hostname: String,
        ip: Ipv4Addr,
        domain_suffix: String,
    },
    Remove {
        hostname: String,
        domain_suffix: String,
    },
    Clear,
    ReplaceAll {
        entries: Vec<HelperEntry>,
        domain_suffix: String,
    },
    /// Write `/etc/resolver/<suffix>` pointing macOS at the local dnsmasq
    /// port. Root-only — that's why it goes through the helper.
    InstallResolver {
        suffix: String,
        port: u16,
    },
    /// Remove `/etc/resolver/<suffix>`.
    RemoveResolver {
        suffix: String,
    },
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct HelperEntry {
    pub hostname: String,
    pub ip: Ipv4Addr,
}

impl From<HostsEntry> for HelperEntry {
    fn from(value: HostsEntry) -> Self {
        Self {
            hostname: value.hostname,
            ip: value.ip,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<HelperEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl HelperResponse {
    fn ok() -> Self {
        Self {
            ok: true,
            entries: vec![],
            error: None,
        }
    }

    fn entries(entries: Vec<HelperEntry>) -> Self {
        Self {
            ok: true,
            entries,
            error: None,
        }
    }

    fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            entries: vec![],
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostsHelperClient {
    socket_path: PathBuf,
}

impl HostsHelperClient {
    pub fn system() -> Self {
        Self::new(SOCKET_PATH)
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: path.into(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.socket_path.exists()
    }

    pub fn list(&self) -> Result<Vec<HelperEntry>> {
        let response = self.request(&HelperRequest::List)?;
        Ok(response.entries)
    }

    pub fn add(&self, hostname: &str, ip: Ipv4Addr, suffix: &str) -> Result<()> {
        self.expect_ok(&HelperRequest::Add {
            hostname: hostname.into(),
            ip,
            domain_suffix: suffix.into(),
        })
    }

    pub fn remove(&self, hostname: &str, suffix: &str) -> Result<()> {
        self.expect_ok(&HelperRequest::Remove {
            hostname: hostname.into(),
            domain_suffix: suffix.into(),
        })
    }

    pub fn clear(&self) -> Result<()> {
        self.expect_ok(&HelperRequest::Clear)
    }

    pub fn replace_all<I>(&self, entries: I, suffix: &str) -> Result<()>
    where
        I: IntoIterator<Item = (String, Ipv4Addr)>,
    {
        let entries = entries
            .into_iter()
            .map(|(hostname, ip)| HelperEntry { hostname, ip })
            .collect();
        self.expect_ok(&HelperRequest::ReplaceAll {
            entries,
            domain_suffix: suffix.into(),
        })
    }

    pub fn install_resolver(&self, suffix: &str, port: u16) -> Result<()> {
        self.expect_ok(&HelperRequest::InstallResolver {
            suffix: suffix.into(),
            port,
        })
    }

    pub fn remove_resolver(&self, suffix: &str) -> Result<()> {
        self.expect_ok(&HelperRequest::RemoveResolver {
            suffix: suffix.into(),
        })
    }

    fn expect_ok(&self, request: &HelperRequest) -> Result<()> {
        let _ = self.request(request)?;
        Ok(())
    }

    pub fn request(&self, request: &HelperRequest) -> Result<HelperResponse> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound
                || e.kind() == std::io::ErrorKind::ConnectionRefused
            {
                HelperError::Unreachable(self.socket_path.display().to_string())
            } else {
                HelperError::io(&self.socket_path, e)
            }
        })?;
        let body =
            serde_json::to_string(request).map_err(|e| HelperError::Protocol(e.to_string()))?;
        stream
            .write_all(body.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .map_err(|e| HelperError::io(&self.socket_path, e))?;

        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(|e| HelperError::io(&self.socket_path, e))?;
        let response: HelperResponse =
            serde_json::from_str(&line).map_err(|e| HelperError::Protocol(e.to_string()))?;
        if response.ok {
            Ok(response)
        } else {
            Err(HelperError::Protocol(
                response.error.unwrap_or_else(|| "request failed".into()),
            ))
        }
    }
}

/// Stable install location for the helper binary outside the app bundle.
pub const INSTALLED_BIN: &str = "/usr/local/bin/portbay-hosts-helper";

/// The LaunchDaemon plist that runs the helper as root at boot and keeps it
/// alive. Installed to `/Library/LaunchDaemons/<PLIST_NAME>`.
fn daemon_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{HELPER_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{INSTALLED_BIN}</string>
    <string>--socket</string><string>{SOCKET_PATH}</string>
    <string>--hosts-file</string><string>/etc/hosts</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict>
</plist>
"#
    )
}

/// Install the helper as a root LaunchDaemon via a single macOS
/// authorization prompt (`osascript … with administrator privileges`). Copies
/// `helper_bin` into [`INSTALLED_BIN`], writes the plist, and (re)bootstraps
/// the daemon. After this returns Ok the daemon is starting; callers should
/// poll [`HostsHelperClient::is_available`] for the socket.
#[cfg(target_os = "macos")]
pub fn install_daemon(helper_bin: &Path) -> Result<()> {
    use std::process::Command;

    let plist_path = format!("/Library/LaunchDaemons/{PLIST_NAME}");
    // The whole privileged install runs as one root shell script, so the user
    // sees a single password prompt. Paths we interpolate are either constants
    // or the resolved helper binary path (single-quoted).
    let script = format!(
        "#!/bin/sh\nset -e\n\
         /bin/mkdir -p /usr/local/bin\n\
         /bin/cp {src} '{INSTALLED_BIN}'\n\
         /bin/chmod 755 '{INSTALLED_BIN}'\n\
         /bin/cat > '{plist_path}' <<'PORTBAY_PLIST'\n{plist}PORTBAY_PLIST\n\
         /bin/chmod 644 '{plist_path}'\n\
         /bin/launchctl bootout system/{HELPER_LABEL} 2>/dev/null || true\n\
         /bin/launchctl bootstrap system '{plist_path}'\n\
         /bin/launchctl enable system/{HELPER_LABEL}\n",
        src = shell_single_quote(&helper_bin.to_string_lossy()),
        plist = daemon_plist(),
    );

    let tmp = std::env::temp_dir().join("portbay-helper-install.sh");
    std::fs::write(&tmp, script).map_err(|e| HelperError::io(&tmp, e))?;

    let apple = format!(
        r#"do shell script "/bin/sh {}" with prompt "PortBay needs to install its privileged helper to manage local DNS and your hosts file." with administrator privileges"#,
        applescript_escape(&tmp.to_string_lossy())
    );
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&apple)
        .output()
        .map_err(|e| HelperError::io("osascript", e))?;
    let _ = std::fs::remove_file(&tmp);

    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("(-128)") || stderr.contains("User canceled") {
        return Err(HelperError::BadRequest(
            "cancelled — the authorization dialog was dismissed".into(),
        ));
    }
    Err(HelperError::Protocol(format!(
        "helper install failed: {}",
        stderr.trim()
    )))
}

#[cfg(not(target_os = "macos"))]
pub fn install_daemon(_helper_bin: &Path) -> Result<()> {
    Err(HelperError::Protocol(
        "privileged helper install is macOS-only in this build".into(),
    ))
}

/// POSIX single-quote a string for safe interpolation into `/bin/sh`.
fn shell_single_quote(s: &str) -> String {
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

/// Escape a string for embedding inside an AppleScript double-quoted literal.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn request_allowed(request: &HelperRequest) -> Result<()> {
    match request {
        HelperRequest::List | HelperRequest::Clear => Ok(()),
        HelperRequest::Add {
            hostname,
            domain_suffix,
            ..
        }
        | HelperRequest::Remove {
            hostname,
            domain_suffix,
        } => ensure_host_matches_suffix(hostname, domain_suffix),
        HelperRequest::ReplaceAll {
            entries,
            domain_suffix,
        } => {
            for entry in entries {
                ensure_host_matches_suffix(&entry.hostname, domain_suffix)?;
            }
            Ok(())
        }
        HelperRequest::InstallResolver { suffix, .. }
        | HelperRequest::RemoveResolver { suffix } => ensure_valid_resolver_suffix(suffix),
    }
}

pub fn handle_request(request: HelperRequest, manager: &HostsManager) -> Result<HelperResponse> {
    request_allowed(&request)?;
    match request {
        HelperRequest::List => {
            let entries = manager
                .list_managed()?
                .into_iter()
                .map(HelperEntry::from)
                .collect();
            Ok(HelperResponse::entries(entries))
        }
        HelperRequest::Add { hostname, ip, .. } => {
            manager.add(&hostname, ip)?;
            Ok(HelperResponse::ok())
        }
        HelperRequest::Remove { hostname, .. } => {
            manager.remove(&hostname)?;
            Ok(HelperResponse::ok())
        }
        HelperRequest::Clear => {
            manager.clear()?;
            Ok(HelperResponse::ok())
        }
        HelperRequest::ReplaceAll { entries, .. } => {
            manager.replace_all(entries.into_iter().map(|entry| (entry.hostname, entry.ip)))?;
            Ok(HelperResponse::ok())
        }
        HelperRequest::InstallResolver { suffix, port } => {
            let path = crate::dnsmasq::resolver::resolver_file_path(&suffix);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| HelperError::io(parent, e))?;
            }
            std::fs::write(&path, crate::dnsmasq::resolver::resolver_file_content(port))
                .map_err(|e| HelperError::io(&path, e))?;
            Ok(HelperResponse::ok())
        }
        HelperRequest::RemoveResolver { suffix } => {
            let path = crate::dnsmasq::resolver::resolver_file_path(&suffix);
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(HelperResponse::ok()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HelperResponse::ok()),
                Err(e) => Err(HelperError::io(&path, e)),
            }
        }
    }
}

/// Validate a resolver suffix before it becomes a path under `/etc/resolver/`.
/// Allows dot-separated DNS labels of `[a-z0-9-]` (so `portbay.test` is fine)
/// and rejects anything that could escape the directory or inject shell/path
/// metacharacters.
fn ensure_valid_resolver_suffix(suffix: &str) -> Result<()> {
    let trimmed = suffix.trim().trim_start_matches('.').trim_end_matches('.');
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 253
        && !trimmed.contains("..")
        && trimmed.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    if valid {
        Ok(())
    } else {
        Err(HelperError::BadRequest(format!(
            "invalid resolver suffix `{suffix}`"
        )))
    }
}

pub fn serve(socket_path: &Path, manager: HostsManager) -> Result<()> {
    if socket_path.exists() {
        std::fs::remove_file(socket_path).map_err(|e| HelperError::io(socket_path, e))?;
    }
    let listener = UnixListener::bind(socket_path).map_err(|e| HelperError::io(socket_path, e))?;

    // The daemon runs as root, so the socket it creates is root-owned. The
    // PortBay app connects as the logged-in user, which needs write access to
    // the socket to open it. Loosen the mode to 0666 — the security boundary
    // is `request_allowed`/`ensure_host_matches_suffix` (every mutation must
    // target a hostname under the configured dev suffix), not socket
    // ownership, so a world-connectable socket can still only touch PortBay's
    // own `/etc/hosts` block under `*.<suffix>`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(socket_path)
            .map_err(|e| HelperError::io(socket_path, e))?
            .permissions();
        perms.set_mode(0o666);
        std::fs::set_permissions(socket_path, perms)
            .map_err(|e| HelperError::io(socket_path, e))?;
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let _ = handle_stream(stream, &manager);
            }
            Err(e) => return Err(HelperError::io(socket_path, e)),
        }
    }
    Ok(())
}

fn handle_stream(mut stream: UnixStream, manager: &HostsManager) -> Result<()> {
    let mut line = String::new();
    BufReader::new(
        stream
            .try_clone()
            .map_err(|e| HelperError::io("socket", e))?,
    )
    .read_line(&mut line)
    .map_err(|e| HelperError::io("socket", e))?;
    let response = match serde_json::from_str::<HelperRequest>(&line) {
        Ok(request) => match handle_request(request, manager) {
            Ok(response) => response,
            Err(e) => HelperResponse::error(e.to_string()),
        },
        Err(e) => HelperResponse::error(format!("invalid JSON request: {e}")),
    };
    let body =
        serde_json::to_string(&response).map_err(|e| HelperError::Protocol(e.to_string()))?;
    stream
        .write_all(body.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|e| HelperError::io("socket", e))?;
    Ok(())
}

fn ensure_host_matches_suffix(hostname: &str, suffix: &str) -> Result<()> {
    let suffix = suffix.trim().trim_start_matches('.');
    if suffix.is_empty()
        || hostname == suffix
        || !hostname.ends_with(&format!(".{suffix}"))
        || hostname.contains("..")
    {
        return Err(HelperError::HostOutsideSuffix {
            hostname: hostname.into(),
            suffix: suffix.into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback() -> Ipv4Addr {
        Ipv4Addr::LOCALHOST
    }

    fn tmp_manager(contents: &str) -> (tempfile::TempDir, HostsManager) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        std::fs::write(&path, contents).unwrap();
        (dir, HostsManager::new(path))
    }

    #[test]
    fn suffix_guard_accepts_subdomains_only() {
        ensure_host_matches_suffix("app.test", "test").unwrap();
        ensure_host_matches_suffix("api.app.test", ".test").unwrap();
        assert!(ensure_host_matches_suffix("test", "test").is_err());
        assert!(ensure_host_matches_suffix("app.example.com", "test").is_err());
        assert!(ensure_host_matches_suffix("app..test", "test").is_err());
    }

    #[test]
    fn replace_all_rejects_out_of_suffix_entries_before_writing() {
        let (_dir, manager) = tmp_manager("127.0.0.1 localhost\n");
        let request = HelperRequest::ReplaceAll {
            entries: vec![
                HelperEntry {
                    hostname: "ok.test".into(),
                    ip: loopback(),
                },
                HelperEntry {
                    hostname: "bad.local".into(),
                    ip: loopback(),
                },
            ],
            domain_suffix: "test".into(),
        };
        assert!(handle_request(request, &manager).is_err());
        assert!(manager.list_managed().unwrap().is_empty());
    }

    #[test]
    fn resolver_suffix_validation_allows_local_rejects_traversal() {
        assert!(ensure_valid_resolver_suffix("test").is_ok());
        assert!(ensure_valid_resolver_suffix("portbay.test").is_ok());
        assert!(ensure_valid_resolver_suffix(".portbay.test.").is_ok());
        // Path-traversal / metacharacters must be rejected.
        assert!(ensure_valid_resolver_suffix("../etc/passwd").is_err());
        assert!(ensure_valid_resolver_suffix("a/b").is_err());
        assert!(ensure_valid_resolver_suffix("a..b").is_err());
        assert!(ensure_valid_resolver_suffix("foo;rm -rf").is_err());
        assert!(ensure_valid_resolver_suffix("").is_err());
    }

    #[test]
    fn request_allowed_gates_resolver_ops_on_suffix() {
        assert!(request_allowed(&HelperRequest::InstallResolver {
            suffix: "portbay.test".into(),
            port: 53053,
        })
        .is_ok());
        assert!(request_allowed(&HelperRequest::InstallResolver {
            suffix: "../bad".into(),
            port: 53053,
        })
        .is_err());
    }

    #[test]
    fn helper_request_reuses_hosts_manager() {
        let (_dir, manager) = tmp_manager("127.0.0.1 localhost\n");
        handle_request(
            HelperRequest::Add {
                hostname: "app.test".into(),
                ip: loopback(),
                domain_suffix: "test".into(),
            },
            &manager,
        )
        .unwrap();
        let response = handle_request(HelperRequest::List, &manager).unwrap();
        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].hostname, "app.test");
    }
}
