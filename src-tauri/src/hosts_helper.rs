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
        std::fs::set_permissions(socket_path, perms).map_err(|e| HelperError::io(socket_path, e))?;
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
