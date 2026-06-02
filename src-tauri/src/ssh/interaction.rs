//! Backend-initiated SSH user interaction.
//!
//! Some auth steps can't be answered from stored config: a first-contact or
//! changed host key needs an explicit trust decision, and (Phase 2)
//! keyboard-interactive auth needs the user to answer server prompts. Each of
//! those pauses a *live* handshake, asks the frontend, and resumes with the
//! answer — the opposite of the reactive password/passphrase flow, which fails
//! with an `SSH_NEEDS_*` code and lets the frontend retry.
//!
//! The mechanism: the connect path holds an [`SshInteractor`]. When it needs a
//! decision it emits a Tauri event carrying a unique `flow_id`, parks on a
//! oneshot registered under that id, and the frontend posts the answer back via
//! the [`ssh_interaction_respond`] / [`ssh_interaction_cancel`] commands. A
//! [`NoopInteractor`] (or a `None` interactor) preserves the old silent-TOFU
//! behaviour for headless callers (the MCP agent, tunnels).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

use crate::error::AppResult;

/// Frontend event carrying a host-key trust decision request.
pub const HOSTKEY_PROMPT_EVENT: &str = "portbay://ssh-hostkey-prompt";

/// Frontend event carrying a keyboard-interactive challenge (2FA / OTP).
pub const KBI_PROMPT_EVENT: &str = "portbay://ssh-kbi-prompt";

/// Hold a handshake open this long waiting for the user before treating silence
/// as a cancel, so a forgotten dialog can't pin a connection (and its server
/// socket) open indefinitely.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Why the connect is asking: a never-seen host key, or one that differs from
/// the recorded one. Mirrors the read-only probe's vocabulary so the UI speaks
/// one language across the dashboard and the connect prompt.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKeyState {
    /// Not in `known_hosts` — trust-on-first-use decision.
    New,
    /// Present in `known_hosts` but the key differs — possible MITM.
    Changed,
}

/// Emitted to the frontend when a connect reaches an untrusted host key. The
/// `flowId` (added by [`PromptEnvelope`]) correlates the eventual
/// [`ssh_interaction_respond`] back to the parked handshake.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub state: HostKeyState,
    /// Server key algorithm, e.g. `ssh-ed25519`.
    pub key_type: String,
    /// `SHA256:…` fingerprint of the key the server presented.
    pub fingerprint: String,
    /// The previously-trusted key's fingerprint, when we can determine it
    /// (changed-key case). `None` keeps the UI to a "key no longer matches"
    /// message rather than a false comparison.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_fingerprint: Option<String>,
}

/// One field of a keyboard-interactive challenge.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbiField {
    /// The server's prompt text, e.g. `Verification code:`.
    pub prompt: String,
    /// Whether typed characters should be echoed (false for passwords/OTPs).
    pub echo: bool,
}

/// Emitted to the frontend when a server's keyboard-interactive auth needs the
/// user (2FA / OTP / any non-password PAM conversation).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KbiPrompt {
    pub host: String,
    /// Server-provided title (often empty).
    pub name: String,
    /// Server-provided instructions (often empty).
    pub instructions: String,
    /// One or more fields the user must answer, in order.
    pub prompts: Vec<KbiField>,
}

/// The user's answer to a [`HostKeyPrompt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyDecision {
    /// Trust for this session only — do not write `known_hosts`.
    TrustOnce,
    /// Trust and persist to `known_hosts` (for a changed key, replace the old).
    TrustAndSave,
    /// Refuse — abort the connect.
    Reject,
}

/// Asked by the connect path when it can't proceed without the user. Trait so
/// the live UI path ([`EventInteractor`]) and headless callers
/// ([`NoopInteractor`]) share one call site.
#[async_trait]
pub trait SshInteractor: Send + Sync {
    /// Decide how to treat an untrusted host key.
    async fn host_key_decision(&self, prompt: HostKeyPrompt) -> HostKeyDecision;

    /// Answer a keyboard-interactive challenge. `Some(answers)` (one per field,
    /// in order) continues auth; `None` cancels the keyboard-interactive leg.
    async fn kbi_responses(&self, prompt: KbiPrompt) -> Option<Vec<String>>;
}

/// Headless interactor: never prompts. Preserves the legacy behaviour — silent
/// TOFU for a new key (the caller learns it), reject for a changed key — and
/// can't answer an interactive challenge.
pub struct NoopInteractor;

#[async_trait]
impl SshInteractor for NoopInteractor {
    async fn host_key_decision(&self, prompt: HostKeyPrompt) -> HostKeyDecision {
        match prompt.state {
            HostKeyState::New => HostKeyDecision::TrustAndSave,
            HostKeyState::Changed => HostKeyDecision::Reject,
        }
    }

    async fn kbi_responses(&self, _prompt: KbiPrompt) -> Option<Vec<String>> {
        None
    }
}

/// One pending interaction's reply channel, keyed by `flow_id`.
static PENDING: LazyLock<Mutex<HashMap<String, oneshot::Sender<InteractionReply>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Monotonic source of `flow_id`s. (`Math.random`/clock-free so it stays
/// deterministic in tests and replay.)
static FLOW_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_flow_id() -> String {
    format!("ssh-flow-{}", FLOW_SEQ.fetch_add(1, Ordering::Relaxed))
}

/// The frontend's posted answer. `action` is the host-key choice; `responses`
/// is reserved for Phase 2 keyboard-interactive.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionReply {
    pub action: String,
    #[serde(default)]
    pub responses: Option<Vec<String>>,
}

/// Live interactor: emits a Tauri event and awaits the frontend's answer.
pub struct EventInteractor {
    app: AppHandle,
}

impl EventInteractor {
    /// Build a UI-driven interactor from a command's `AppHandle`.
    pub fn new(app: AppHandle) -> std::sync::Arc<dyn SshInteractor> {
        std::sync::Arc::new(Self { app })
    }

    /// Emit `event` with `payload` under a fresh `flow_id`, then park until the
    /// frontend answers (via `ssh_interaction_respond`), the flow is cancelled
    /// (`ssh_interaction_cancel` drops the sender), or [`PROMPT_TIMEOUT`]
    /// elapses. `None` means "no answer" — every caller fails closed on it.
    async fn ask<P: Serialize>(&self, event: &str, payload: &P) -> Option<InteractionReply> {
        let flow_id = next_flow_id();
        let (tx, rx) = oneshot::channel();
        PENDING
            .lock()
            .expect("interaction registry poisoned")
            .insert(flow_id.clone(), tx);

        if self.app.emit(event, &PromptEnvelope { flow_id: &flow_id, inner: payload }).is_err() {
            PENDING.lock().ok().and_then(|mut p| p.remove(&flow_id));
            return None;
        }

        match tokio::time::timeout(PROMPT_TIMEOUT, rx).await {
            Ok(Ok(reply)) => Some(reply),
            // Timed out, or the sender was dropped (cancel).
            _ => {
                PENDING.lock().ok().and_then(|mut p| p.remove(&flow_id));
                None
            }
        }
    }
}

/// Wraps a typed payload with the `flowId` the frontend echoes back, so each
/// prompt struct doesn't have to carry (and we don't have to mutate) its own.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptEnvelope<'a, P: Serialize> {
    flow_id: &'a str,
    #[serde(flatten)]
    inner: &'a P,
}

#[async_trait]
impl SshInteractor for EventInteractor {
    async fn host_key_decision(&self, prompt: HostKeyPrompt) -> HostKeyDecision {
        match self.ask(HOSTKEY_PROMPT_EVENT, &prompt).await {
            Some(reply) => match reply.action.as_str() {
                "trust_save" => HostKeyDecision::TrustAndSave,
                "trust_once" => HostKeyDecision::TrustOnce,
                _ => HostKeyDecision::Reject,
            },
            None => HostKeyDecision::Reject,
        }
    }

    async fn kbi_responses(&self, prompt: KbiPrompt) -> Option<Vec<String>> {
        let reply = self.ask(KBI_PROMPT_EVENT, &prompt).await?;
        match reply.action.as_str() {
            "submit" => reply.responses,
            _ => None,
        }
    }
}

/// Post the user's answer back to the parked handshake. No-op if the flow
/// already timed out or was answered (idempotent).
#[tauri::command]
pub fn ssh_interaction_respond(
    flow_id: String,
    action: String,
    responses: Option<Vec<String>>,
) -> AppResult<()> {
    if let Some(tx) = PENDING
        .lock()
        .expect("interaction registry poisoned")
        .remove(&flow_id)
    {
        let _ = tx.send(InteractionReply { action, responses });
    }
    Ok(())
}

/// Cancel a pending interaction (dialog dismissed). Dropping the sender makes
/// the parked handshake observe a closed channel and fail closed.
#[tauri::command]
pub fn ssh_interaction_cancel(flow_id: String) -> AppResult<()> {
    PENDING
        .lock()
        .expect("interaction registry poisoned")
        .remove(&flow_id);
    Ok(())
}
