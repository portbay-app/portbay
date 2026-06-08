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
use tauri::{AppHandle, State};

use crate::commands::projects::load_registry;
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::{SshConnection, SshConnectionId};
use crate::ssh::agent::{
    cleanup_attachment_turn, cleanup_attachments, detect, ollama_generate, open_agent_cli_channel,
    open_chat_channel, run_command, sweep_stale_attachments, upload_attachment, wrap_in_cwd,
    AgentInfo, CliProvider, DEFAULT_OLLAMA_PORT, MAX_ATTACHMENT_BYTES,
};
use crate::ssh::exec::ExecResult;
use crate::ssh::interaction::EventInteractor;
use crate::state::AppState;

/// One chat message relayed to the host's model (ollama `/api/chat` shape).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// A streamed event from one chat turn. The ollama path emits only
/// `token`/`done`/`error`; the official agent CLIs (Claude Code / Codex) also
/// emit `session` (for `--resume` threading) and `toolUse`/`toolResult` so we
/// can render the provider's own tool activity verbatim.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    /// An incremental chunk of the assistant's reply.
    Token { text: String },
    /// An incremental chunk of the agent's reasoning/thinking (Claude
    /// `thinking_delta`, Codex `reasoning` items). Only emitted when the agent
    /// actually surfaces its thinking; many turns emit none. Parsed from the
    /// stream we already read — no extra tokens.
    Reasoning { text: String },
    /// The CLI agent reported its session id (Claude `system/init`). The
    /// frontend keeps it to continue the conversation with `--resume`.
    Session { id: String },
    /// The agent started one of its own tools (read-only mirror, not a gate).
    ToolUse { name: String, summary: String },
    /// One of the agent's tools returned.
    ToolResult { summary: String, is_error: bool },
    /// The agent published or updated its own task list (Claude `TodoWrite`,
    /// Codex `todo_list`). Carries the full current list; the frontend replaces
    /// any prior list wholesale. Free: it's parsed from the stream we already read.
    Todos { items: Vec<TodoItem> },
    /// The turn finished; `content` is the full assistant reply.
    Done { content: String },
    /// The request failed (e.g. ollama not running, or the agent CLI errored).
    /// `auth` is `true` when the failure looks like the host CLI isn't signed in,
    /// so the frontend can offer an in-app sign-in CTA instead of a bare error.
    Error { message: String, auth: bool },
}

/// One entry in an agent-authored task list. `status` is `pending`,
/// `in_progress`, or `completed` (Codex only distinguishes done/not-done, which
/// maps to `completed`/`pending`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub text: String,
    pub status: String,
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
    app: AppHandle,
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
            Some(EventInteractor::shared(app)),
        )
        .await
        .map_err(AppError::Ssh)?
    };

    // Sweep attachment staging a previous app run left behind (crash, dropped
    // network — the paths where per-turn/on-close cleanup never ran). In the
    // background so it can't delay the open/probe.
    {
        let session = session.clone();
        tauri::async_runtime::spawn(async move {
            sweep_stale_attachments(&session).await;
        });
    }

    detect(&session, DEFAULT_OLLAMA_PORT)
        .await
        .map_err(AppError::Ssh)
}

/// Relay one chat turn to the host's model and stream the reply. Requires a
/// session already opened by [`ssh_agent_open`] (reused, no re-prompt).
#[tauri::command]
pub async fn ssh_agent_chat(
    app: AppHandle,
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
        mgr.session_for(&conn, None, None, None, Some(EventInteractor::shared(app)))
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

    // Register an abort handle so a Stop press can end this turn mid-stream.
    let abort = {
        let mut mgr = state.agent.lock().await;
        mgr.register_abort(&connection_id)
    };

    // ollama streams newline-delimited JSON; accumulate partial lines across
    // chunks, emit each chunk's content token, finish on `done` / channel close.
    let mut buf = String::new();
    let mut stderr = String::new();
    let mut content = String::new();
    let mut aborted = false;
    let notified = abort.notified();
    tokio::pin!(notified);
    loop {
        tokio::select! {
            maybe = channel.wait() => {
                let Some(msg) = maybe else { break };
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
            _ = &mut notified => {
                aborted = true;
                break;
            }
        }
    }
    {
        let mut mgr = state.agent.lock().await;
        mgr.clear_abort(&connection_id);
    }

    // User pressed Stop: close the channel and commit the partial reply.
    if aborted {
        let _ = channel.close().await;
        let _ = on_event.send(AgentEvent::Done { content });
        return Ok(());
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
            auth: false,
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

/// Drive the host's official agent CLI (Claude Code / Codex) for one turn and
/// stream its own event stream back. Requires a session already opened by
/// [`ssh_agent_open`]. `resume_id` threads a Claude conversation across turns;
/// `permission_mode` is Claude's official `--permission-mode`. We render the
/// provider's output verbatim and never inject prompts or gate its tools.
// Tauri command params are flat IPC args (not a struct), so the count is inherent.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ssh_agent_cli_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    provider: String,
    prompt: String,
    permission_mode: Option<String>,
    resume_id: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    on_event: IpcChannel<AgentEvent>,
) -> AppResult<()> {
    let cli_provider = CliProvider::parse(&provider)
        .ok_or_else(|| AppError::BadInput(format!("`{provider}` is not a CLI agent provider")))?;
    let conn = resolve_conn(&state, &connection_id)?;
    let session = {
        let mut mgr = state.agent.lock().await;
        mgr.session_for(&conn, None, None, None, Some(EventInteractor::shared(app)))
            .await
            .map_err(AppError::Ssh)?
    };

    let mut channel = open_agent_cli_channel(
        &session,
        cli_provider,
        &prompt,
        permission_mode.as_deref(),
        resume_id.as_deref(),
        model.as_deref(),
        cwd.as_deref(),
    )
    .await
    .map_err(AppError::Ssh)?;

    // Register an abort handle so `ssh_agent_abort` can stop this turn; the
    // select loop below races the stream against it.
    let abort = {
        let mut mgr = state.agent.lock().await;
        mgr.register_abort(&connection_id)
    };

    let mut buf = String::new();
    let mut stderr = String::new();
    let mut content = String::new();
    let mut terminated = false;
    let mut aborted = false;
    let forward = |provider, line: &str, content: &mut String| {
        let (events, term) = parse_cli_line(provider, line, content);
        for ev in events {
            let _ = on_event.send(ev);
        }
        term
    };
    let notified = abort.notified();
    tokio::pin!(notified);
    loop {
        tokio::select! {
            maybe = channel.wait() => {
                let Some(msg) = maybe else { break };
                match msg {
                    ChannelMsg::Data { data } => {
                        buf.push_str(&String::from_utf8_lossy(&data));
                        while let Some(nl) = buf.find('\n') {
                            let line = buf[..nl].to_string();
                            buf.drain(..=nl);
                            terminated |= forward(cli_provider, line.trim(), &mut content);
                        }
                    }
                    ChannelMsg::ExtendedData { data, .. } => {
                        stderr.push_str(&String::from_utf8_lossy(&data));
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            _ = &mut notified => {
                aborted = true;
                break;
            }
        }
    }
    {
        let mut mgr = state.agent.lock().await;
        mgr.clear_abort(&connection_id);
    }

    // User pressed Stop: close the channel (so the remote CLI process gets EOF
    // and exits) and commit whatever streamed so far as the turn — an abort is
    // not a failure, even if nothing arrived yet.
    if aborted {
        let _ = channel.close().await;
        if !terminated {
            let _ = on_event.send(AgentEvent::Done { content });
        }
        return Ok(());
    }

    // Flush any trailing partial line (e.g. Codex plain text without a newline).
    terminated |= forward(cli_provider, buf.trim(), &mut content);

    // The CLI didn't emit its own terminal `result` — synthesize one so the
    // frontend's turn always resolves. Empty output ⇒ surface the real stderr
    // (the actual cause) with a sign-in tip appended only if it looks like auth.
    if !terminated {
        if content.trim().is_empty() {
            let (message, auth) = cli_failure_hint(cli_provider, stderr.trim());
            let _ = on_event.send(AgentEvent::Error { message, auth });
        } else {
            let _ = on_event.send(AgentEvent::Done { content });
        }
    }
    Ok(())
}

/// Build a failure message for a CLI run, plus whether it looks like an auth
/// (not-signed-in) failure. We surface the agent's **real** error text verbatim
/// (so a version/flag/other error isn't mis-reported as "not signed in"), and
/// only *append* a sign-in tip when the text actually looks like an auth failure.
/// The `bool` lets the UI offer an in-app sign-in CTA only when it's warranted.
/// Keys never touch PortBay — auth lives on the host.
fn cli_failure_hint(provider: CliProvider, detail: &str) -> (String, bool) {
    let detail = detail.trim();
    let name = match provider {
        CliProvider::Claude => "Claude Code",
        CliProvider::Codex => "Codex",
    };
    let tip = match provider {
        // `claude auth login` creates the subscription *session* credential that
        // `claude -p` reads. NOT `setup-token` (that only mints a long-lived token
        // for SDK/API use and does not authenticate the CLI). No bare `claude login`.
        CliProvider::Claude => "sign in on the host: run `claude auth login` (works over SSH)",
        CliProvider::Codex => "sign in on the host: run `codex login`",
    };
    let lower = detail.to_lowercase();
    let looks_unauth = lower.contains("login")
        || lower.contains("log in") // "please log in again"
        || lower.contains("sign in")
        || lower.contains("auth")
        || lower.contains("api key")
        || lower.contains("unauthorized")
        || lower.contains("credential")
        // Expired/invalid OAuth session — Codex's "Failed to refresh token …
        // Your session has ended" (a single-use refresh token that got
        // invalidated). Routing these to the sign-in CTA lets the user re-auth
        // in place instead of hitting a dead-end generic error.
        || lower.contains("refresh token")
        || lower.contains("session has ended")
        || lower.contains("session expired")
        || lower.contains("token has expired");

    if detail.is_empty() {
        // No output / no stated reason — most often a headless session with no
        // usable creds, and the CLI gave no actionable text. Treat as auth-suspect
        // so the UI can offer in-app sign-in rather than a dead-end error.
        (
            format!("{name} couldn't complete the run. If it isn't signed in here, {tip}."),
            true,
        )
    } else if looks_unauth {
        (format!("{name} couldn't run: {detail} — {tip}."), true)
    } else {
        (format!("{name} couldn't run: {detail}"), false)
    }
}

/// Parse one line of a CLI agent's output into events. The `bool` is `true` when
/// the line is a terminal event (Claude `result`) — the returned vec already
/// holds the `done`/`error`, so the caller must not synthesize another.
fn parse_cli_line(
    provider: CliProvider,
    line: &str,
    content: &mut String,
) -> (Vec<AgentEvent>, bool) {
    if line.is_empty() {
        return (Vec::new(), false);
    }
    match provider {
        CliProvider::Claude => parse_claude_line(line, content),
        CliProvider::Codex => parse_codex_line(line, content),
    }
}

/// Map one line of `codex exec --json` to events. Codex emits a JSONL event
/// stream: `thread.started` (carries the thread id for resume), `turn.started`,
/// per-`item` events, then a terminal `turn.completed`/`turn.failed`. The
/// assistant's reply arrives whole on the `item.completed` for an `agent_message`
/// (codex doesn't stream it token-by-token), so the turn isn't incremental.
/// Anything that isn't recognisable JSON falls back to verbatim text so we never
/// silently swallow output from an unexpected build.
fn parse_codex_line(line: &str, content: &mut String) -> (Vec<AgentEvent>, bool) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        // Plain text (or a non-JSON edge build): render it as-is.
        content.push_str(line);
        content.push('\n');
        return (
            vec![AgentEvent::Token {
                text: format!("{line}\n"),
            }],
            false,
        );
    };
    let mut out = Vec::new();
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        // Thread id → frontend keeps it to continue the conversation on resume.
        "thread.started" => {
            if let Some(id) = v.get("thread_id").and_then(|s| s.as_str()) {
                out.push(AgentEvent::Session { id: id.to_string() });
            }
        }
        "item.started" | "item.updated" | "item.completed" => {
            let completed = kind == "item.completed";
            if let Some(item) = v.get("item") {
                let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match item_type {
                    // The assistant's answer: emitted whole, on completion only.
                    "agent_message" if completed => {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            content.push_str(text);
                            out.push(AgentEvent::Token {
                                text: text.to_string(),
                            });
                        }
                    }
                    // Tool activity → read-only mirror chips (same as Claude).
                    "command_execution" => {
                        let cmd = item.get("command").and_then(|c| c.as_str()).unwrap_or("");
                        if completed {
                            let is_error = item
                                .get("exit_code")
                                .and_then(|c| c.as_i64())
                                .is_some_and(|c| c != 0)
                                || item.get("status").and_then(|s| s.as_str()) == Some("failed");
                            out.push(AgentEvent::ToolResult {
                                summary: codex_truncate(cmd),
                                is_error,
                            });
                        } else {
                            out.push(AgentEvent::ToolUse {
                                name: "shell".to_string(),
                                summary: codex_truncate(cmd),
                            });
                        }
                    }
                    "file_change" if completed => {
                        let n = item
                            .get("changes")
                            .and_then(|c| c.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        out.push(AgentEvent::ToolResult {
                            summary: format!("{n} file change{}", if n == 1 { "" } else { "s" }),
                            is_error: false,
                        });
                    }
                    "mcp_tool_call" => {
                        let server = item.get("server").and_then(|s| s.as_str()).unwrap_or("");
                        let tool = item.get("tool").and_then(|t| t.as_str()).unwrap_or("tool");
                        let name = if server.is_empty() {
                            tool.to_string()
                        } else {
                            format!("{server}/{tool}")
                        };
                        if completed {
                            let is_error = item.get("error").map(|e| !e.is_null()).unwrap_or(false);
                            out.push(AgentEvent::ToolResult {
                                summary: name,
                                is_error,
                            });
                        } else {
                            out.push(AgentEvent::ToolUse {
                                name,
                                summary: String::new(),
                            });
                        }
                    }
                    "web_search" if completed => {
                        let query = item.get("query").and_then(|q| q.as_str()).unwrap_or("");
                        out.push(AgentEvent::ToolUse {
                            name: "web_search".to_string(),
                            summary: codex_truncate(query),
                        });
                    }
                    // Reasoning summary — emitted whole on completion (only when
                    // the agent surfaces its thinking). Append a newline so
                    // multiple reasoning items read as separate paragraphs.
                    "reasoning" if completed => {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            out.push(AgentEvent::Reasoning {
                                text: format!("{text}\n"),
                            });
                        }
                    }
                    // The agent's plan — surfaced on every update so it animates
                    // live. Codex items only carry a `completed` bool (no
                    // in-progress state), so they map to completed/pending.
                    "todo_list" => {
                        let items = item
                            .get("items")
                            .and_then(|i| i.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|t| {
                                        let text = t.get("text").and_then(|s| s.as_str())?;
                                        let done = t
                                            .get("completed")
                                            .and_then(|b| b.as_bool())
                                            .unwrap_or(false);
                                        Some(TodoItem {
                                            text: text.to_string(),
                                            status: if done { "completed" } else { "pending" }
                                                .to_string(),
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        out.push(AgentEvent::Todos { items });
                    }
                    // item-level errors / other types: not surfaced.
                    _ => {}
                }
            }
        }
        "turn.completed" => {
            out.push(AgentEvent::Done {
                content: std::mem::take(content),
            });
            return (out, true);
        }
        "turn.failed" => {
            let detail = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            let (message, auth) = cli_failure_hint(CliProvider::Codex, detail);
            out.push(AgentEvent::Error { message, auth });
            return (out, true);
        }
        // A top-level `error` is non-fatal (e.g. a reconnection notice): surface
        // it but don't end the turn — a later `turn.completed` still resolves it,
        // and the frontend drops the error once real content arrives.
        "error" => {
            if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                let (message, auth) = cli_failure_hint(CliProvider::Codex, msg);
                out.push(AgentEvent::Error { message, auth });
            }
        }
        _ => {}
    }
    (out, false)
}

/// Shorten a codex command/query for a one-line tool chip.
fn codex_truncate(s: &str) -> String {
    let s = s.trim();
    let one_line = s.replace('\n', " ");
    if one_line.chars().count() > 80 {
        let cut: String = one_line.chars().take(79).collect();
        format!("{cut}…")
    } else {
        one_line
    }
}

/// Parse the todo list out of a Claude `TodoWrite` tool input. The shape is
/// `{ todos: [{ content, status, activeForm }] }`; `status` is one of
/// `pending`/`in_progress`/`completed`. A malformed input yields an empty list.
fn parse_claude_todos(input: Option<&serde_json::Value>) -> Vec<TodoItem> {
    let Some(todos) = input
        .and_then(|i| i.get("todos"))
        .and_then(|t| t.as_array())
    else {
        return Vec::new();
    };
    todos
        .iter()
        .filter_map(|t| {
            let text = t.get("content").and_then(|c| c.as_str())?;
            let status = t
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("pending");
            Some(TodoItem {
                text: text.to_string(),
                status: status.to_string(),
            })
        })
        .collect()
}

/// Map one Claude Code `stream-json` line to events. Text comes only from
/// `stream_event` deltas (so we don't double-render the final `assistant`
/// message); tool activity comes from `assistant`/`user` content blocks; the
/// session id from `system/init`; the turn ends on `result`.
fn parse_claude_line(line: &str, content: &mut String) -> (Vec<AgentEvent>, bool) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return (Vec::new(), false);
    };
    let mut out = Vec::new();
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        "system" => {
            let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
            if subtype == "init" {
                if let Some(id) = v.get("session_id").and_then(|s| s.as_str()) {
                    out.push(AgentEvent::Session { id: id.to_string() });
                }
            } else if subtype == "api_retry" {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    if err.contains("auth") {
                        let (message, auth) = cli_failure_hint(CliProvider::Claude, err);
                        out.push(AgentEvent::Error { message, auth });
                        return (out, true);
                    }
                }
            }
        }
        "stream_event" => {
            // Streaming deltas: event.delta is either a text_delta (the reply) or
            // a thinking_delta (extended-thinking reasoning, when enabled).
            if let Some(delta) = v.get("event").and_then(|e| e.get("delta")) {
                match delta.get("type").and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                            content.push_str(text);
                            out.push(AgentEvent::Token {
                                text: text.to_string(),
                            });
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                            out.push(AgentEvent::Reasoning {
                                text: text.to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "assistant" => {
            // Pull tool_use blocks (text is already streamed via stream_event).
            for block in message_content_blocks(&v) {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                    // `TodoWrite` is the agent's own plan — surface it as a todo
                    // list rather than a generic tool chip.
                    if name == "TodoWrite" {
                        out.push(AgentEvent::Todos {
                            items: parse_claude_todos(block.get("input")),
                        });
                        continue;
                    }
                    out.push(AgentEvent::ToolUse {
                        name: name.to_string(),
                        summary: tool_input_summary(block.get("input")),
                    });
                }
            }
        }
        "user" => {
            for block in message_content_blocks(&v) {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    let is_error = block
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);
                    out.push(AgentEvent::ToolResult {
                        summary: tool_result_summary(block.get("content")),
                        is_error,
                    });
                }
            }
        }
        "result" => {
            let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false)
                || v.get("subtype")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s != "success");
            if is_error {
                // No `result`/`error` text means the CLI failed without stating a
                // reason — pass empty so it's treated as auth-suspect (the common
                // cause is "not signed in"), surfacing the in-app sign-in CTA.
                let detail = v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .or_else(|| v.get("error").and_then(|e| e.as_str()))
                    .unwrap_or("");
                let (message, auth) = cli_failure_hint(CliProvider::Claude, detail);
                out.push(AgentEvent::Error { message, auth });
            } else {
                out.push(AgentEvent::Done {
                    content: std::mem::take(content),
                });
            }
            return (out, true);
        }
        _ => {}
    }
    (out, false)
}

/// The `message.content` array of a Claude `assistant`/`user` stream event, or
/// an empty slice when absent/malformed.
fn message_content_blocks(v: &serde_json::Value) -> &[serde_json::Value] {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// A compact one-line summary of a tool_use `input` object for the activity
/// chip (e.g. a file path or command), capped so the UI stays tidy.
fn tool_input_summary(input: Option<&serde_json::Value>) -> String {
    let Some(obj) = input.and_then(|i| i.as_object()) else {
        return String::new();
    };
    // Prefer the fields that read well in a chip; fall back to the first string.
    let pick = [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "description",
    ]
    .iter()
    .find_map(|k| obj.get(*k).and_then(|x| x.as_str()))
    .or_else(|| obj.values().find_map(|x| x.as_str()))
    .unwrap_or("");
    truncate(pick, 120)
}

/// A short summary of a tool_result `content` (string, or array of text blocks).
fn tool_result_summary(content: Option<&serde_json::Value>) -> String {
    let text = match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    truncate(text.trim(), 160)
}

/// Truncate to `max` chars on a char boundary, appending an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// One-shot inline code completion against the host's ollama `/api/generate`
/// (FIM via the native `suffix` field). Shares the warm agent session and
/// returns the raw completion text — the frontend's completion engine does the
/// caching, debouncing, cancellation, and post-processing. `port` defaults to
/// ollama's loopback default; `num_predict` caps the output length (small for a
/// snappy inline hint).
// Tauri command params are flat IPC args (not a struct), so the count is inherent.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ssh_ollama_complete(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    model: String,
    prefix: String,
    suffix: String,
    port: Option<u16>,
    num_predict: Option<u32>,
) -> AppResult<String> {
    let conn = resolve_conn(&state, &connection_id)?;
    let session = {
        let mut mgr = state.agent.lock().await;
        mgr.session_for(&conn, None, None, None, Some(EventInteractor::shared(app)))
            .await
            .map_err(AppError::Ssh)?
    };
    ollama_generate(
        &session,
        &model,
        &prefix,
        &suffix,
        port.unwrap_or(DEFAULT_OLLAMA_PORT),
        num_predict.unwrap_or(64),
    )
    .await
    .map_err(AppError::Ssh)
}

/// Run one **user-approved** command on the agent's cached session.
#[tauri::command]
pub async fn ssh_agent_run(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    command: String,
    cwd: Option<String>,
) -> AppResult<ExecResult> {
    let conn = resolve_conn(&state, &connection_id)?;
    let session = {
        let mut mgr = state.agent.lock().await;
        mgr.session_for(&conn, None, None, None, Some(EventInteractor::shared(app)))
            .await
            .map_err(AppError::Ssh)?
    };
    // Approved commands run in the agent's working directory too, so an ollama
    // model's proposed `cp …` / `mv …` lands in the project like a local shell.
    let command = wrap_in_cwd(&command, cwd.as_deref());
    run_command(&session, &command).await.map_err(AppError::Ssh)
}

/// Upload one chat attachment to the host (over the cached agent session) and
/// return its absolute remote path. `data_base64` is the file's bytes — this is
/// the path for clipboard-pasted images, which have no local file path. The host
/// path is then referenced in the turn so the official CLI reads it with its own
/// tools (no credential re-prompt; no content processing in PortBay).
#[tauri::command]
pub async fn ssh_agent_upload_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    turn_id: String,
    name: String,
    data_base64: String,
) -> AppResult<String> {
    use base64::Engine as _;
    // Reject oversized payloads before decoding: base64 is ~4/3 of the raw
    // size, so anything past this bound can't fit the attachment cap — don't
    // materialise it in memory just to fail the check in `upload_attachment`.
    if data_base64.len() > MAX_ATTACHMENT_BYTES / 3 * 4 + 4 {
        return Err(AppError::BadInput(format!(
            "attachment is larger than the {} MiB limit",
            MAX_ATTACHMENT_BYTES / (1024 * 1024)
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|e| AppError::BadInput(format!("attachment was not valid base64: {e}")))?;
    let conn = resolve_conn(&state, &connection_id)?;
    let session = {
        let mut mgr = state.agent.lock().await;
        mgr.session_for(&conn, None, None, None, Some(EventInteractor::shared(app)))
            .await
            .map_err(AppError::Ssh)?
    };
    upload_attachment(&session, &turn_id, &name, &bytes)
        .await
        .map_err(AppError::Ssh)
}

/// Upload a local file (chosen via the picker or OS drag-drop) to the host and
/// return its absolute remote path. Reading the local file here mirrors
/// `sftp_upload`, so the frontend never needs a filesystem plugin.
///
/// The renderer-supplied `local_path` is validated against the host-approved
/// set (`sftp_pick_upload_files` picks and OS drag-drops are the only sources
/// that populate it) before any read, so the renderer cannot exfiltrate
/// arbitrary files by passing a crafted path. We read from the canonical path
/// the check returns, not the raw string.
#[tauri::command]
pub async fn ssh_agent_upload_path(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    turn_id: String,
    name: String,
    local_path: String,
) -> AppResult<String> {
    let local = crate::commands::sftp::ensure_local_path_approved(&state, &local_path)?;
    let bytes = std::fs::read(&local)
        .map_err(|e| AppError::BadInput(format!("couldn't read `{}`: {e}", local.display())))?;
    let conn = resolve_conn(&state, &connection_id)?;
    let session = {
        let mut mgr = state.agent.lock().await;
        mgr.session_for(&conn, None, None, None, Some(EventInteractor::shared(app)))
            .await
            .map_err(AppError::Ssh)?
    };
    upload_attachment(&session, &turn_id, &name, &bytes)
        .await
        .map_err(AppError::Ssh)
}

/// Delete one turn's remote attachment staging directory after the agent has
/// consumed it. Best-effort: a stale screenshot should not break the chat turn,
/// and a missing cached session just means there is nothing warm to clean.
#[tauri::command]
pub async fn ssh_agent_cleanup_attachments(
    state: State<'_, AppState>,
    connection_id: String,
    turn_id: String,
) -> AppResult<()> {
    let session = state.agent.lock().await.peek(&connection_id);
    if let Some(session) = session {
        cleanup_attachment_turn(&session, &turn_id).await;
    }
    Ok(())
}

/// Start an ephemeral local→remote port forward over the agent's session. Used by
/// the codex sign-in: `codex login` runs its OAuth callback server on a loopback
/// port of the *host* (1455), so forwarding the user's local port to it lets the
/// browser redirect to `http://localhost:1455/...` complete. Torn down by
/// [`ssh_agent_forward_stop`] (and on close).
#[tauri::command]
pub async fn ssh_agent_forward_start(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    local_port: u16,
    remote_port: u16,
) -> AppResult<()> {
    let conn = resolve_conn(&state, &connection_id)?;
    let mut mgr = state.agent.lock().await;
    let session = mgr
        .session_for(&conn, None, None, None, Some(EventInteractor::shared(app)))
        .await
        .map_err(AppError::Ssh)?;
    mgr.start_forward(&connection_id, session, local_port, remote_port)
        .await
        .map_err(AppError::Ssh)
}

/// Stop the ephemeral sign-in port forward for a connection (best-effort).
#[tauri::command]
pub async fn ssh_agent_forward_stop(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<()> {
    state.agent.lock().await.stop_forward(&connection_id);
    Ok(())
}

/// Stop the in-flight chat turn for a connection (the Stop button / Escape).
/// Best-effort: notifies the streaming loop, which closes the channel so the
/// remote model/CLI process exits and commits whatever streamed so far. A no-op
/// if no turn is running.
#[tauri::command]
pub async fn ssh_agent_abort(state: State<'_, AppState>, connection_id: String) -> AppResult<()> {
    state.agent.lock().await.abort(&connection_id);
    Ok(())
}

/// Drop the agent's cached session for a connection. Best-effort: first clear any
/// attachments staged on the host this session, while the session is still warm.
#[tauri::command]
pub async fn ssh_agent_close(state: State<'_, AppState>, connection_id: String) -> AppResult<()> {
    let session = state.agent.lock().await.peek(&connection_id);
    if let Some(session) = session {
        cleanup_attachments(&session).await;
    }
    state.agent.lock().await.disconnect(&connection_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(line: &str, content: &mut String) -> (Vec<AgentEvent>, bool) {
        parse_cli_line(CliProvider::Claude, line, content)
    }

    #[test]
    fn claude_init_yields_session_id() {
        let mut c = String::new();
        let (ev, term) = claude(
            r#"{"type":"system","subtype":"init","session_id":"abc-123"}"#,
            &mut c,
        );
        assert!(!term);
        assert_eq!(
            ev,
            vec![AgentEvent::Session {
                id: "abc-123".into()
            }]
        );
    }

    #[test]
    fn claude_text_delta_streams_token_and_accumulates() {
        let mut c = String::new();
        let line =
            r#"{"type":"stream_event","event":{"delta":{"type":"text_delta","text":"Hello "}}}"#;
        let (ev, _) = claude(line, &mut c);
        assert_eq!(
            ev,
            vec![AgentEvent::Token {
                text: "Hello ".into()
            }]
        );
        assert_eq!(c, "Hello ");
    }

    #[test]
    fn claude_non_text_delta_is_ignored() {
        let mut c = String::new();
        let line = r#"{"type":"stream_event","event":{"delta":{"type":"input_json_delta","partial_json":"{"}}}"#;
        let (ev, term) = claude(line, &mut c);
        assert!(ev.is_empty() && !term && c.is_empty());
    }

    #[test]
    fn claude_assistant_block_yields_tool_use_with_summary() {
        let mut c = String::new();
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ignored"},{"type":"tool_use","name":"Edit","input":{"file_path":"app/logger.py"}}]}}"#;
        let (ev, _) = claude(line, &mut c);
        assert_eq!(
            ev,
            vec![AgentEvent::ToolUse {
                name: "Edit".into(),
                summary: "app/logger.py".into()
            }]
        );
    }

    #[test]
    fn claude_user_block_yields_tool_result() {
        let mut c = String::new();
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#;
        let (ev, _) = claude(line, &mut c);
        assert_eq!(
            ev,
            vec![AgentEvent::ToolResult {
                summary: "boom".into(),
                is_error: true
            }]
        );
    }

    #[test]
    fn claude_result_success_is_terminal_done() {
        let mut c = String::from("streamed reply");
        let (ev, term) = claude(
            r#"{"type":"result","subtype":"success","is_error":false}"#,
            &mut c,
        );
        assert!(term);
        assert_eq!(
            ev,
            vec![AgentEvent::Done {
                content: "streamed reply".into()
            }]
        );
        assert!(c.is_empty(), "content is taken into the Done event");
    }

    #[test]
    fn claude_result_error_is_terminal_error() {
        let mut c = String::new();
        let (ev, term) = claude(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#,
            &mut c,
        );
        assert!(term);
        assert!(matches!(ev.as_slice(), [AgentEvent::Error { .. }]));
    }

    #[test]
    fn claude_result_error_without_detail_offers_sign_in() {
        // The real not-signed-in case that gives no `result`/`error` text: it must
        // still flag `auth` so the UI shows the in-app sign-in CTA (regression for
        // the "the agent run did not complete" dead-end).
        let mut c = String::new();
        let (ev, term) = claude(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#,
            &mut c,
        );
        assert!(term);
        match ev.as_slice() {
            [AgentEvent::Error { message, auth }] => {
                assert!(*auth, "a detail-less failure offers sign-in: {message}");
                assert!(message.contains("auth login"), "carries the tip: {message}");
            }
            other => panic!("expected an Error, got {other:?}"),
        }
    }

    #[test]
    fn claude_not_logged_in_result_surfaces_real_text_and_tip() {
        // The real shape an unauthenticated `claude -p` emits: result with
        // subtype "success" but is_error true and the actual message.
        let mut c = String::new();
        let line = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#;
        let (ev, term) = claude(line, &mut c);
        assert!(term);
        match ev.as_slice() {
            [AgentEvent::Error { message, auth }] => {
                assert!(
                    message.contains("Not logged in"),
                    "shows the real text: {message}"
                );
                assert!(
                    message.contains("auth login"),
                    "appends the headless sign-in tip: {message}"
                );
                assert!(*auth, "an auth failure flags the sign-in CTA");
            }
            other => panic!("expected a single Error, got {other:?}"),
        }
    }

    #[test]
    fn claude_auth_retry_is_terminal_error_with_hint() {
        let mut c = String::new();
        let (ev, term) = claude(
            r#"{"type":"system","subtype":"api_retry","error":"authentication_failed"}"#,
            &mut c,
        );
        assert!(term);
        match ev.as_slice() {
            [AgentEvent::Error { message, auth }] => {
                assert!(message.contains("auth login"));
                assert!(*auth, "api_retry auth failure flags the sign-in CTA");
            }
            other => panic!("expected an auth Error, got {other:?}"),
        }
    }

    #[test]
    fn claude_garbage_line_is_ignored() {
        let mut c = String::new();
        let (ev, term) = claude("not json at all", &mut c);
        assert!(ev.is_empty() && !term);
    }

    fn codex(line: &str, content: &mut String) -> (Vec<AgentEvent>, bool) {
        parse_cli_line(CliProvider::Codex, line, content)
    }

    #[test]
    fn codex_non_json_line_falls_back_to_verbatim_token() {
        let mut c = String::new();
        let (ev, term) = codex("working on it", &mut c);
        assert!(!term);
        assert_eq!(
            ev,
            vec![AgentEvent::Token {
                text: "working on it\n".into()
            }]
        );
    }

    #[test]
    fn codex_thread_started_yields_session_id() {
        let mut c = String::new();
        let (ev, term) = codex(
            r#"{"type":"thread.started","thread_id":"019e8b82-3783-7c51"}"#,
            &mut c,
        );
        assert!(!term);
        assert_eq!(
            ev,
            vec![AgentEvent::Session {
                id: "019e8b82-3783-7c51".into()
            }]
        );
    }

    #[test]
    fn codex_lifecycle_events_are_ignored() {
        let mut c = String::new();
        let (ev, term) = codex(r#"{"type":"turn.started"}"#, &mut c);
        assert!(ev.is_empty() && !term && c.is_empty());
    }

    #[test]
    fn codex_agent_message_completed_streams_text_and_accumulates() {
        let mut c = String::new();
        let line = r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Done. Updated the docs."}}"#;
        let (ev, term) = codex(line, &mut c);
        assert!(!term);
        assert_eq!(
            ev,
            vec![AgentEvent::Token {
                text: "Done. Updated the docs.".into()
            }]
        );
        assert_eq!(c, "Done. Updated the docs.");
    }

    #[test]
    fn codex_agent_message_only_emits_on_completed() {
        // started/updated for an agent_message carry no final text — ignore them
        // so we don't double-render when the completed event arrives.
        let mut c = String::new();
        let (ev, _) = codex(
            r#"{"type":"item.started","item":{"id":"i1","type":"agent_message"}}"#,
            &mut c,
        );
        assert!(ev.is_empty() && c.is_empty());
    }

    #[test]
    fn codex_command_execution_yields_tool_use_then_result() {
        let mut c = String::new();
        let (started, _) = codex(
            r#"{"type":"item.started","item":{"id":"c1","type":"command_execution","command":"ls -la"}}"#,
            &mut c,
        );
        assert_eq!(
            started,
            vec![AgentEvent::ToolUse {
                name: "shell".into(),
                summary: "ls -la".into()
            }]
        );
        let (done, _) = codex(
            r#"{"type":"item.completed","item":{"id":"c1","type":"command_execution","command":"ls -la","exit_code":2,"status":"failed"}}"#,
            &mut c,
        );
        assert_eq!(
            done,
            vec![AgentEvent::ToolResult {
                summary: "ls -la".into(),
                is_error: true
            }]
        );
    }

    #[test]
    fn codex_turn_completed_is_terminal_done() {
        let mut c = String::from("the answer");
        let (ev, term) = codex(
            r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":5}}"#,
            &mut c,
        );
        assert!(term);
        assert_eq!(
            ev,
            vec![AgentEvent::Done {
                content: "the answer".into()
            }]
        );
        assert!(c.is_empty(), "content is taken into the Done event");
    }

    #[test]
    fn codex_turn_failed_is_terminal_error_with_real_text() {
        let mut c = String::new();
        let line = r#"{"type":"turn.failed","error":{"message":"Not logged in. Run codex login"}}"#;
        let (ev, term) = codex(line, &mut c);
        assert!(term);
        match ev.as_slice() {
            [AgentEvent::Error { message, auth }] => {
                assert!(message.contains("Not logged in"), "real text: {message}");
                assert!(
                    message.contains("codex login"),
                    "carries the tip: {message}"
                );
                assert!(*auth, "an auth failure flags the sign-in CTA");
            }
            other => panic!("expected an auth Error, got {other:?}"),
        }
    }

    #[test]
    fn claude_thinking_delta_yields_reasoning_not_content() {
        let mut c = String::new();
        let line = r#"{"type":"stream_event","event":{"delta":{"type":"thinking_delta","thinking":"Let me check the config."}}}"#;
        let (ev, _) = claude(line, &mut c);
        assert_eq!(
            ev,
            vec![AgentEvent::Reasoning {
                text: "Let me check the config.".into()
            }]
        );
        assert!(c.is_empty(), "reasoning never lands in the reply content");
    }

    #[test]
    fn codex_reasoning_item_yields_reasoning() {
        let mut c = String::new();
        let line = r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"Planning the edits"}}"#;
        let (ev, _) = codex(line, &mut c);
        assert_eq!(
            ev,
            vec![AgentEvent::Reasoning {
                text: "Planning the edits\n".into()
            }]
        );
        assert!(c.is_empty());
    }

    #[test]
    fn claude_todowrite_yields_todos_not_a_tool_chip() {
        let mut c = String::new();
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"Wire the parser","status":"completed","activeForm":"Wiring"},{"content":"Render the panel","status":"in_progress","activeForm":"Rendering"}]}}]}}"#;
        let (ev, term) = claude(line, &mut c);
        assert!(!term);
        assert_eq!(
            ev,
            vec![AgentEvent::Todos {
                items: vec![
                    TodoItem {
                        text: "Wire the parser".into(),
                        status: "completed".into()
                    },
                    TodoItem {
                        text: "Render the panel".into(),
                        status: "in_progress".into()
                    },
                ]
            }]
        );
    }

    #[test]
    fn codex_todo_list_yields_todos_on_update() {
        let mut c = String::new();
        let line = r#"{"type":"item.updated","item":{"id":"t1","type":"todo_list","items":[{"text":"Step one","completed":true},{"text":"Step two","completed":false}]}}"#;
        let (ev, term) = codex(line, &mut c);
        assert!(!term);
        assert_eq!(
            ev,
            vec![AgentEvent::Todos {
                items: vec![
                    TodoItem {
                        text: "Step one".into(),
                        status: "completed".into()
                    },
                    TodoItem {
                        text: "Step two".into(),
                        status: "pending".into()
                    },
                ]
            }]
        );
    }

    #[test]
    fn auth_hint_flags_codex_expired_session_for_resign_in() {
        // The real "logged in but token died" case (openai/codex#9634/#23647):
        // it must offer the sign-in CTA, not a dead-end generic error.
        let (msg, auth) = cli_failure_hint(
            CliProvider::Codex,
            "Failed to refresh token: 400 Bad Request: Your session has ended. Please log in again.",
        );
        assert!(auth, "expired session is treated as auth: {msg}");
        assert!(
            msg.contains("codex login"),
            "carries the re-login tip: {msg}"
        );
    }

    #[test]
    fn codex_top_level_error_is_non_terminal() {
        // A reconnection notice must not end the turn — a later turn.completed
        // still resolves it.
        let mut c = String::new();
        let (ev, term) = codex(
            r#"{"type":"error","message":"stream disconnected, retrying"}"#,
            &mut c,
        );
        assert!(!term, "top-level error is non-fatal");
        assert!(matches!(ev.as_slice(), [AgentEvent::Error { .. }]));
    }

    #[test]
    fn provider_parse_maps_ids() {
        assert_eq!(CliProvider::parse("claude"), Some(CliProvider::Claude));
        assert_eq!(CliProvider::parse("codex"), Some(CliProvider::Codex));
        assert_eq!(CliProvider::parse("ollama"), None);
    }

    #[test]
    fn tool_input_summary_prefers_readable_fields_and_truncates() {
        let v = serde_json::json!({ "command": "ls -la /var/log" });
        assert_eq!(tool_input_summary(Some(&v)), "ls -la /var/log");
        let long = serde_json::json!({ "path": "x".repeat(200) });
        assert!(tool_input_summary(Some(&long)).ends_with('…'));
        assert!(tool_input_summary(None).is_empty());
    }

    #[test]
    fn auth_hint_detects_unauthenticated_stderr() {
        let (msg, auth) = cli_failure_hint(
            CliProvider::Claude,
            "Error: Not logged in. Run claude login",
        );
        assert!(msg.contains("claude login"));
        assert!(auth, "an auth-looking failure is flagged");
        let (generic, generic_auth) = cli_failure_hint(CliProvider::Claude, "segfault");
        assert!(generic.contains("segfault") && !generic.contains("login"));
        assert!(!generic_auth, "a non-auth failure is not flagged");
    }

    #[test]
    fn empty_output_is_auth_suspect() {
        // No output at all is treated as a likely not-signed-in headless session.
        let (msg, auth) = cli_failure_hint(CliProvider::Codex, "");
        assert!(msg.contains("codex login"));
        assert!(auth);
    }
}
