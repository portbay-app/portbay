//! Loopback CONNECT proxy that pins the sandboxed install phase to package
//! registries.
//!
//! macOS Seatbelt filters network by IP/port, not DNS name, so a profile alone
//! can't say "only reach npm/Packagist." The install phase therefore runs
//! `loopback_only` (it can reach `127.0.0.1` but nothing on the internet) with
//! its `HTTP(S)_PROXY` pointed at this proxy. The proxy runs *outside* the
//! sandbox, so it has real network access, and only forwards CONNECT tunnels to
//! an allowlist of registry domains — every other host is refused and recorded
//! so the UI can show what an install tried to reach.
//!
//! It allowlists by the CONNECT target hostname (and, for plain HTTP, the
//! request's Host), so it never decrypts TLS — no MITM, no cert handling.
//! Non-CONNECT (plain HTTP) requests are refused: every mainstream registry is
//! HTTPS, which always arrives as CONNECT through a proxy.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// Bare registry/package-host domains the install phase may reach. Matched
/// exactly or as a dot-bounded subdomain (so `npmjs.org` covers
/// `registry.npmjs.org`). Covers the default hosts for npm/pnpm/yarn/bun, pip,
/// Cargo, RubyGems, Go modules, Composer/Packagist, and the Git forges that
/// `git+https` dependencies and Composer `dist` archives are pulled from.
const REGISTRY_DOMAINS: &[&str] = &[
    // JavaScript.
    "npmjs.org",
    "npmjs.com",
    "yarnpkg.com",
    "nodejs.org",
    "bun.sh",
    // Python.
    "pypi.org",
    "pythonhosted.org",
    // Rust.
    "crates.io",
    // Ruby.
    "rubygems.org",
    // Go.
    "golang.org",
    "sum.golang.org",
    // PHP / Composer.
    "packagist.org",
    "getcomposer.org",
    // Git forges — git+https deps and Composer dist tarballs live here.
    "github.com",
    "githubusercontent.com",
    "gitlab.com",
    "bitbucket.org",
];

/// Allowlist of bare domains. `permits` matches a host exactly or as a
/// dot-bounded subdomain, so an attacker host like `npmjs.org.evil.com` is *not*
/// allowed by the `npmjs.org` entry.
#[derive(Clone)]
pub struct Allowlist(Arc<Vec<String>>);

impl Allowlist {
    /// The built-in package-registry allowlist.
    pub fn registries() -> Self {
        Self(Arc::new(
            REGISTRY_DOMAINS.iter().map(|s| s.to_string()).collect(),
        ))
    }

    /// Build from an explicit list — used by tests.
    pub fn from_domains(domains: Vec<String>) -> Self {
        Self(Arc::new(domains))
    }

    pub fn permits(&self, host: &str) -> bool {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return false;
        }
        self.0.iter().any(|d| {
            let d = d.to_ascii_lowercase();
            host == d || host.ends_with(&format!(".{d}"))
        })
    }
}

/// A running proxy bound to `127.0.0.1:port`. Drop or [`stop`](RunningProxy::stop)
/// it once the install finishes; `stop` returns the sorted set of hosts that were
/// refused.
pub struct RunningProxy {
    port: u16,
    blocked: Arc<Mutex<BTreeSet<String>>>,
    accept_task: JoinHandle<()>,
}

impl RunningProxy {
    /// Start the proxy with the built-in registry allowlist on an ephemeral
    /// loopback port.
    pub async fn start() -> std::io::Result<Self> {
        Self::start_with(Allowlist::registries()).await
    }

    /// Start with a caller-supplied allowlist (tests).
    pub async fn start_with(allow: Allowlist) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let blocked = Arc::new(Mutex::new(BTreeSet::new()));
        let blocked_for_task = blocked.clone();
        let accept_task = tokio::spawn(async move {
            // Exits when the listener errors or the task is aborted on `stop`.
            while let Ok((client, _)) = listener.accept().await {
                let allow = allow.clone();
                let blocked = blocked_for_task.clone();
                tokio::spawn(async move {
                    handle_conn(client, allow, blocked).await;
                });
            }
        });
        Ok(Self {
            port,
            blocked,
            accept_task,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>` — the value to put in `HTTP(S)_PROXY`.
    pub fn proxy_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Stop accepting connections and return the sorted hosts that were refused
    /// (non-registry CONNECT targets and any plain-HTTP attempts).
    pub fn stop(self) -> Vec<String> {
        self.accept_task.abort();
        self.blocked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

async fn handle_conn(
    mut client: TcpStream,
    allow: Allowlist,
    blocked: Arc<Mutex<BTreeSet<String>>>,
) {
    let head = match read_head(&mut client).await {
        Ok(h) => h,
        Err(_) => return,
    };
    let request_line = head.lines().next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    // Only HTTPS tunnels (CONNECT) are forwarded; every mainstream registry is
    // HTTPS, so a plain-HTTP request through the proxy is unexpected — refuse and
    // record it rather than open an unaudited egress path.
    if !method.eq_ignore_ascii_case("CONNECT") {
        let host = host_from_absolute_uri(target).unwrap_or_else(|| target.to_string());
        record(&blocked, &host);
        let _ = client
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\n\r\n")
            .await;
        return;
    }

    let Some((host, port)) = split_host_port(target) else {
        let _ = client
            .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
            .await;
        return;
    };

    if !allow.permits(&host) {
        record(&blocked, &host);
        let _ = client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
            .await;
        return;
    }

    let mut upstream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(s) => s,
        Err(_) => {
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                .await;
            return;
        }
    };
    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .is_err()
    {
        return;
    }
    // Splice both directions until either side closes. The proxy never sees
    // plaintext — this is an opaque TLS tunnel.
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

/// Read the request head (up to the blank line) so we can parse the request
/// line. Byte-at-a-time is fine for a short head; capped so a misbehaving client
/// can't make us buffer unbounded.
async fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            break;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() >= 16 * 1024 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn record(blocked: &Arc<Mutex<BTreeSet<String>>>, host: &str) {
    if host.is_empty() {
        return;
    }
    blocked
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(host.to_string());
}

/// Split a CONNECT target (`host:port`) into its parts. Defaults to 443 when no
/// port is present. Returns `None` for an empty host.
fn split_host_port(target: &str) -> Option<(String, u16)> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    match target.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            let port = port.parse().ok()?;
            Some((host.to_string(), port))
        }
        _ => Some((target.to_string(), 443)),
    }
}

/// Pull the host out of an absolute-form request URI (`http://host:port/path`),
/// used only to record what a refused plain-HTTP request was aiming at.
fn host_from_absolute_uri(uri: &str) -> Option<String> {
    let rest = uri
        .strip_prefix("http://")
        .or_else(|| uri.strip_prefix("https://"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = authority
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_matches_exact_and_subdomains_only() {
        let a = Allowlist::registries();
        assert!(a.permits("npmjs.org"));
        assert!(a.permits("registry.npmjs.org"));
        assert!(a.permits("REGISTRY.NPMJS.ORG")); // case-insensitive
        assert!(a.permits("codeload.github.com"));
        assert!(a.permits("files.pythonhosted.org"));
        // Not a registry.
        assert!(!a.permits("evil.com"));
        // Suffix must be dot-bounded — these are classic bypass attempts.
        assert!(!a.permits("notnpmjs.org"));
        assert!(!a.permits("npmjs.org.evil.com"));
        assert!(!a.permits(""));
    }

    #[test]
    fn split_host_port_defaults_to_443() {
        assert_eq!(
            split_host_port("registry.npmjs.org:443"),
            Some(("registry.npmjs.org".to_string(), 443))
        );
        assert_eq!(
            split_host_port("registry.npmjs.org"),
            Some(("registry.npmjs.org".to_string(), 443))
        );
        assert_eq!(
            split_host_port("host:8443"),
            Some(("host".to_string(), 8443))
        );
        assert_eq!(split_host_port(""), None);
    }

    #[test]
    fn host_from_absolute_uri_extracts_host() {
        assert_eq!(
            host_from_absolute_uri("http://example.com/path").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host_from_absolute_uri("http://example.com:8080/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_from_absolute_uri("/just/a/path"), None);
    }

    /// End-to-end: an allowed CONNECT tunnels through to a local echo upstream;
    /// a denied CONNECT gets 403 and is recorded. The allowlist is set to
    /// `localhost` so we can use a loopback upstream as the "registry".
    #[tokio::test]
    async fn connect_tunnels_allowed_and_refuses_denied() {
        // Upstream echo server standing in for an allowed registry host.
        let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let up_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = upstream.accept().await {
                let mut b = [0u8; 4];
                let _ = s.read_exact(&mut b).await;
                let _ = s.write_all(&b).await; // echo
            }
        });

        let proxy = RunningProxy::start_with(Allowlist::from_domains(vec!["localhost".into()]))
            .await
            .unwrap();

        // Allowed: CONNECT localhost:<up_port>, then the tunnel echoes our bytes.
        let mut c = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        c.write_all(format!("CONNECT localhost:{up_port} HTTP/1.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let resp = read_head(&mut c).await.unwrap();
        assert!(
            resp.starts_with("HTTP/1.1 200"),
            "expected tunnel, got: {resp}"
        );
        c.write_all(b"ping").await.unwrap();
        let mut echoed = [0u8; 4];
        c.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"ping");

        // Denied: CONNECT to a host not on the allowlist → 403.
        let mut d = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        d.write_all(b"CONNECT evil.com:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let resp = read_head(&mut d).await.unwrap();
        assert!(
            resp.starts_with("HTTP/1.1 403"),
            "expected 403, got: {resp}"
        );

        let blocked = proxy.stop();
        assert!(
            blocked.contains(&"evil.com".to_string()),
            "denied host should be recorded, got: {blocked:?}"
        );
    }
}
