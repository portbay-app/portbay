use std::net::{TcpListener, TcpStream};
use std::process::{Command as StdCommand, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use russh::client;
use russh::keys::{self, HashAlg, PublicKey};
use tokio::io::{copy_bidirectional, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio::sync::Mutex as TokioMutex;

use crate::registry::{
    SshAuthKind, SshConnection, SshConnectionId, SshForwardKind, SshTunnelConnection, SshTunnelId,
};
use crate::ssh::interaction::{
    HostKeyDecision, HostKeyPrompt, HostKeyState, SshInteractor, PROMPT_TIMEOUT,
};
use crate::ssh::probe::HostTrust;
use crate::ssh::secret::SecretString;

/// A fully-resolved tunnel: an [`SshTunnelConnection`] joined with its
/// [`SshConnection`]. Carries everything the command/spawn builders read, so
/// they never reach back into the registry. Field names mirror the pre-v3
/// self-contained tunnel, so the builders below are unchanged by the split.
#[derive(Debug, Clone)]
pub struct EffectiveSshTunnel {
    pub id: SshTunnelId,
    pub connection_id: SshConnectionId,
    pub name: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub auth_kind: SshAuthKind,
    pub key_path: Option<String>,
    pub proxy_jump: Option<String>,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub forward_kind: SshForwardKind,
    pub keep_alive: bool,
    pub auto_reconnect: bool,
}

/// Resolve every saved tunnel in `registry` against its connection. Tunnels
/// whose connection is missing (corruption) are skipped with a warning rather
/// than failing the whole list. Shared by the commands layer and the
/// cross-process state mirror so they always agree.
pub fn resolve_tunnels(registry: &crate::registry::Registry) -> Vec<EffectiveSshTunnel> {
    registry
        .list_ssh_tunnels()
        .iter()
        .filter_map(|t| match registry.get_ssh_connection(&t.connection_id) {
            Some(c) => {
                // Fold in a borrowed identity (user / key / auth) before resolving.
                let effective = registry.effective_ssh_connection(c);
                Some(EffectiveSshTunnel::resolve(t, &effective))
            }
            None => {
                tracing::warn!(
                    tunnel_id = %t.id,
                    connection_id = %t.connection_id,
                    "SSH tunnel references a missing connection; skipping"
                );
                None
            }
        })
        .collect()
}

impl EffectiveSshTunnel {
    /// Join a saved tunnel with the connection it references.
    pub fn resolve(tunnel: &SshTunnelConnection, connection: &SshConnection) -> Self {
        Self {
            id: tunnel.id.clone(),
            connection_id: connection.id.clone(),
            name: tunnel.name.clone(),
            ssh_host: connection.ssh_host.clone(),
            ssh_port: connection.ssh_port,
            ssh_user: connection.ssh_user.clone(),
            auth_kind: connection.auth_kind,
            key_path: connection.key_path.clone(),
            proxy_jump: connection.proxy_jump.clone(),
            local_host: tunnel.local_host.clone(),
            local_port: tunnel.local_port,
            remote_host: tunnel.remote_host.clone(),
            remote_port: tunnel.remote_port,
            forward_kind: tunnel.forward_kind,
            keep_alive: tunnel.keep_alive,
            auto_reconnect: tunnel.auto_reconnect,
        }
    }
}

const DEFAULT_SSH_PORT: u16 = 22;
const START_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const CLI_PATH_SUFFIX: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("a password is required for SSH password authentication")]
    PasswordRequired,

    #[error("password authentication currently supports local (-L) forwards only")]
    PasswordForwardUnsupported,

    #[error("failed to launch system ssh: {0}")]
    SpawnFailed(String),

    #[error("russh tunnel failed: {0}")]
    Russh(String),

    /// The connection's key file is passphrase-protected and the passphrase is
    /// neither supplied nor held by the agent. Surfaced to the UI so it can
    /// prompt for the passphrase (VS Code-style) and retry.
    #[error("the SSH key for {host} is passphrase-protected — enter its passphrase")]
    NeedsKeyPassphrase { host: String },

    /// A password-auth host has no usable password (none stored, none supplied).
    /// Surfaced so the UI can prompt for the password and retry.
    #[error("{host} needs an SSH password")]
    MissingPassword { host: String },

    #[error("ssh exited before the tunnel became ready")]
    ExitedEarly,

    #[error("timed out waiting for SSH tunnel on 127.0.0.1:{0}")]
    ReadinessTimeout(u16),

    #[error("SSH tunnel `{0}` is already running")]
    AlreadyRunning(String),

    #[error("SSH tunnel `{0}` is not running")]
    NotRunning(String),

    #[error("system ssh is not installed")]
    BinaryMissing,

    /// The user declined the host-key trust prompt (or it timed out). Fail
    /// closed: no credentials were sent and nothing was written to
    /// `known_hosts`.
    #[error("the host key for {0} was not accepted")]
    HostKeyRejected(String),
}

pub type Result<T> = std::result::Result<T, SshError>;

#[derive(Debug)]
pub enum SshProcess {
    System {
        child: std::process::Child,
        /// Keeps a Trust-Once pinned known_hosts file alive (auto-deleted on
        /// drop) for the child's lifetime, so `ssh` can re-read it and nothing
        /// is persisted past the session.
        pinned_known_hosts: Option<tempfile::TempPath>,
    },
    Russh {
        running: Arc<AtomicBool>,
        alive: Arc<AtomicBool>,
    },
}

impl SshProcess {
    pub fn stop(&mut self) {
        match self {
            Self::System { child, .. } => {
                let _ = child.kill();
            }
            Self::Russh { running, .. } => {
                running.store(false, Ordering::Relaxed);
            }
        }
    }

    pub fn is_running(&mut self) -> bool {
        match self {
            Self::System { child, .. } => match child.try_wait() {
                Ok(Some(_)) | Err(_) => false,
                Ok(None) => true,
            },
            Self::Russh { running, alive } => {
                running.load(Ordering::Relaxed) && alive.load(Ordering::Relaxed)
            }
        }
    }
}

impl Drop for SshProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
pub struct RusshClientHandler {
    pub(crate) ssh_host: String,
    pub(crate) ssh_port: u16,
    /// Decides untrusted host keys. `None` preserves the legacy silent TOFU
    /// (learn a new key, reject a changed one) for truly headless callers —
    /// the MCP agent — that have no window to prompt. UI-triggered paths
    /// (sessions, tunnels, connection tests) must pass a real interactor.
    interactor: Option<Arc<dyn SshInteractor>>,
}

impl RusshClientHandler {
    pub(crate) fn with_interactor(
        ssh_host: impl Into<String>,
        ssh_port: u16,
        interactor: Option<Arc<dyn SshInteractor>>,
    ) -> Self {
        Self {
            ssh_host: ssh_host.into(),
            ssh_port,
            interactor,
        }
    }

    /// Persist a host key to `known_hosts`, logging (not failing) on error.
    fn learn(&self, key: &PublicKey) {
        if let Err(e) = keys::known_hosts::learn_known_hosts(&self.ssh_host, self.ssh_port, key) {
            tracing::warn!(
                host = %self.ssh_host,
                port = self.ssh_port,
                error = %e,
                "could not persist SSH host key"
            );
        }
    }

    /// First-contact key. With no interactor, keep the legacy silent learn;
    /// with one, ask the user and honour their choice.
    async fn decide_new_key(&self, key: &PublicKey) -> std::result::Result<bool, russh::Error> {
        let Some(interactor) = &self.interactor else {
            self.learn(key);
            return Ok(true);
        };
        let decision = interactor
            .host_key_decision(self.prompt(key, HostKeyState::New))
            .await;
        match decision {
            HostKeyDecision::TrustAndSave => {
                self.learn(key);
                Ok(true)
            }
            HostKeyDecision::TrustOnce => Ok(true),
            // Reject the key for this handshake (russh treats `Ok(false)` as a
            // rejected server key — no need to fabricate an error variant).
            HostKeyDecision::Reject => Ok(false),
        }
    }

    /// Changed key (mismatch with `known_hosts`). Reject unless the user
    /// explicitly approves replacing the stored key.
    async fn decide_changed_key(
        &self,
        key: &PublicKey,
        line: usize,
    ) -> std::result::Result<bool, russh::Error> {
        let Some(interactor) = &self.interactor else {
            tracing::error!(
                host = %self.ssh_host,
                port = self.ssh_port,
                line,
                "SSH host key changed; rejecting connection"
            );
            return Err(russh::Error::KeyChanged { line });
        };
        match interactor
            .host_key_decision(self.prompt(key, HostKeyState::Changed))
            .await
        {
            // Replace: drop the stale entry, then record the accepted key so the
            // mismatch doesn't reappear on the next connect.
            HostKeyDecision::TrustAndSave => {
                if let Err(e) = crate::ssh::known_hosts::remove_host(&self.ssh_host, self.ssh_port)
                {
                    tracing::warn!(
                        host = %self.ssh_host,
                        error = %e,
                        "couldn't remove stale known_hosts entry before replacing"
                    );
                }
                self.learn(key);
                Ok(true)
            }
            // Accept for this session only; leave the stale record so the next
            // connect prompts again.
            HostKeyDecision::TrustOnce => Ok(true),
            HostKeyDecision::Reject => {
                tracing::error!(
                    host = %self.ssh_host,
                    port = self.ssh_port,
                    line,
                    "SSH host key changed; user declined"
                );
                Err(russh::Error::KeyChanged { line })
            }
        }
    }

    /// Build the prompt payload for the frontend (the interactor fills in the
    /// `flow_id`). For a changed key, look up the previously-trusted key of the
    /// same algorithm so the dialog can show old-vs-new fingerprints.
    fn prompt(&self, key: &PublicKey, state: HostKeyState) -> HostKeyPrompt {
        let key_type = key.algorithm().as_str().to_string();
        let expected_fingerprint = match state {
            HostKeyState::Changed => crate::ssh::known_hosts::stored_fingerprint(
                &self.ssh_host,
                self.ssh_port,
                &key_type,
            ),
            HostKeyState::New => None,
        };
        HostKeyPrompt {
            host: self.ssh_host.clone(),
            port: self.ssh_port,
            state,
            key_type,
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            expected_fingerprint,
        }
    }
}

impl client::Handler for RusshClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        match keys::check_known_hosts(&self.ssh_host, self.ssh_port, server_public_key) {
            // Already trusted — proceed silently.
            Ok(true) => Ok(true),
            // First contact: trust-on-first-use, surfacing the decision when a UI
            // interactor is present.
            Ok(false) => self.decide_new_key(server_public_key).await,
            // Mismatch with the recorded key: reject unless the user approves.
            Err(keys::Error::KeyChanged { line }) => {
                self.decide_changed_key(server_public_key, line).await
            }
            Err(e) => {
                tracing::error!(
                    host = %self.ssh_host,
                    port = self.ssh_port,
                    error = %e,
                    "SSH host-key check failed"
                );
                Err(russh::Error::CouldNotReadKey)
            }
        }
    }
}

#[inline]
pub fn should_use_system_ssh(ssh_password: Option<&str>) -> bool {
    ssh_password.map(|p| p.trim().is_empty()).unwrap_or(true)
}

#[inline]
pub fn build_tunnel_key(
    ssh_user: &str,
    ssh_host: &str,
    ssh_port: u16,
    remote_host: &str,
    remote_port: u16,
) -> String {
    format!("{ssh_user}@{ssh_host}:{ssh_port}:{remote_host}->{remote_port}")
}

/// How a spawned system `ssh` should verify the server's host key.
#[derive(Debug)]
pub enum SystemHostTrust {
    /// Legacy `accept-new`: silently learn unknown keys, reject changed ones.
    /// Used when there is no interactor (headless callers), the host sits
    /// behind a `ProxyJump` (we can't dial it directly to pre-flight), or the
    /// pre-flight handshake itself failed (russh may lack an algorithm the
    /// server requires — never regress a tunnel system `ssh` could open).
    AcceptNew,
    /// The key is in `known_hosts` (already trusted, or the user just chose
    /// Trust & Save): enforce strictly so a swap after the pre-flight fails.
    Strict,
    /// Trust Once: pin exactly the key the user approved via a throwaway
    /// known_hosts file; nothing is persisted past this process.
    PinnedOnce(tempfile::TempPath),
}

impl SystemHostTrust {
    fn args(&self) -> Vec<String> {
        match self {
            Self::AcceptNew => vec!["-o".into(), "StrictHostKeyChecking=accept-new".into()],
            Self::Strict => vec!["-o".into(), "StrictHostKeyChecking=yes".into()],
            Self::PinnedOnce(path) => vec![
                "-o".into(),
                "StrictHostKeyChecking=yes".into(),
                // Only the pinned file: a Trust-Once on a *changed* key must
                // not trip over the stale entry still in ~/.ssh/known_hosts.
                "-o".into(),
                format!("UserKnownHostsFile={}", path.display()),
                "-o".into(),
                "GlobalKnownHostsFile=/dev/null".into(),
            ],
        }
    }
}

/// What the pre-flight handshake captures off the server's host key. Strings
/// only (no `PublicKey`), so learning/pinning is a plain line write in
/// the standard OpenSSH format.
struct CapturedHostKey {
    key_type: String,
    key_base64: String,
    fingerprint: String,
    trust: HostTrust,
}

/// Read-only handler that records the server key and lets the handshake
/// complete (we never authenticate on this connection) — the probe's twin,
/// but keeping the raw key material so the caller can learn or pin it.
#[derive(Clone)]
struct KeyCaptureHandler {
    host: String,
    port: u16,
    captured: Arc<std::sync::Mutex<Option<CapturedHostKey>>>,
}

impl client::Handler for KeyCaptureHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        use russh::keys::PublicKeyBase64;
        let trust = match keys::check_known_hosts(&self.host, self.port, key) {
            Ok(true) => HostTrust::Trusted,
            Ok(false) => HostTrust::New,
            Err(keys::Error::KeyChanged { .. }) => HostTrust::Changed,
            Err(_) => HostTrust::Unknown,
        };
        *self.captured.lock().unwrap_or_else(|e| e.into_inner()) = Some(CapturedHostKey {
            key_type: key.algorithm().as_str().to_string(),
            key_base64: key.public_key_base64(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            trust,
        });
        Ok(true)
    }
}

/// Pre-flight the host key for a system-`ssh` spawn. With a UI interactor we
/// fetch the server key over a throwaway unauthenticated handshake, surface
/// new/changed keys through the same Trust dialog the russh path uses, and
/// return the strictness the child should run with. Every fallback to
/// `AcceptNew` is the pre-2026-06 status quo, never something weaker.
async fn resolve_system_host_trust(
    profile: &EffectiveSshTunnel,
    interactor: Option<&Arc<dyn SshInteractor>>,
) -> Result<SystemHostTrust> {
    let Some(interactor) = interactor else {
        return Ok(SystemHostTrust::AcceptNew);
    };
    if profile
        .proxy_jump
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
    {
        return Ok(SystemHostTrust::AcceptNew);
    }

    let captured: Arc<std::sync::Mutex<Option<CapturedHostKey>>> = Arc::default();
    let handler = KeyCaptureHandler {
        host: profile.ssh_host.clone(),
        port: profile.ssh_port,
        captured: Arc::clone(&captured),
    };
    let config = Arc::new(client::Config::default());
    let addr = format!("{}:{}", profile.ssh_host, profile.ssh_port);
    // The handle (when the handshake completes) drops immediately — this
    // connection never authenticates.
    let _ = tokio::time::timeout(START_TIMEOUT, client::connect(config, addr, handler)).await;

    let Some(captured) = captured.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        tracing::warn!(
            host = %profile.ssh_host,
            port = profile.ssh_port,
            "host-key pre-flight captured no key; falling back to accept-new"
        );
        return Ok(SystemHostTrust::AcceptNew);
    };

    let state = match captured.trust {
        HostTrust::Trusted => return Ok(SystemHostTrust::Strict),
        HostTrust::New => HostKeyState::New,
        HostTrust::Changed => HostKeyState::Changed,
        HostTrust::Unknown => {
            tracing::warn!(
                host = %profile.ssh_host,
                "known_hosts unreadable during pre-flight; falling back to accept-new"
            );
            return Ok(SystemHostTrust::AcceptNew);
        }
    };

    let prompt = HostKeyPrompt {
        host: profile.ssh_host.clone(),
        port: profile.ssh_port,
        state,
        key_type: captured.key_type.clone(),
        fingerprint: captured.fingerprint.clone(),
        expected_fingerprint: match state {
            HostKeyState::Changed => crate::ssh::known_hosts::stored_fingerprint(
                &profile.ssh_host,
                profile.ssh_port,
                &captured.key_type,
            ),
            HostKeyState::New => None,
        },
    };

    match interactor.host_key_decision(prompt).await {
        HostKeyDecision::TrustAndSave => {
            if matches!(state, HostKeyState::Changed) {
                if let Err(e) =
                    crate::ssh::known_hosts::remove_host(&profile.ssh_host, profile.ssh_port)
                {
                    tracing::warn!(
                        host = %profile.ssh_host,
                        error = %e,
                        "couldn't remove stale known_hosts entry before replacing"
                    );
                }
            }
            crate::ssh::known_hosts::append_host(
                &profile.ssh_host,
                profile.ssh_port,
                &captured.key_type,
                &captured.key_base64,
            )
            .map_err(|e| SshError::Russh(format!("could not persist host key: {e}")))?;
            Ok(SystemHostTrust::Strict)
        }
        HostKeyDecision::TrustOnce => {
            use std::io::Write;
            let mut tmp = tempfile::NamedTempFile::new()
                .map_err(|e| SshError::Russh(format!("could not pin host key: {e}")))?;
            writeln!(
                tmp,
                "{} {} {}",
                crate::ssh::known_hosts::host_entry_name(&profile.ssh_host, profile.ssh_port),
                captured.key_type,
                captured.key_base64
            )
            .map_err(|e| SshError::Russh(format!("could not pin host key: {e}")))?;
            Ok(SystemHostTrust::PinnedOnce(tmp.into_temp_path()))
        }
        HostKeyDecision::Reject => Err(SshError::HostKeyRejected(profile.ssh_host.clone())),
    }
}

/// Blocking wrapper for [`resolve_system_host_trust`] — the spawn paths run on
/// dedicated/blocking threads (never the async worker pool), so a private
/// runtime here is the same pattern as [`spawn_russh_local_forward`].
fn resolve_system_host_trust_blocking(
    profile: &EffectiveSshTunnel,
    interactor: Option<&Arc<dyn SshInteractor>>,
) -> Result<SystemHostTrust> {
    if interactor.is_none() {
        return Ok(SystemHostTrust::AcceptNew);
    }
    let runtime = Runtime::new().map_err(|e| SshError::Russh(e.to_string()))?;
    runtime.block_on(resolve_system_host_trust(profile, interactor))
}

pub fn ssh_args(profile: &EffectiveSshTunnel) -> Vec<String> {
    ssh_args_with_trust(profile, &SystemHostTrust::AcceptNew)
}

fn ssh_args_with_trust(profile: &EffectiveSshTunnel, trust: &SystemHostTrust) -> Vec<String> {
    let mut args = Vec::with_capacity(24);
    args.push("-N".into());
    args.push("-o".into());
    args.push("BatchMode=yes".into());
    args.extend(trust.args());
    args.push("-o".into());
    args.push("ExitOnForwardFailure=yes".into());

    if profile.keep_alive || profile.auto_reconnect {
        args.push("-o".into());
        args.push("ServerAliveInterval=15".into());
        args.push("-o".into());
        args.push("ServerAliveCountMax=3".into());
    }

    match profile.forward_kind {
        SshForwardKind::Local => {
            args.push("-L".into());
            args.push(format!(
                "{}:{}:{}:{}",
                profile.local_host, profile.local_port, profile.remote_host, profile.remote_port
            ));
        }
        SshForwardKind::Reverse => {
            args.push("-R".into());
            // Always name the remote bind address. Without one, a server
            // running `GatewayPorts yes` (common on bastions) binds the
            // wildcard address and the user's local service becomes reachable
            // through the server with zero indication in the UI. `local_host`
            // is the listen-bind address in both directions (-L binds it
            // locally, -R requests it server-side); it's loopback unless the
            // user explicitly opted into a wider bind at save time.
            args.push(format!(
                "{}:{}:{}:{}",
                reverse_bind_host(profile),
                profile.local_port,
                profile.remote_host,
                profile.remote_port
            ));
        }
        SshForwardKind::Socks => {
            args.push("-D".into());
            args.push(format!("{}:{}", profile.local_host, profile.local_port));
        }
    }

    if let Some(proxy_jump) = profile
        .proxy_jump
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("-J".into());
        args.push(proxy_jump.into());
    }

    if profile.ssh_port != DEFAULT_SSH_PORT {
        args.push("-p".into());
        args.push(profile.ssh_port.to_string());
    }

    if let Some(key_path) = profile
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("-i".into());
        args.push(key_path.into());
    }

    args.push(destination(profile));
    args
}

/// The bind address a reverse (-R) forward requests on the server: the
/// tunnel's `local_host`, falling back to loopback for pre-existing entries
/// saved before `local_host` applied to reverse forwards. Never empty — an
/// empty bind address means "all interfaces" to OpenSSH.
fn reverse_bind_host(profile: &EffectiveSshTunnel) -> &str {
    let host = profile.local_host.trim();
    if host.is_empty() {
        "127.0.0.1"
    } else {
        host
    }
}

pub fn equivalent_ssh_command(profile: &EffectiveSshTunnel) -> String {
    let mut parts = vec!["ssh".to_string()];
    parts.extend(ssh_args(profile).into_iter().map(|arg| shell_quote(&arg)));
    parts.join(" ")
}

pub fn spawn_system_ssh(
    profile: &EffectiveSshTunnel,
    trust: &SystemHostTrust,
) -> Result<std::process::Child> {
    let ssh = system_ssh_path()?;
    let mut cmd = StdCommand::new(ssh);
    cmd.args(ssh_args_with_trust(profile, trust))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_ssh_environment(&mut cmd);
    cmd.spawn()
        .map_err(|e| SshError::SpawnFailed(e.to_string()))
}

pub fn spawn_tunnel(
    profile: &EffectiveSshTunnel,
    password: Option<&str>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<SshProcess> {
    if should_use_system_ssh(password) {
        let trust = resolve_system_host_trust_blocking(profile, interactor.as_ref())?;
        let child = spawn_system_ssh(profile, &trust)?;
        let pinned_known_hosts = match trust {
            SystemHostTrust::PinnedOnce(path) => Some(path),
            _ => None,
        };
        return Ok(SshProcess::System {
            child,
            pinned_known_hosts,
        });
    }

    if !matches!(profile.forward_kind, SshForwardKind::Local) {
        return Err(SshError::PasswordForwardUnsupported);
    }
    spawn_russh_local_forward(profile, password, interactor)
}

fn spawn_russh_local_forward(
    profile: &EffectiveSshTunnel,
    password: Option<&str>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<SshProcess> {
    let password = password
        .filter(|p| !p.trim().is_empty())
        .map(|p| SecretString::new(p.to_owned()))
        .ok_or(SshError::PasswordRequired)?;
    let listener = TcpListener::bind((profile.local_host.as_str(), profile.local_port))
        .map_err(|e| SshError::Russh(format!("failed to bind local port: {e}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| SshError::Russh(format!("failed to configure listener: {e}")))?;

    let running = Arc::new(AtomicBool::new(true));
    let alive = Arc::new(AtomicBool::new(false));
    let running_for_thread = running.clone();
    let alive_for_thread = alive.clone();
    let profile_for_thread = profile.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    // A host-key prompt can legitimately park the handshake for up to
    // PROMPT_TIMEOUT; only the promptless path keeps the tight deadline.
    let ready_timeout = if interactor.is_some() {
        START_TIMEOUT + PROMPT_TIMEOUT
    } else {
        START_TIMEOUT
    };

    thread::spawn(move || {
        let runtime = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("failed to start async runtime: {e}")));
                return;
            }
        };

        let ready_tx_for_async = ready_tx.clone();
        let alive_for_async = alive_for_thread.clone();
        let result = runtime.block_on(async move {
            let config = Arc::new(client::Config::default());
            let addr = format!(
                "{}:{}",
                profile_for_thread.ssh_host, profile_for_thread.ssh_port
            );
            let mut handle = client::connect(
                config,
                addr,
                RusshClientHandler {
                    ssh_host: profile_for_thread.ssh_host.clone(),
                    ssh_port: profile_for_thread.ssh_port,
                    // Password tunnels prompt like any interactive connect: a
                    // first-contact MITM here would capture the password, so an
                    // unknown key must be the user's call, never silent TOFU.
                    interactor,
                },
            )
            .await
            .map_err(|e| format!("failed to connect to SSH server: {e}"))?;

            let authenticated = tokio::time::timeout(
                AUTH_TIMEOUT,
                handle.authenticate_password(&profile_for_thread.ssh_user, password.as_str()),
            )
            .await
            .map_err(|_| "SSH password authentication timed out".to_string())?
            .map_err(|e| format!("SSH password authentication failed: {e}"))?
            .success();

            if !authenticated {
                return Err("SSH password authentication failed".to_string());
            }

            let listener = tokio::net::TcpListener::from_std(listener)
                .map_err(|e| format!("failed to configure async listener: {e}"))?;
            let handle = Arc::new(TokioMutex::new(handle));
            alive_for_async.store(true, Ordering::Relaxed);
            let _ = ready_tx_for_async.send(Ok(()));

            while running_for_thread.load(Ordering::Relaxed) {
                // Detect a silently-dropped upstream session. russh keeps this
                // *local* listener alive even after the SSH connection to the
                // server dies, so without this probe the tunnel would look "up"
                // forever and the reconnect supervisor would never fire. Breaking
                // out lets the thread fall through and clear `alive`, which the
                // supervisor reads as "needs reconnecting".
                if handle.lock().await.is_closed() {
                    break;
                }
                let accept = tokio::time::timeout(ACCEPT_POLL_INTERVAL, listener.accept()).await;
                let (mut stream, _) = match accept {
                    Ok(Ok(result)) => result,
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "SSH local listener accept failed");
                        continue;
                    }
                    Err(_) => continue,
                };

                let handle = handle.clone();
                let remote_host = profile_for_thread.remote_host.clone();
                let remote_port = profile_for_thread.remote_port;
                tokio::spawn(async move {
                    let handle = handle.lock().await;
                    let channel = match handle
                        .channel_open_direct_tcpip(
                            remote_host,
                            u32::from(remote_port),
                            "127.0.0.1",
                            0,
                        )
                        .await
                    {
                        Ok(channel) => channel,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to open SSH direct-tcpip channel");
                            return;
                        }
                    };
                    drop(handle);

                    let mut channel_stream = channel.into_stream();
                    if let Err(e) = copy_bidirectional(&mut stream, &mut channel_stream).await {
                        tracing::debug!(error = %e, "SSH tunnel copy finished with error");
                    }
                    // Shut both sides down explicitly. `copy_bidirectional`
                    // returns on the first EOF/error without closing the other
                    // direction, and a half-closed keep-alive client can pin
                    // the SSH channel open (the session caps channels at 128).
                    let _ = stream.shutdown().await;
                    let _ = channel_stream.shutdown().await;
                });
            }

            Ok(())
        });

        alive_for_thread.store(false, Ordering::Relaxed);
        if let Err(err) = result {
            let _ = ready_tx.send(Err(err));
        }
    });

    match ready_rx.recv_timeout(ready_timeout) {
        Ok(Ok(())) => Ok(SshProcess::Russh { running, alive }),
        Ok(Err(err)) => Err(SshError::Russh(err)),
        Err(_) => {
            // Tell the worker to wind down — otherwise a connect that resolves
            // *after* this deadline would keep serving on an orphaned listener
            // no manager entry owns.
            running.store(false, Ordering::Relaxed);
            Err(SshError::ReadinessTimeout(profile.local_port))
        }
    }
}

pub fn wait_for_ready(child: &mut std::process::Child, local_port: u16) -> Result<()> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return Err(SshError::ExitedEarly);
        }
        if TcpStream::connect(("127.0.0.1", local_port)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err(SshError::ReadinessTimeout(local_port));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

pub fn test_system_connection(profile: &EffectiveSshTunnel, trust: &SystemHostTrust) -> Result<()> {
    let ssh = system_ssh_path()?;
    let mut args = Vec::with_capacity(18);
    args.push("-o".to_string());
    args.push("BatchMode=yes".to_string());
    args.push("-o".to_string());
    args.push("ConnectTimeout=10".to_string());
    args.extend(trust.args());
    if profile.ssh_port != DEFAULT_SSH_PORT {
        args.push("-p".to_string());
        args.push(profile.ssh_port.to_string());
    }
    if let Some(key_path) = profile
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("-i".to_string());
        args.push(key_path.to_string());
    }
    if let Some(proxy_jump) = profile
        .proxy_jump
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("-J".to_string());
        args.push(proxy_jump.to_string());
    }
    args.push(destination(profile));
    args.push("exit".to_string());

    let mut cmd = StdCommand::new(ssh);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_ssh_environment(&mut cmd);

    let status = cmd
        .status()
        .map_err(|e| SshError::SpawnFailed(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(SshError::SpawnFailed(format!("ssh exited with {status}")))
    }
}

pub fn test_connection(
    profile: &EffectiveSshTunnel,
    password: Option<&str>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<()> {
    if should_use_system_ssh(password) {
        let trust = resolve_system_host_trust_blocking(profile, interactor.as_ref())?;
        // `trust` (and any Trust-Once pin file) lives until the child exits.
        return test_system_connection(profile, &trust);
    }
    test_russh_connection(profile, password, interactor)
}

fn test_russh_connection(
    profile: &EffectiveSshTunnel,
    password: Option<&str>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<()> {
    let password = password
        .filter(|p| !p.trim().is_empty())
        .map(|p| SecretString::new(p.to_owned()))
        .ok_or(SshError::PasswordRequired)?;
    let profile = profile.clone();
    thread::spawn(move || {
        let runtime = Runtime::new().map_err(|e| SshError::Russh(e.to_string()))?;
        runtime.block_on(async move {
            let config = Arc::new(client::Config::default());
            let addr = format!("{}:{}", profile.ssh_host, profile.ssh_port);
            let mut handle = client::connect(
                config,
                addr,
                RusshClientHandler {
                    ssh_host: profile.ssh_host.clone(),
                    ssh_port: profile.ssh_port,
                    // The test path sends the real password — an unknown key
                    // must go through the trust prompt, same as a real connect.
                    interactor,
                },
            )
            .await
            .map_err(|e| SshError::Russh(format!("failed to connect to SSH server: {e}")))?;

            let authenticated = tokio::time::timeout(
                AUTH_TIMEOUT,
                handle.authenticate_password(&profile.ssh_user, password.as_str()),
            )
            .await
            .map_err(|_| SshError::Russh("SSH password authentication timed out".into()))?
            .map_err(|e| SshError::Russh(format!("SSH password authentication failed: {e}")))?
            .success();
            if authenticated {
                Ok(())
            } else {
                Err(SshError::Russh("SSH password authentication failed".into()))
            }
        })
    })
    .join()
    .map_err(|_| SshError::Russh("SSH test worker panicked".into()))?
}

fn destination(profile: &EffectiveSshTunnel) -> String {
    if profile.ssh_user.trim().is_empty() {
        profile.ssh_host.clone()
    } else {
        format!("{}@{}", profile.ssh_user, profile.ssh_host)
    }
}

fn system_ssh_path() -> Result<std::path::PathBuf> {
    which::which("ssh")
        .or_else(|_| {
            let fallback = std::path::PathBuf::from("/usr/bin/ssh");
            if fallback.exists() {
                Ok(fallback)
            } else {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                which::which_in("ssh", Some(CLI_PATH_SUFFIX), cwd)
            }
        })
        .map_err(|_| SshError::BinaryMissing)
}

fn apply_ssh_environment(cmd: &mut StdCommand) {
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        cmd.env("SSH_AUTH_SOCK", sock);
    }
    let path = match std::env::var("PATH") {
        Ok(existing) if !existing.trim().is_empty() => format!("{existing}:{CLI_PATH_SUFFIX}"),
        _ => CLI_PATH_SUFFIX.to_string(),
    };
    cmd.env("PATH", path);
}

fn shell_quote(arg: &str) -> String {
    if arg.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'@' | b'=')
    }) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{SshAuthKind, SshConnectionId, SshForwardKind, SshTunnelId};

    fn profile() -> EffectiveSshTunnel {
        EffectiveSshTunnel {
            id: SshTunnelId::new("prod-db"),
            connection_id: SshConnectionId::new("bastion"),
            name: "Production DB".into(),
            ssh_host: "bastion.example.com".into(),
            ssh_port: 2222,
            ssh_user: "deploy".into(),
            auth_kind: SshAuthKind::Key,
            key_path: Some("/Users/me/.ssh/id_ed25519".into()),
            local_host: "127.0.0.1".into(),
            local_port: 15432,
            remote_host: "db.internal".into(),
            remote_port: 5432,
            forward_kind: SshForwardKind::Local,
            proxy_jump: Some("jump.example.com".into()),
            keep_alive: true,
            auto_reconnect: false,
        }
    }

    #[test]
    fn system_ssh_is_used_without_password() {
        assert!(should_use_system_ssh(None));
        assert!(should_use_system_ssh(Some("  ")));
        assert!(!should_use_system_ssh(Some("secret")));
    }

    #[test]
    fn host_trust_maps_to_strictness_args() {
        assert_eq!(
            SystemHostTrust::AcceptNew.args(),
            vec!["-o", "StrictHostKeyChecking=accept-new"]
        );
        assert_eq!(
            SystemHostTrust::Strict.args(),
            vec!["-o", "StrictHostKeyChecking=yes"]
        );

        let pin = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let pin_display = pin.display().to_string();
        let args = SystemHostTrust::PinnedOnce(pin).args();
        assert!(args.contains(&"StrictHostKeyChecking=yes".to_string()));
        assert!(args.contains(&format!("UserKnownHostsFile={pin_display}")));
        // The stale ~/.ssh entry must not veto a Trust-Once on a changed key,
        // and the global file must not re-open a hole either.
        assert!(args.contains(&"GlobalKnownHostsFile=/dev/null".to_string()));
    }

    #[test]
    fn display_command_keeps_legacy_accept_new() {
        // `equivalent_ssh_command` (UI display / legacy headless) goes through
        // `ssh_args`, which must stay on accept-new — strictness is decided per
        // spawn by the pre-flight, not baked into the shown command.
        let cmd = equivalent_ssh_command(&profile());
        assert!(cmd.contains("StrictHostKeyChecking=accept-new"));
    }

    #[test]
    fn rejecting_host_key_is_a_dedicated_error() {
        // The UI matches on this message shape to explain a declined prompt.
        let err = SshError::HostKeyRejected("db.example.com".into());
        assert_eq!(
            err.to_string(),
            "the host key for db.example.com was not accepted"
        );
    }

    #[test]
    fn tunnel_key_matches_expected_shape() {
        assert_eq!(
            build_tunnel_key("deploy", "bastion", 22, "db", 5432),
            "deploy@bastion:22:db->5432"
        );
    }

    #[test]
    fn builds_local_forward_command_with_jump_and_key() {
        let command = equivalent_ssh_command(&profile());
        assert!(command.contains("-L 127.0.0.1:15432:db.internal:5432"));
        assert!(command.contains("-J jump.example.com"));
        assert!(command.contains("-p 2222"));
        assert!(command.contains("-i /Users/me/.ssh/id_ed25519"));
        assert!(command.ends_with("deploy@bastion.example.com"));
    }

    #[test]
    fn builds_command_for_ssh_config_host_alias_without_user() {
        let mut p = profile();
        p.ssh_host = "teleport-prod".into();
        p.ssh_user = "".into();
        p.ssh_port = 22;
        p.key_path = None;
        p.proxy_jump = None;

        let command = equivalent_ssh_command(&p);

        assert!(command.ends_with("teleport-prod"));
        assert!(!command.contains("@teleport-prod"));
    }

    #[test]
    fn builds_socks_forward_without_remote_target() {
        let mut p = profile();
        p.forward_kind = SshForwardKind::Socks;
        let command = equivalent_ssh_command(&p);
        assert!(command.contains("-D 127.0.0.1:15432"));
        assert!(!command.contains("-L"));
    }

    /// Pins the P1-2 fix from the 2026-06-10 assessment: a reverse forward
    /// must always name its server-side bind address. Without one, a server
    /// running `GatewayPorts yes` binds the wildcard address and exposes the
    /// forwarded local service to the network.
    #[test]
    fn reverse_forward_always_names_a_bind_address() {
        let mut p = profile();
        p.forward_kind = SshForwardKind::Reverse;
        let command = equivalent_ssh_command(&p);
        assert!(command.contains("-R 127.0.0.1:15432:db.internal:5432"));

        // Pre-existing entries saved before local_host applied to reverse
        // forwards (or with a blank host) still get loopback, never "".
        p.local_host = "  ".into();
        let command = equivalent_ssh_command(&p);
        assert!(command.contains("-R 127.0.0.1:15432:db.internal:5432"));

        // An explicitly saved wide bind (allow_wide_bind opt-in) is honored.
        p.local_host = "0.0.0.0".into();
        let command = equivalent_ssh_command(&p);
        assert!(command.contains("-R 0.0.0.0:15432:db.internal:5432"));
    }
}
