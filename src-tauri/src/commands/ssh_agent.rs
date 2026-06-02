//! Server-side AI agent commands for the SSH workspace.
//!
//! `ssh_agent_open` connects (synchronously, so auth gaps hit the credential
//! prompt), caches the session, and probes the host for a model runtime.
//! `ssh_agent_chat` relays one turn to the host's model and streams the reply
//! over a `Channel<AgentEvent>`. `ssh_agent_run` executes one **user-approved**
//! command on the same cached session. The agent loop, the system prompt, and
//! the approval gate all live on the frontend — the backend never runs a
//! command the user hasn't approved.

use russh::ChannelMsg;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel as IpcChannel;
use tauri::State;

use crate::commands::projects::load_registry;
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::{SshConnection, SshConnectionId};
use crate::ssh::agent::{detect, open_chat_channel, run_command, AgentInfo, DEFAULT_OLLAMA_PORT};
use crate::ssh::exec::ExecResult;
use crate::state::AppState;

/// One chat message relayed to the host's model (ollama `/api/chat` shape).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// A streamed event from one chat turn.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    /// An incremental chunk of the assistant's reply.
    Token { text: String },
    /// The turn finished; `content` is the full assistant reply.
    Done { content: String },
    /// The model request failed (e.g. ollama not running, curl error).
    Error { message: String },
}

/// Resolve the effective connection from the registry.
fn resolve_conn(state: &State<'_, AppState>, id: &str) -> AppResult<SshConnection> {
    let registry = load_registry(state)?;
    let raw = registry
        .get_ssh_connection(&SshConnectionId::new(id))
        .ok_or_else(|| AppError::BadInput(format!("SSH connection `{id}` not found")))?;
    Ok(registry.effective_ssh_connection(raw))
}

/// Connect (prompting for a credential when needed), cache the session, and
/// probe the host for a model runtime. Returns what the host offers so the UI
/// can pick a model or show the dispatch fallback.
#[tauri::command]
pub async fn ssh_agent_open(
    state: State<'_, AppState>,
    connection_id: String,
    password: Option<String>,
    passphrase: Option<String>,
) -> AppResult<AgentInfo> {
    let conn = resolve_conn(&state, &connection_id)?;
    let nonblank = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let password = match nonblank(password) {
        Some(p) => Some(p),
        None => load_stored_password(&conn.id)?,
    };
    let passphrase = match nonblank(passphrase) {
        Some(p) => Some(p),
        None => load_stored_key_passphrase(&conn.id)?,
    };
    let proxy_password = load_stored_proxy_password(&conn.id)?;

    let session = {
        let mut mgr = state.agent.lock().await;
        mgr.session_for(
            &conn,
            password.as_deref(),
            proxy_password.as_deref(),
            passphrase.as_deref(),
        )
        .await
        .map_err(AppError::Ssh)?
    };

    detect(&session, DEFAULT_OLLAMA_PORT)
        .await
        .map_err(AppError::Ssh)
}

/// Relay one chat turn to the host's model and stream the reply. Requires a
/// session already opened by [`ssh_agent_open`] (reused, no re-prompt).
#[tauri::command]
pub async fn ssh_agent_chat(
    state: State<'_, AppState>,
    connection_id: String,
    model: String,
    messages: Vec<ChatMessage>,
    port: Option<u16>,
    on_event: IpcChannel<AgentEvent>,
) -> AppResult<()> {
    let conn = resolve_conn(&state, &connection_id)?;
    let session = {
        let mut mgr = state.agent.lock().await;
        // Reuse the cached session (opened by ssh_agent_open). No secrets here:
        // a cache miss means the session dropped — surface that rather than
        // silently re-prompting mid-chat.
        mgr.session_for(&conn, None, None, None)
            .await
            .map_err(AppError::Ssh)?
    };

    let messages_value = serde_json::to_value(&messages)
        .map_err(|e| AppError::Internal(format!("couldn't encode messages: {e}")))?;
    let mut channel = open_chat_channel(
        &session,
        &model,
        &messages_value,
        port.unwrap_or(DEFAULT_OLLAMA_PORT),
    )
    .await
    .map_err(AppError::Ssh)?;

    // ollama streams newline-delimited JSON; accumulate partial lines across
    // chunks, emit each chunk's content token, finish on `done` / channel close.
    let mut buf = String::new();
    let mut stderr = String::new();
    let mut content = String::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { data } => {
                buf.push_str(&String::from_utf8_lossy(&data));
                while let Some(nl) = buf.find('\n') {
                    let line = buf[..nl].trim().to_string();
                    buf.drain(..=nl);
                    if line.is_empty() {
                        continue;
                    }
                    if let Some(token) = parse_chat_line(&line) {
                        if !token.is_empty() {
                            content.push_str(&token);
                            let _ = on_event.send(AgentEvent::Token { text: token });
                        }
                    }
                }
            }
            ChannelMsg::ExtendedData { data, .. } => {
                stderr.push_str(&String::from_utf8_lossy(&data));
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    // Flush any trailing partial line.
    if let Some(token) = parse_chat_line(buf.trim()) {
        if !token.is_empty() {
            content.push_str(&token);
            let _ = on_event.send(AgentEvent::Token { text: token });
        }
    }

    if content.is_empty() && !stderr.trim().is_empty() {
        let _ = on_event.send(AgentEvent::Error {
            message: format!("Model request failed: {}", stderr.trim()),
        });
    } else {
        let _ = on_event.send(AgentEvent::Done { content });
    }
    Ok(())
}

/// Extract the assistant content chunk from one ollama `/api/chat` NDJSON line.
fn parse_chat_line(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::to_owned)
}

/// Run one **user-approved** command on the agent's cached session.
#[tauri::command]
pub async fn ssh_agent_run(
    state: State<'_, AppState>,
    connection_id: String,
    command: String,
) -> AppResult<ExecResult> {
    let conn = resolve_conn(&state, &connection_id)?;
    let session = {
        let mut mgr = state.agent.lock().await;
        mgr.session_for(&conn, None, None, None)
            .await
            .map_err(AppError::Ssh)?
    };
    run_command(&session, &command).await.map_err(AppError::Ssh)
}

/// Drop the agent's cached session for a connection.
#[tauri::command]
pub async fn ssh_agent_close(state: State<'_, AppState>, connection_id: String) -> AppResult<()> {
    state.agent.lock().await.disconnect(&connection_id);
    Ok(())
}
