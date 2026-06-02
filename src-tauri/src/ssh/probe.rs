//! Read-only reachability + host-key probe for a saved [`SshConnection`].
//!
//! The SSH transport handshake delivers the server's host key (to
//! [`check_server_key`]) *before* any authentication, so a single
//! **unauthenticated** connect yields everything the host dashboard needs at a
//! glance: is the box reachable, how far away is it (latency), what's its host
//! key fingerprint, and do we already trust it. We never send credentials and
//! never learn the key — this is purely observational, the read-only twin of
//! [`crate::ssh::backend::RusshClientHandler`] (which does TOFU-learn on the
//! real connect path).
//!
//! MVP scope: the **direct** target only. A host behind a `ProxyJump`/proxy
//! isn't reachable by a direct dial, so we report `unknown` rather than a
//! misleading `down`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use russh::client;
use russh_keys::key;
use serde::Serialize;
use tokio::time::timeout;

use crate::registry::SshConnection;

/// Cap both the TCP dial and the SSH handshake. A probe is a quick health
/// check, not a connect attempt — fail fast so a dead host doesn't stall the
/// dashboard's refresh.
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

/// Coarse health banding derived from reachability + handshake + latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeHealth {
    Healthy,
    Degraded,
    Down,
    /// Not probed (jumped/proxied host) — reachability is indeterminate here.
    Unknown,
}

/// Trust state of the server's host key against the local `known_hosts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostTrust {
    /// Key matches a known_hosts entry.
    Trusted,
    /// Host absent from known_hosts (first contact — TOFU not yet recorded).
    New,
    /// Key differs from the recorded one — a real warning.
    Changed,
    /// Couldn't determine (no key captured, or a read error).
    Unknown,
}

/// One probe's findings. `#[serde(rename_all = "camelCase")]` so the frontend
/// reads `latencyMs` / `fingerprint` directly.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    /// TCP port answered (the SSH handshake may still have failed).
    pub reachable: bool,
    /// Round-trip to a completed transport handshake, in milliseconds.
    pub latency_ms: Option<u32>,
    pub health: ProbeHealth,
    /// `SHA256:<base64>` of the server's host key, when captured.
    pub fingerprint: Option<String>,
    pub trust: HostTrust,
}

impl ProbeResult {
    /// The result for a host we deliberately don't dial (jumped/proxied).
    fn indeterminate() -> Self {
        Self {
            reachable: false,
            latency_ms: None,
            health: ProbeHealth::Unknown,
            fingerprint: None,
            trust: HostTrust::Unknown,
        }
    }
}

/// What the handler captures off the host key during the handshake.
#[derive(Default)]
struct Captured {
    fingerprint: Option<String>,
    trust: Option<HostTrust>,
}

/// A client handler that observes the host key and accepts it **without
/// learning it**, so the handshake completes and proves reachability while
/// leaving `known_hosts` untouched.
#[derive(Clone)]
struct ProbeHandler {
    host: String,
    port: u16,
    captured: Arc<Mutex<Captured>>,
}

#[async_trait]
impl client::Handler for ProbeHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        let fingerprint = format!("SHA256:{}", server_public_key.fingerprint());
        let trust = match russh_keys::check_known_hosts(&self.host, self.port, server_public_key) {
            Ok(true) => HostTrust::Trusted,
            Ok(false) => HostTrust::New,
            Err(russh_keys::Error::KeyChanged { .. }) => HostTrust::Changed,
            Err(_) => HostTrust::Unknown,
        };
        if let Ok(mut c) = self.captured.lock() {
            c.fingerprint = Some(fingerprint);
            c.trust = Some(trust);
        }
        // Accept unconditionally: this is a read-only probe that never
        // authenticates, so completing the handshake to even a changed-key host
        // exposes nothing. We report the trust state; we don't enforce it here.
        Ok(true)
    }
}

/// Probe `conn`'s reachability, latency, host-key fingerprint, and trust with a
/// single unauthenticated handshake. Never panics, never sends credentials,
/// never mutates `known_hosts`.
pub async fn probe_connection(conn: &SshConnection) -> ProbeResult {
    let jumped = conn
        .proxy_jump
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if jumped || conn.proxy.is_some() {
        return ProbeResult::indeterminate();
    }

    let host = conn.ssh_host.clone();
    let port = conn.ssh_port;
    let started = Instant::now();

    let stream = match timeout(
        PROBE_TIMEOUT,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        // Connection refused, no route, DNS failure, or dial timeout → down.
        _ => {
            return ProbeResult {
                reachable: false,
                latency_ms: None,
                health: ProbeHealth::Down,
                fingerprint: None,
                trust: HostTrust::Unknown,
            }
        }
    };

    let captured = Arc::new(Mutex::new(Captured::default()));
    let handler = ProbeHandler {
        host: host.clone(),
        port,
        captured: Arc::clone(&captured),
    };
    let config = Arc::new(client::Config::default());
    let handshake = timeout(
        PROBE_TIMEOUT,
        client::connect_stream(config, stream, handler),
    )
    .await;

    let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    let (fingerprint, trust) = {
        let c = captured.lock().unwrap_or_else(|e| e.into_inner());
        (c.fingerprint.clone(), c.trust.unwrap_or(HostTrust::Unknown))
    };

    match handshake {
        // Handshake completed → the unauthenticated handle drops here, tearing
        // the session down. The host is up and SSH answered, so it's Healthy —
        // latency is reported separately (a far-away host that connects fine is
        // not "degraded"). Only a half-open port (handshake failure, below)
        // counts as degraded.
        Ok(Ok(_handle)) => ProbeResult {
            reachable: true,
            latency_ms: Some(latency_ms),
            health: ProbeHealth::Healthy,
            fingerprint,
            trust,
        },
        // TCP answered but the SSH handshake failed/timed out: the port is open
        // yet SSH isn't healthy. Surface any key captured before the failure.
        _ => ProbeResult {
            reachable: true,
            latency_ms: Some(latency_ms),
            health: ProbeHealth::Degraded,
            fingerprint,
            trust,
        },
    }
}
