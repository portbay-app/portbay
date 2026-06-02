//! Authenticated russh client sessions to a saved [`SshConnection`].
//!
//! Port-forward tunnels use the system `ssh` binary (key auth) or an in-process
//! russh session (password). The connection-centric capabilities — SFTP today,
//! deploy/shell later — need an **in-process russh session regardless of auth**,
//! because they multiplex their own channels (sftp subsystem, exec, pty) over it.
//! This module is that shared session establishment: connect, verify the host
//! key (TOFU, via [`RusshClientHandler`]), and authenticate through an ordered
//! pipeline (key → agent → password → keyboard-interactive) so agent-only hosts,
//! encrypted keys (decrypted by the agent), and PAM-password-over-KI all work
//! without the user pre-declaring the one exact method.

use std::ops::Deref;
use std::sync::Arc;

use russh::client::{self, KeyboardInteractiveAuthResponse};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::timeout;

use crate::registry::{SshAuthKind, SshConnection};
use crate::ssh::backend::{Result, RusshClientHandler, SshError, AUTH_TIMEOUT};
use crate::ssh::interaction::SshInteractor;
use crate::ssh::proxy;

/// A connected, authenticated russh session handle. Open channels off it
/// (`channel_open_session().request_subsystem("sftp")`, exec, pty…).
pub type SshSessionHandle = client::Handle<RusshClientHandler>;

/// A connected, authenticated session to the **target** host, plus any
/// intermediate jump-host sessions it tunnels through. [`Deref`] exposes the
/// target handle, so callers open channels exactly as they did against a bare
/// [`SshSessionHandle`]. The intermediate handles are held only to keep their
/// russh session tasks (and the direct-tcpip channels deeper hops ride on)
/// alive for the target's lifetime. A direct connection carries an empty chain,
/// so its behaviour is identical to before this type existed.
///
/// Dropping tears the chain down leaf-first: the `handle` field drops before
/// `_chain` (struct field order), and `_chain` is stored deepest-jump-first, so
/// the order is target → deepest jump → … → first jump.
pub struct SshSession {
    handle: SshSessionHandle,
    _chain: Vec<SshSessionHandle>,
}

impl SshSession {
    /// Borrow the target host's session handle.
    pub fn handle(&self) -> &SshSessionHandle {
        &self.handle
    }
}

impl Deref for SshSession {
    type Target = SshSessionHandle;
    fn deref(&self) -> &SshSessionHandle {
        &self.handle
    }
}

/// Default SSH port a jump hop assumes when its spec omits one.
const DEFAULT_SSH_PORT: u16 = 22;

/// One hop in a `ProxyJump` chain. Port defaults to 22; a `None` user is
/// resolved to the connection's user at connect time (OpenSSH behaviour).
#[derive(Debug, Clone, PartialEq, Eq)]
struct JumpHop {
    host: String,
    port: u16,
    user: Option<String>,
}

/// Server `MaxAuthTries` defaults to 6; stay under it so a busy fallback chain
/// never trips the server into dropping us mid-pipeline.
const MAX_AUTH_ATTEMPTS: usize = 5;
/// An agent can hold many keys; only the first few are worth trying before we
/// move on (each is a separate auth request against the attempt budget).
const MAX_AGENT_IDENTITIES: usize = 3;
/// Bound keyboard-interactive prompt rounds so a misbehaving server can't loop
/// us forever; the PAM-password case needs only one.
const MAX_KI_ROUNDS: usize = 6;

/// The auth methods the pipeline can try, in canonical preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AuthMethod {
    Key,
    Agent,
    Password,
    KeyboardInteractive,
}

/// Build the ordered, deduped method list: the connection's preferred
/// `auth_kind` first, then the remaining methods as fallbacks in canonical
/// order. Keyboard-interactive is always last (it's only viable with a
/// password, and password proper is tried before it).
fn auth_method_order(preferred: SshAuthKind) -> Vec<AuthMethod> {
    use AuthMethod::*;
    let preferred = match preferred {
        SshAuthKind::Key => Key,
        SshAuthKind::Agent => Agent,
        SshAuthKind::Password => Password,
    };
    let mut order = Vec::with_capacity(4);
    order.push(preferred);
    for method in [Key, Agent, Password, KeyboardInteractive] {
        if method != preferred {
            order.push(method);
        }
    }
    order
}

/// Connect to `conn` and authenticate, traversing any `ProxyJump` chain and
/// optional forward proxy along the way.
///
/// The destination authenticates through the full pipeline (key → agent →
/// password → keyboard-interactive), stopping at the first success and
/// respecting the server's auth-try budget. Intermediate jump hosts use
/// key/agent only (the connection's local identities), never the saved host
/// password — mirroring OpenSSH. A configured `conn.proxy` (SOCKS5 / HTTP
/// CONNECT) fronts only the **first** transport hop; the rest tunnel through
/// the SSH chain. `proxy_password` is consulted only for an authenticated
/// proxy.
///
/// The direct path (no jump chain, no proxy) is byte-for-byte the behaviour
/// from before this slice — it returns an [`SshSession`] with an empty chain.
pub async fn connect_session(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<SshSession> {
    let user = conn.ssh_user.as_str();
    let password = password.map(str::trim).filter(|p| !p.is_empty());
    // An explicitly-supplied *empty* passphrase means the user chose "Skip" on
    // the passphrase prompt — i.e. this key has no passphrase, so don't ask for
    // one again; fall through to the next auth method (and ultimately the
    // password prompt). Distinguish that from `None` (never asked) before the
    // empty value is filtered away below.
    let passphrase_declined = passphrase == Some("");
    let passphrase = passphrase.filter(|p| !p.is_empty());

    // Dev-only diagnostic (compiled out of release builds): no secret values,
    // only presence/length.
    #[cfg(debug_assertions)]
    tracing::info!(
        host = %conn.ssh_host,
        port = conn.ssh_port,
        user = %user,
        auth_kind = ?conn.auth_kind,
        has_password = password.is_some(),
        password_len = password.map(|p| p.len()).unwrap_or(0),
        has_passphrase = passphrase.is_some(),
        proxy_jump = ?conn.proxy_jump,
        "ssh connect_session: begin auth"
    );

    let jumps = conn
        .proxy_jump
        .as_deref()
        .map(parse_jump_chain)
        .unwrap_or_default();

    // Direct (optionally proxied) path.
    if jumps.is_empty() {
        let handle = connect_target(
            conn,
            password,
            proxy_password,
            passphrase,
            passphrase_declined,
            None,
            interactor,
        )
        .await?;
        return Ok(SshSession {
            handle,
            _chain: Vec::new(),
        });
    }

    // Establish the jump chain hop by hop, authenticating each with key/agent.
    let total = jumps.len();
    let mut chain: Vec<SshSessionHandle> = Vec::with_capacity(total);
    for (i, hop) in jumps.iter().enumerate() {
        let hop_user = hop.user.as_deref().unwrap_or(user);
        let mut handle = if i == 0 {
            // Hop 0 owns the real transport: TCP, or the proxy when configured.
            match &conn.proxy {
                Some(proxy) => {
                    let stream =
                        proxy::connect_via_proxy(proxy, &hop.host, hop.port, proxy_password)
                            .await
                            .map_err(|e| jump_hop_error(i, total, hop, e))?;
                    connect_over_stream(stream, &hop.host, hop.port, interactor.clone())
                        .await
                        .map_err(|e| jump_hop_error(i, total, hop, e))?
                }
                None => connect_over_tcp(&hop.host, hop.port, interactor.clone())
                    .await
                    .map_err(|e| jump_hop_error(i, total, hop, e))?,
            }
        } else {
            // Deeper hops tunnel through the previous hop's session.
            let prev = chain.last().expect("a previous hop exists for i > 0");
            let channel = prev
                .channel_open_direct_tcpip(hop.host.clone(), u32::from(hop.port), "127.0.0.1", 0)
                .await
                .map_err(|e| {
                    jump_hop_error(
                        i,
                        total,
                        hop,
                        SshError::Russh(format!("failed to open tunnel channel: {e}")),
                    )
                })?;
            connect_over_stream(channel.into_stream(), &hop.host, hop.port, interactor.clone())
                .await
                .map_err(|e| jump_hop_error(i, total, hop, e))?
        };

        let ok = authenticate_jump(&mut handle, conn, hop_user, passphrase)
            .await
            .map_err(|e| jump_hop_error(i, total, hop, e))?;
        if !ok {
            return Err(jump_hop_error(
                i,
                total,
                hop,
                SshError::Russh("authentication failed (jump hops accept key/agent only)".into()),
            ));
        }
        chain.push(handle);
    }

    // The destination, tunnelled through the last jump hop, full pipeline.
    let target = {
        let via = chain.last().expect("at least one jump hop");
        connect_target(
            conn,
            password,
            proxy_password,
            passphrase,
            passphrase_declined,
            Some(via),
            interactor,
        )
        .await?
    };

    // Store deepest-jump-first so Drop closes the chain from the leaf inward.
    chain.reverse();
    Ok(SshSession {
        handle: target,
        _chain: chain,
    })
}

/// Tag a hop failure with its 1-based index + address so a mid-chain failure is
/// unambiguous. Strips the inner `SshError::Russh` Display prefix to avoid a
/// doubled "russh tunnel failed:" in the message.
fn jump_hop_error(index: usize, total: usize, hop: &JumpHop, err: SshError) -> SshError {
    let inner = err.to_string();
    let inner = inner
        .strip_prefix("russh tunnel failed: ")
        .unwrap_or(&inner);
    SshError::Russh(format!(
        "jump hop {}/{} ({}:{}): {inner}",
        index + 1,
        total,
        hop.host,
        hop.port
    ))
}

/// Parse an OpenSSH `ProxyJump` spec — a comma-separated chain of
/// `[user@]host[:port]` hops — into ordered [`JumpHop`]s. Blank/whitespace
/// tokens are skipped; a token with an unparseable port is skipped rather than
/// silently mis-dialled.
fn parse_jump_chain(spec: &str) -> Vec<JumpHop> {
    spec.split(',')
        .filter_map(|token| parse_jump_hop(token.trim()))
        .collect()
}

/// Parse a single `[user@]host[:port]` hop. Returns `None` for an empty token,
/// a blank host, or an unparseable port.
fn parse_jump_hop(token: &str) -> Option<JumpHop> {
    if token.is_empty() {
        return None;
    }
    let (user, host_port) = match token.split_once('@') {
        Some((user, rest)) => {
            let user = user.trim();
            let user = (!user.is_empty()).then(|| user.to_string());
            (user, rest.trim())
        }
        None => (None, token),
    };
    if host_port.is_empty() {
        return None;
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => (host.trim(), port.trim().parse::<u16>().ok()?),
        None => (host_port, DEFAULT_SSH_PORT),
    };
    if host.is_empty() {
        return None;
    }
    Some(JumpHop {
        host: host.to_string(),
        port,
        user,
    })
}

/// Connect + fully authenticate the destination host. `via` is the last jump
/// hop's session to tunnel through (`None` = direct/proxied transport). Honors
/// `conn.proxy` only on the direct path; a jumped destination is reached
/// through the SSH chain. Runs the full key → agent → password →
/// keyboard-interactive pipeline, opening a fresh transport for the KI leg
/// exactly as the pre-jump code did.
async fn connect_target(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
    passphrase_declined: bool,
    via: Option<&SshSessionHandle>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<SshSessionHandle> {
    let user = conn.ssh_user.as_str();
    let mut tried: Vec<&'static str> = Vec::new();
    // Set when an encrypted key couldn't be loaded for want of a passphrase —
    // the most actionable failure, so it wins the error decision below.
    let mut needs_passphrase = false;

    // Key / agent / password all multiplex on one connection.
    let mut handle = open_target_handle(conn, proxy_password, via, interactor.clone()).await?;
    if authenticate_multiplexed(
        &mut handle,
        conn,
        password,
        passphrase,
        passphrase_declined,
        &mut tried,
        &mut needs_passphrase,
    )
    .await?
    {
        return Ok(handle);
    }

    // Keyboard-interactive runs on a *fresh* transport. russh 0.43 only wires
    // the server's info-request prompts when KI is the first auth method on a
    // handle, so a fresh connect makes KI the first method. For a jumped target
    // "fresh" means a new direct-tcpip channel on the last hop.
    //
    // Run the leg when we can actually answer: a password to echo (PAM-password-
    // over-KBI) *or* a UI interactor that can collect 2FA/OTP responses. The
    // latter is what lets an OTP-only host authenticate with no saved password.
    if password.is_some() || interactor.is_some() {
        push_unique(&mut tried, "keyboard-interactive");
        let mut ki_handle =
            open_target_handle(conn, proxy_password, via, interactor.clone()).await?;
        if try_keyboard_interactive(
            &mut ki_handle,
            &conn.ssh_host,
            user,
            password,
            interactor.as_ref(),
        )
        .await?
        {
            return Ok(ki_handle);
        }
    }

    // An encrypted key we couldn't open is the clearest thing to ask for.
    if needs_passphrase {
        return Err(SshError::NeedsKeyPassphrase {
            host: conn.ssh_host.clone(),
        });
    }
    // Nothing authenticated us and we never had a password to try. Offer the
    // password prompt as a last resort — regardless of the declared `auth_kind`
    // — mirroring OpenSSH's fall-through to a password prompt. This covers a
    // key/agent host whose key the server refused *and* one with no usable local
    // method at all (no key, no agent), so the user can always fall back to a
    // password instead of hitting a dead end.
    if password.is_none() {
        // Dev-only diagnostic (compiled out of release builds).
        #[cfg(debug_assertions)]
        tracing::info!(
            host = %conn.ssh_host,
            tried = ?tried,
            "ssh connect_target: no password reached the pipeline — returning MissingPassword"
        );
        return Err(SshError::MissingPassword {
            host: conn.ssh_host.clone(),
        });
    }
    // A password (and/or key/agent) was tried and rejected.
    Err(SshError::Russh(format!(
        "SSH authentication failed for {user}@{} (tried {})",
        conn.ssh_host,
        if tried.is_empty() {
            "password".to_string()
        } else {
            tried.join(", ")
        }
    )))
}

/// Open + host-key-verify a transport to the destination (no auth yet): a
/// direct-tcpip channel on `via` when jumping, otherwise TCP or the proxy.
async fn open_target_handle(
    conn: &SshConnection,
    proxy_password: Option<&str>,
    via: Option<&SshSessionHandle>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<SshSessionHandle> {
    match via {
        Some(prev) => {
            let channel = prev
                .channel_open_direct_tcpip(
                    conn.ssh_host.clone(),
                    u32::from(conn.ssh_port),
                    "127.0.0.1",
                    0,
                )
                .await
                .map_err(|e| {
                    SshError::Russh(format!(
                        "failed to open tunnel channel to {}:{}: {e}",
                        conn.ssh_host, conn.ssh_port
                    ))
                })?;
            connect_over_stream(channel.into_stream(), &conn.ssh_host, conn.ssh_port, interactor)
                .await
        }
        None => match &conn.proxy {
            Some(proxy) => {
                let stream =
                    proxy::connect_via_proxy(proxy, &conn.ssh_host, conn.ssh_port, proxy_password)
                        .await?;
                connect_over_stream(stream, &conn.ssh_host, conn.ssh_port, interactor).await
            }
            None => connect_over_tcp(&conn.ssh_host, conn.ssh_port, interactor).await,
        },
    }
}

/// Dial `host:port` over TCP and run the russh client over it (no auth).
async fn connect_over_tcp(
    host: &str,
    port: u16,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<SshSessionHandle> {
    let stream = tokio::net::TcpStream::connect((host, port))
        .await
        .map_err(|e| SshError::Russh(format!("failed to connect to SSH server: {e}")))?;
    connect_over_stream(stream, host, port, interactor).await
}

/// Run a russh client over an already-connected stream (TCP, proxied TCP, or a
/// direct-tcpip channel). The handler carries `host`/`port` so host-key trust is
/// resolved per hop; `interactor` (when present) surfaces an untrusted-key
/// decision to the user, otherwise the handler keeps the legacy silent TOFU.
async fn connect_over_stream<R>(
    stream: R,
    host: &str,
    port: u16,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<SshSessionHandle>
where
    R: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let config = Arc::new(client::Config::default());
    client::connect_stream(
        config,
        stream,
        RusshClientHandler::with_interactor(host.to_string(), port, interactor),
    )
    .await
    .map_err(|e| SshError::Russh(format!("failed to connect to SSH server: {e}")))
}

/// Authenticate an intermediate **jump hop** with local identities only: the
/// connection's key file (when present and loadable) then the SSH agent. Jump
/// hosts never receive the saved host password — that's reserved for the
/// destination (OpenSSH behaviour).
async fn authenticate_jump(
    handle: &mut SshSessionHandle,
    conn: &SshConnection,
    user: &str,
    passphrase: Option<&str>,
) -> Result<bool> {
    let mut attempts = 0usize;
    let mut tried: Vec<&'static str> = Vec::new();

    if let Some(key_path) = conn
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let expanded = expand_tilde(key_path);
        match russh_keys::load_secret_key(&expanded, passphrase) {
            Ok(keypair) => {
                attempts += 1;
                push_unique(&mut tried, "key");
                let ok = timeout(
                    AUTH_TIMEOUT,
                    handle.authenticate_publickey(user, Arc::new(keypair)),
                )
                .await
                .map_err(|_| SshError::Russh("SSH key authentication timed out".into()))?
                .map_err(|e| SshError::Russh(format!("SSH key authentication failed: {e}")))?;
                if ok {
                    return Ok(true);
                }
            }
            Err(e) => {
                tracing::debug!(
                    key = %key_path,
                    error = %e,
                    "skipping jump-hop SSH key (encrypted or unreadable); trying the agent"
                );
            }
        }
    }

    try_agent(handle, user, &mut attempts, &mut tried).await
}

/// Run the key → agent → password legs of the pipeline against an already
/// connected `handle`, in the connection's preferred order. Returns `Ok(true)`
/// on the first success. Keyboard-interactive is handled separately (it needs a
/// fresh handle), so it's skipped here. Records each genuinely-attempted method
/// in `tried` for the caller's aggregated error.
async fn authenticate_multiplexed(
    handle: &mut SshSessionHandle,
    conn: &SshConnection,
    password: Option<&str>,
    passphrase: Option<&str>,
    passphrase_declined: bool,
    tried: &mut Vec<&'static str>,
    needs_passphrase: &mut bool,
) -> Result<bool> {
    let user = conn.ssh_user.as_str();
    let mut attempts = 0usize;

    for method in auth_method_order(conn.auth_kind) {
        if attempts >= MAX_AUTH_ATTEMPTS {
            break;
        }
        match method {
            AuthMethod::Key => {
                let Some(key_path) = conn
                    .key_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let expanded = expand_tilde(key_path);
                let keypair = match russh_keys::load_secret_key(&expanded, passphrase) {
                    Ok(keypair) => keypair,
                    Err(e) => {
                        // Encrypted/passphrase-protected or unreadable key. The
                        // agent commonly holds the decrypted form, so skip to the
                        // next method instead of hard-failing here — but remember
                        // an encrypted key with no passphrase so the caller can
                        // prompt for one if nothing else authenticates.
                        // Skip the passphrase prompt when the user already
                        // declined it (chose "Skip" → this key has no
                        // passphrase / can't be unlocked here), so we fall
                        // through to the password prompt instead of re-asking.
                        if passphrase.is_none()
                            && !passphrase_declined
                            && matches!(e, russh_keys::Error::KeyIsEncrypted)
                        {
                            *needs_passphrase = true;
                        }
                        // A key that won't load is the usual cause of an
                        // otherwise-inexplicable auth dead-end (wrong passphrase,
                        // or a key format/cipher russh 0.43 can't parse), so make
                        // it visible rather than burying it at debug.
                        tracing::warn!(
                            key = %key_path,
                            passphrase_supplied = passphrase.is_some(),
                            error = %e,
                            "SSH key could not be loaded; skipping to the next auth method"
                        );
                        continue;
                    }
                };
                attempts += 1;
                push_unique(tried, "key");
                let ok = timeout(
                    AUTH_TIMEOUT,
                    handle.authenticate_publickey(user, Arc::new(keypair)),
                )
                .await
                .map_err(|_| SshError::Russh("SSH key authentication timed out".into()))?
                .map_err(|e| SshError::Russh(format!("SSH key authentication failed: {e}")))?;
                if ok {
                    return Ok(true);
                }
            }
            AuthMethod::Agent => {
                if try_agent(handle, user, &mut attempts, tried).await? {
                    return Ok(true);
                }
            }
            AuthMethod::Password => {
                let Some(password) = password else {
                    continue;
                };
                attempts += 1;
                push_unique(tried, "password");
                let ok = timeout(AUTH_TIMEOUT, handle.authenticate_password(user, password))
                    .await
                    .map_err(|_| SshError::Russh("SSH password authentication timed out".into()))?
                    .map_err(|e| {
                        SshError::Russh(format!("SSH password authentication failed: {e}"))
                    })?;
                tracing::info!(accepted = ok, "ssh password auth attempt finished");
                if ok {
                    return Ok(true);
                }
            }
            // Handled on a fresh handle by the caller — see `connect_session`.
            AuthMethod::KeyboardInteractive => continue,
        }
    }

    Ok(false)
}

/// Try each agent identity (capped) against `handle`. Skips silently when no
/// agent is reachable or it holds no keys. Returns `Ok(true)` on the first
/// identity the server accepts.
async fn try_agent(
    handle: &mut SshSessionHandle,
    user: &str,
    attempts: &mut usize,
    tried: &mut Vec<&'static str>,
) -> Result<bool> {
    let mut agent = match russh_keys::agent::client::AgentClient::connect_env().await {
        Ok(agent) => agent,
        Err(e) => {
            tracing::debug!(error = %e, "SSH agent unavailable; skipping agent auth");
            return Ok(false);
        }
    };
    let identities = match agent.request_identities().await {
        Ok(identities) => identities,
        Err(e) => {
            tracing::debug!(error = %e, "couldn't list SSH agent identities; skipping agent auth");
            return Ok(false);
        }
    };
    if identities.is_empty() {
        return Ok(false);
    }

    for key in identities.into_iter().take(MAX_AGENT_IDENTITIES) {
        if *attempts >= MAX_AUTH_ATTEMPTS {
            break;
        }
        *attempts += 1;
        push_unique(tried, "agent");

        // `authenticate_future` consumes the signer and hands it back, so thread
        // it through the loop to reuse one agent connection across identities.
        let attempt = timeout(AUTH_TIMEOUT, handle.authenticate_future(user, key, agent)).await;
        let (returned_agent, result) = match attempt {
            Ok(pair) => pair,
            Err(_) => {
                tracing::debug!("SSH agent authentication timed out");
                return Ok(false);
            }
        };
        agent = returned_agent;
        match result {
            Ok(true) => return Ok(true),
            Ok(false) => continue,
            Err(e) => {
                tracing::debug!(error = %e, "SSH agent signing failed; stopping agent auth");
                break;
            }
        }
    }
    Ok(false)
}

/// Run a keyboard-interactive exchange. Each server info-request is answered by
/// [`resolve_kbi_answers`]: a lone password-looking prompt is auto-filled from
/// `password` (the silent PAM-password-over-KBI case), anything else (OTP, 2FA,
/// multi-field) is surfaced to the user through `interactor`. Returns `Ok(true)`
/// on acceptance, `Ok(false)` on rejection or when the user cancels.
async fn try_keyboard_interactive(
    handle: &mut SshSessionHandle,
    host: &str,
    user: &str,
    password: Option<&str>,
    interactor: Option<&Arc<dyn SshInteractor>>,
) -> Result<bool> {
    let mut response = timeout(
        AUTH_TIMEOUT,
        handle.authenticate_keyboard_interactive_start(user, None),
    )
    .await
    .map_err(|_| SshError::Russh("SSH keyboard-interactive authentication timed out".into()))?
    .map_err(|e| {
        SshError::Russh(format!(
            "SSH keyboard-interactive authentication failed: {e}"
        ))
    })?;

    for _ in 0..MAX_KI_ROUNDS {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let Some(answers) =
                    resolve_kbi_answers(host, name, instructions, &prompts, password, interactor)
                        .await
                else {
                    // No way to answer (no password, no UI) or the user cancelled.
                    return Ok(false);
                };
                response = timeout(
                    AUTH_TIMEOUT,
                    handle.authenticate_keyboard_interactive_respond(answers),
                )
                .await
                .map_err(|_| {
                    SshError::Russh("SSH keyboard-interactive authentication timed out".into())
                })?
                .map_err(|e| {
                    SshError::Russh(format!(
                        "SSH keyboard-interactive authentication failed: {e}"
                    ))
                })?;
            }
        }
    }
    Ok(false)
}

/// Decide how to answer one keyboard-interactive info-request.
///
/// 1. A single hidden, password-looking prompt with a password in hand →
///    auto-answer silently (the ubiquitous PAM-password-over-KBI case; no UI).
/// 2. Otherwise, with a UI interactor → surface every field to the user (OTP,
///    2FA, "Duo passcode", multi-field), returning their answers or `None` if
///    they cancel.
/// 3. Otherwise (headless) with a password → echo it into every field, the
///    legacy fallback. With nothing to offer → `None`.
async fn resolve_kbi_answers(
    host: &str,
    name: String,
    instructions: String,
    prompts: &[client::Prompt],
    password: Option<&str>,
    interactor: Option<&Arc<dyn SshInteractor>>,
) -> Option<Vec<String>> {
    if prompts.len() == 1 && !prompts[0].echo && prompt_looks_like_password(&prompts[0].prompt) {
        if let Some(pw) = password {
            return Some(vec![pw.to_string()]);
        }
    }

    if let Some(interactor) = interactor {
        let prompt = crate::ssh::interaction::KbiPrompt {
            host: host.to_string(),
            name,
            instructions,
            prompts: prompts
                .iter()
                .map(|p| crate::ssh::interaction::KbiField {
                    prompt: p.prompt.clone(),
                    echo: p.echo,
                })
                .collect(),
        };
        return interactor.kbi_responses(prompt).await;
    }

    password.map(|pw| prompts.iter().map(|_| pw.to_string()).collect())
}

/// Does this prompt look like a request for the account password (as opposed to
/// an OTP/2FA token)? Used to keep the silent password-over-KBI fast path.
fn prompt_looks_like_password(prompt: &str) -> bool {
    let lowered = prompt.to_lowercase();
    lowered.contains("password") || prompt.contains("密码")
}

fn push_unique(list: &mut Vec<&'static str>, item: &'static str) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// Expand a leading `~/` to `$HOME/` so `load_secret_key` opens the right file.
fn expand_tilde(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var("HOME") {
            Ok(home) => format!("{home}/{rest}"),
            Err(_) => path.to_string(),
        },
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_puts_preferred_first_then_canonical_fallbacks() {
        use AuthMethod::*;
        assert_eq!(
            auth_method_order(SshAuthKind::Key),
            vec![Key, Agent, Password, KeyboardInteractive]
        );
        assert_eq!(
            auth_method_order(SshAuthKind::Password),
            vec![Password, Key, Agent, KeyboardInteractive]
        );
        assert_eq!(
            auth_method_order(SshAuthKind::Agent),
            vec![Agent, Key, Password, KeyboardInteractive]
        );
    }

    #[test]
    fn order_is_complete_and_deduped() {
        for kind in [SshAuthKind::Key, SshAuthKind::Password, SshAuthKind::Agent] {
            let order = auth_method_order(kind);
            assert_eq!(order.len(), 4, "every method appears exactly once");
            let mut sorted = order.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), 4, "no method is duplicated");
        }
    }

    #[test]
    fn push_unique_skips_repeats() {
        let mut list = Vec::new();
        push_unique(&mut list, "key");
        push_unique(&mut list, "agent");
        push_unique(&mut list, "key");
        assert_eq!(list, vec!["key", "agent"]);
    }

    #[test]
    fn parse_jump_chain_handles_single_multi_and_userport() {
        // Single bare host → default port, inherited user.
        assert_eq!(
            parse_jump_chain("bastion"),
            vec![JumpHop {
                host: "bastion".into(),
                port: 22,
                user: None
            }]
        );

        // Comma chain with mixed user@host:port forms.
        assert_eq!(
            parse_jump_chain("alice@a:2222, b , root@c"),
            vec![
                JumpHop {
                    host: "a".into(),
                    port: 2222,
                    user: Some("alice".into())
                },
                JumpHop {
                    host: "b".into(),
                    port: 22,
                    user: None
                },
                JumpHop {
                    host: "c".into(),
                    port: 22,
                    user: Some("root".into())
                },
            ]
        );
    }

    #[test]
    fn parse_jump_chain_skips_blank_and_bad_port_tokens() {
        assert_eq!(parse_jump_chain(""), vec![]);
        assert_eq!(parse_jump_chain("   ,  ,"), vec![]);
        // A non-numeric port skips just that hop, keeping the good one.
        assert_eq!(
            parse_jump_chain("good, bad:notaport"),
            vec![JumpHop {
                host: "good".into(),
                port: 22,
                user: None
            }]
        );
        // Out-of-range port (>65535) is unparseable as u16 → skipped.
        assert_eq!(parse_jump_chain("h:99999"), vec![]);
    }

    #[test]
    fn expand_tilde_resolves_home_and_passes_absolute_paths() {
        // Read HOME rather than set it — mutating a process-global env var here
        // races sibling lib tests (cargo runs them on shared threads).
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(
                expand_tilde("~/.ssh/id_ed25519"),
                format!("{home}/.ssh/id_ed25519")
            );
        }
        assert_eq!(expand_tilde("/etc/ssh/key"), "/etc/ssh/key");
        assert_eq!(expand_tilde("relative/key"), "relative/key");
    }
}
