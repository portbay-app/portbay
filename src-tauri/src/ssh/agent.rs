//! Server-side AI agent transport.
//!
//! The agent's *brain* runs on the remote host: we talk to the host's own model
//! runtime (ollama / OpenAI-compatible on `localhost`) by running `curl` over an
//! exec channel on the cached SSH session. Inference and command execution both
//! happen on the box — only the rendered chat crosses the (encrypted) SSH wire,
//! no API keys live in PortBay, and nothing reaches a third-party cloud. When
//! the host has no model, the UI falls back to dispatching the user's own agent.
//!
//! [`AgentManager`] caches one authenticated session per connection (like
//! [`SftpManager`](crate::ssh::SftpManager)) so detection, each chat turn, and
//! approved-command execution all reuse one warm connection — one credential
//! prompt, low latency.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client::Msg;
use russh::Channel;
use russh::ChannelMsg;
use serde::Serialize;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::registry::SshConnection;
use crate::ssh::backend::{Result, SshError};
use crate::ssh::exec::{exec_on, ExecResult};
use crate::ssh::interaction::SshInteractor;
use crate::ssh::session::{connect_session, SshSession};

/// Default ollama port; the host's model API is assumed on loopback here.
pub const DEFAULT_OLLAMA_PORT: u16 = 11434;

/// What model tooling the host offers — drives whether the Agent tab runs a
/// server-side chat or shows the dispatch fallback.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    /// `curl` is present (the streaming HTTP client for the model API).
    pub has_curl: bool,
    /// `wget` is present (the non-streaming fallback when `curl` is absent).
    pub has_wget: bool,
    /// The `ollama` CLI is present (lets us list models even without curl/wget).
    pub has_ollama: bool,
    /// Simon Willison's `llm` CLI is present (informational; a later backend).
    pub has_llm: bool,
    /// The `claude` CLI (Claude Code) is on the host's PATH. Driving it is the
    /// sibling task; here it only flips the provider switcher's Claude option on.
    pub has_claude: bool,
    /// The `codex` CLI is on the host's PATH. Same: detection only, for now.
    pub has_codex: bool,
    /// Models discovered on the host — via ollama's `/api/tags`, falling back to
    /// `ollama list`. Empty = no server model found.
    pub ollama_models: Vec<String>,
    /// Loopback port the model API answered on (resolved from `OLLAMA_HOST`,
    /// else the default). Chat requests target this port.
    pub port: u16,
}

/// A live ephemeral local→remote port forward (e.g. for the codex sign-in's
/// localhost OAuth callback). Stopping aborts the accept loop and frees the port.
pub struct ForwardHandle {
    running: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl ForwardHandle {
    fn stop(self) {
        self.running.store(false, Ordering::Relaxed);
        self.task.abort();
    }
}

/// A live agent session plus when it was last handed out, for idle reaping.
struct CachedAgent {
    session: Arc<SshSession>,
    last_used: Instant,
}

/// One cached, authenticated session backing the agent for a connection, plus any
/// ephemeral port forward opened for its sign-in flow.
#[derive(Default)]
pub struct AgentManager {
    sessions: HashMap<String, CachedAgent>,
    forwards: HashMap<String, ForwardHandle>,
    /// One abort handle per connection for the in-flight chat turn, if any. The
    /// turn registers a [`Notify`] at the start and clears it at the end;
    /// [`ssh_agent_abort`](crate::commands::ssh_agent::ssh_agent_abort) notifies
    /// it so the streaming loop stops and the remote CLI process is dropped.
    aborts: HashMap<String, Arc<Notify>>,
}

impl AgentManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// A live session for `conn`, reusing the cached one while its handle is
    /// open, otherwise (re)connecting. `password`/`passphrase` are only consulted
    /// when a new session must be opened.
    pub async fn session_for(
        &mut self,
        conn: &SshConnection,
        password: Option<&str>,
        proxy_password: Option<&str>,
        passphrase: Option<&str>,
        interactor: Option<Arc<dyn SshInteractor>>,
    ) -> Result<Arc<SshSession>> {
        if let Some(cached) = self.sessions.get_mut(conn.id.as_str()) {
            if !cached.session.is_closed() {
                cached.last_used = Instant::now();
                return Ok(cached.session.clone());
            }
            self.sessions.remove(conn.id.as_str());
        }
        let session = Arc::new(
            connect_session(conn, password, proxy_password, passphrase, interactor).await?,
        );
        self.sessions.insert(
            conn.id.as_str().to_string(),
            CachedAgent {
                session: session.clone(),
                last_used: Instant::now(),
            },
        );
        Ok(session)
    }

    /// The cached session for `conn_id` if one is present and still open — used
    /// to clean up host-side state on close without forcing a reconnect.
    pub fn peek(&self, conn_id: &str) -> Option<Arc<SshSession>> {
        self.sessions
            .get(conn_id)
            .filter(|c| !c.session.is_closed())
            .map(|c| c.session.clone())
    }

    /// Drop sessions idle longer than `max_idle`, plus any whose handle has
    /// already closed — the backstop that keeps an agent session from holding a
    /// host authenticated forever if the pane-unmount close never fired. A
    /// connection with a live port forward (an in-flight sign-in) is exempt:
    /// reaping under it would strand the OAuth callback.
    pub fn reap_idle(&mut self, max_idle: Duration) {
        let forwards = &self.forwards;
        self.sessions.retain(|id, c| {
            forwards.contains_key(id)
                || (!c.session.is_closed() && c.last_used.elapsed() < max_idle)
        });
    }

    /// Whether a still-open session is cached for this connection. Read-only —
    /// doesn't bump `last_used`, so a status poll never keeps a session alive.
    pub fn has_session(&self, conn_id: &str) -> bool {
        self.sessions
            .get(conn_id)
            .is_some_and(|c| !c.session.is_closed())
    }

    /// Start an ephemeral local→remote TCP forward bound to `127.0.0.1:local_port`
    /// that pipes each connection to `127.0.0.1:remote_port` on the host over the
    /// agent's already-authed `session` (no second auth). Replaces any existing
    /// forward for this connection. Used so an OAuth callback to a loopback port on
    /// the host (e.g. `codex login` → `localhost:1455`) is reachable from the
    /// user's local browser.
    pub async fn start_forward(
        &mut self,
        conn_id: &str,
        session: Arc<SshSession>,
        local_port: u16,
        remote_port: u16,
    ) -> Result<()> {
        self.stop_forward(conn_id);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", local_port))
            .await
            .map_err(|e| SshError::Russh(format!("couldn't bind local port {local_port}: {e}")))?;
        let running = Arc::new(AtomicBool::new(true));
        let running_for_task = running.clone();
        let task = tokio::spawn(async move {
            while running_for_task.load(Ordering::Relaxed) {
                // Time-bounded accept so a stop is noticed even with no traffic.
                let accepted =
                    tokio::time::timeout(Duration::from_millis(400), listener.accept()).await;
                let mut stream = match accepted {
                    Ok(Ok((stream, _))) => stream,
                    Ok(Err(_)) => continue,
                    Err(_) => continue,
                };
                let session = session.clone();
                tokio::spawn(async move {
                    let channel = match session
                        .channel_open_direct_tcpip(
                            "127.0.0.1",
                            u32::from(remote_port),
                            "127.0.0.1",
                            0,
                        )
                        .await
                    {
                        Ok(channel) => channel,
                        Err(_) => return,
                    };
                    let mut channel_stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut channel_stream).await;
                });
            }
        });
        self.forwards
            .insert(conn_id.to_string(), ForwardHandle { running, task });
        Ok(())
    }

    pub fn stop_forward(&mut self, conn_id: &str) {
        if let Some(handle) = self.forwards.remove(conn_id) {
            handle.stop();
        }
    }

    /// Register (replacing any prior) an abort handle for `conn_id`'s current
    /// turn and return it so the streaming loop can await it. One turn runs per
    /// connection at a time, so overwriting a stale handle is safe.
    pub fn register_abort(&mut self, conn_id: &str) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.aborts.insert(conn_id.to_string(), notify.clone());
        notify
    }

    /// Drop `conn_id`'s abort handle once its turn has finished.
    pub fn clear_abort(&mut self, conn_id: &str) {
        self.aborts.remove(conn_id);
    }

    /// Signal the in-flight turn for `conn_id` to stop. Returns whether a turn
    /// was registered. Uses `notify_waiters`, so it only wakes a turn that is
    /// already awaiting (i.e. actually streaming).
    pub fn abort(&self, conn_id: &str) -> bool {
        match self.aborts.get(conn_id) {
            Some(notify) => {
                notify.notify_waiters();
                true
            }
            None => false,
        }
    }

    pub fn disconnect(&mut self, conn_id: &str) {
        self.stop_forward(conn_id);
        self.aborts.remove(conn_id);
        self.sessions.remove(conn_id);
    }

    pub fn disconnect_all(&mut self) {
        for (_, handle) in self.forwards.drain() {
            handle.stop();
        }
        self.aborts.clear();
        self.sessions.clear();
    }
}

/// Probe the host for model tooling in one exec round-trip. Resolves the ollama
/// endpoint from `OLLAMA_HOST` (falling back to `127.0.0.1:<port>`), lists models
/// via `/api/tags` using whichever HTTP client exists (`curl`, else `wget`), and
/// falls back to the `ollama list` CLI when the HTTP API can't be reached or no
/// client is installed. Also records which clients are present so chat can pick a
/// transport. This is why a host with models but no `curl`, or ollama on a
/// non-default port, no longer reports "no local model".
pub async fn detect(session: &SshSession, port: u16) -> Result<AgentInfo> {
    let probe = format!(
        r#"
host="${{OLLAMA_HOST:-127.0.0.1:{port}}}"
host="${{host#http://}}"; host="${{host#https://}}"; host="${{host%/}}"
case "$host" in */*) host="${{host%%/*}}";; esac
case "$host" in
  *:*) h="${{host%:*}}"; p="${{host##*:}}";;
  *) h="$host"; p={port};;
esac
case "$p" in ''|*[!0-9]*) p={port};; esac
[ -z "$h" ] && h=127.0.0.1
case "$h" in 0.0.0.0|::|'[::]'|'*') h=127.0.0.1;; esac
base="http://$h:$p"
echo '###PORT'; echo "$p"
echo '###TAGS'
if command -v curl >/dev/null 2>&1; then curl -s --max-time 5 "$base/api/tags" 2>/dev/null;
elif command -v wget >/dev/null 2>&1; then wget -qO- --timeout=5 "$base/api/tags" 2>/dev/null; fi
echo
echo '###LIST'; command -v ollama >/dev/null 2>&1 && ollama list 2>/dev/null
echo '###CURL'; command -v curl >/dev/null 2>&1 && echo yes
echo '###WGET'; command -v wget >/dev/null 2>&1 && echo yes
echo '###OLLAMA'; command -v ollama >/dev/null 2>&1 && echo yes
echo '###LLM'; command -v llm >/dev/null 2>&1 && echo yes
echo '###CLAUDE'; command -v claude >/dev/null 2>&1 && echo yes
echo '###CODEX'; command -v codex >/dev/null 2>&1 && echo yes
"#
    );
    let out = exec_on(session, &probe, None).await?;
    Ok(parse_detect(&out.stdout, port))
}

/// Parse the marker-delimited detect output into [`AgentInfo`]. Lenient: a
/// missing/unparseable section just yields its empty/false default. Models come
/// from `/api/tags` JSON first; if that's empty, from `ollama list` output.
fn parse_detect(stdout: &str, default_port: u16) -> AgentInfo {
    let mut section = "";
    let mut tags = String::new();
    let mut list = String::new();
    let mut port_text = String::new();
    let mut has_curl = false;
    let mut has_wget = false;
    let mut has_ollama = false;
    let mut has_llm = false;
    let mut has_claude = false;
    let mut has_codex = false;
    for line in stdout.lines() {
        match line.trim() {
            "###PORT" => section = "port",
            "###TAGS" => section = "tags",
            "###LIST" => section = "list",
            "###CURL" => section = "curl",
            "###WGET" => section = "wget",
            "###OLLAMA" => section = "ollama",
            "###LLM" => section = "llm",
            "###CLAUDE" => section = "claude",
            "###CODEX" => section = "codex",
            other => match section {
                "port" => port_text.push_str(other),
                "tags" => {
                    tags.push_str(line);
                    tags.push('\n');
                }
                "list" => {
                    list.push_str(line);
                    list.push('\n');
                }
                "curl" => has_curl |= other == "yes",
                "wget" => has_wget |= other == "yes",
                "ollama" => has_ollama |= other == "yes",
                "llm" => has_llm |= other == "yes",
                "claude" => has_claude |= other == "yes",
                "codex" => has_codex |= other == "yes",
                _ => {}
            },
        }
    }

    let mut ollama_models: Vec<String> = serde_json::from_str::<serde_json::Value>(tags.trim())
        .ok()
        .and_then(|v| v.get("models").and_then(|m| m.as_array()).cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    // Fall back to the `ollama list` CLI when the HTTP API returned nothing
    // (no curl/wget, or the server wasn't reachable over HTTP for this probe).
    if ollama_models.is_empty() {
        ollama_models = parse_ollama_list(&list);
    }

    let port = port_text.trim().parse::<u16>().unwrap_or(default_port);

    AgentInfo {
        has_curl,
        has_wget,
        has_ollama,
        has_llm,
        has_claude,
        has_codex,
        ollama_models,
        port,
    }
}

/// Parse `ollama list` tabular output into model names (the first column),
/// skipping the `NAME …` header row and blank lines.
fn parse_ollama_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(name) = trimmed.split_whitespace().next() else {
            continue;
        };
        if name == "NAME" || name.is_empty() {
            continue;
        }
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Open an exec channel running `curl` against the host's ollama `/api/chat`,
/// pipe the request body (`model` + `messages`, streaming) to its stdin, and
/// return the channel — the caller streams the NDJSON response off it. The
/// request travels only over the SSH session to `localhost` on the host.
pub async fn open_chat_channel(
    session: &SshSession,
    model: &str,
    messages: &serde_json::Value,
    port: u16,
) -> Result<Channel<Msg>> {
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
    });
    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| SshError::Russh(format!("couldn't encode chat request: {e}")))?;

    // Body arrives on stdin; stash it in a temp file so either client can POST
    // it. curl streams the NDJSON reply (`-N`); wget buffers it (still parsed
    // line-by-line by the caller), so a host with only wget still works.
    let cmd = format!(
        r#"umask 077; t=$(mktemp 2>/dev/null || echo /tmp/portbay-agent.$$); cat > "$t"; trap 'rm -f "$t"' EXIT
if command -v curl >/dev/null 2>&1; then
  curl -sN --max-time 600 -X POST "http://127.0.0.1:{port}/api/chat" -H 'Content-Type: application/json' --data-binary @"$t"
else
  wget -qO- --timeout=600 --header='Content-Type: application/json' --post-file="$t" "http://127.0.0.1:{port}/api/chat"
fi"#
    );
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't open chat channel: {e}")))?;
    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't start the model request: {e}")))?;
    channel
        .data(body_bytes.as_slice())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't send the chat request: {e}")))?;
    channel
        .eof()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't finish the chat request: {e}")))?;
    Ok(channel)
}

/// Run one approved command on the agent's cached session.
pub async fn run_command(session: &SshSession, command: &str) -> Result<ExecResult> {
    exec_on(session, command, None).await
}

/// One-shot code completion against the host's ollama `/api/generate`, using its
/// native Fill-in-the-Middle `suffix` field (no hand-rolled FIM tokens). Used
/// for inline ghost-text completion in the editor and the next-command terminal
/// hint. `prefix` is the text before the cursor, `suffix` the text after.
///
/// The request body travels on stdin (so the code never reaches the shell
/// parser), `stream:false` keeps the reply to one JSON object, and a short
/// `--max-time` bounds an abandoned request on the host — cancellation itself is
/// the frontend's job (it ignores out-of-date results). Returns the raw
/// completion string for the caller to post-process.
pub async fn ollama_generate(
    session: &SshSession,
    model: &str,
    prefix: &str,
    suffix: &str,
    port: u16,
    num_predict: u32,
) -> Result<String> {
    let body = serde_json::json!({
        "model": model,
        "prompt": prefix,
        "suffix": suffix,
        "stream": false,
        "options": {
            "num_predict": num_predict,
            "temperature": 0.1,
            "stop": ["\n\n", "<|file_separator|>", "<|endoftext|>"],
        },
    });
    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| SshError::Russh(format!("couldn't encode completion request: {e}")))?;

    let cmd = format!(
        r#"umask 077; t=$(mktemp 2>/dev/null || echo /tmp/portbay-complete.$$); cat > "$t"; trap 'rm -f "$t"' EXIT
if command -v curl >/dev/null 2>&1; then
  curl -s --max-time 8 -X POST "http://127.0.0.1:{port}/api/generate" -H 'Content-Type: application/json' --data-binary @"$t"
else
  wget -qO- --timeout=8 --header='Content-Type: application/json' --post-file="$t" "http://127.0.0.1:{port}/api/generate"
fi"#
    );

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't open completion channel: {e}")))?;
    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't start the completion request: {e}")))?;
    channel
        .data(body_bytes.as_slice())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't send the completion request: {e}")))?;
    channel
        .eof()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't finish the completion request: {e}")))?;

    let mut stdout: Vec<u8> = Vec::new();
    let mut channel = channel;
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }

    let text = String::from_utf8_lossy(&stdout);
    // `/api/generate` with `stream:false` replies with a single JSON object.
    let parsed: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|_| SshError::Russh("ollama returned an unreadable completion response".into()))?;
    Ok(parsed
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// Host directory (under `$HOME`) where chat attachments are staged so the
/// official CLI can read them with its own tools. Cleared on session close.
const ATTACHMENT_BASE: &str = ".portbay/agent-attachments";

/// Cap a single attachment at 25 MiB — screenshots and source files, not bulk
/// transfers (the file browser is the path for those).
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// Reduce an attachment name to a safe basename: drop any directory part, reject
/// `..`, and keep only a conservative charset so it's safe to splice into the
/// upload shell command. Empty/odd names fall back to `file`.
fn sanitize_attachment_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.replace(' ', "_")
    }
}

/// A turn id is spliced into the upload path, so accept only the uuid-ish charset
/// the frontend generates. Returns `None` for anything else.
fn safe_turn_id(id: &str) -> Option<&str> {
    if !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Some(id)
    } else {
        None
    }
}

/// Upload one attachment's bytes to `~/.portbay/agent-attachments/<turn>/<name>`
/// on the host over the cached agent session and return the **absolute** remote
/// path (resolved from the host's `$HOME`). The CLI then reads that path with its
/// own tools (`@path` for Claude). Content travels base64 on stdin so arbitrary
/// bytes never reach the shell parser; the name + turn id are sanitized. Remote
/// staging uses private permissions because screenshots can include secrets.
pub async fn upload_attachment(
    session: &SshSession,
    turn_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<String> {
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(SshError::Russh(format!(
            "attachment is larger than the {} MiB limit",
            MAX_ATTACHMENT_BYTES / (1024 * 1024)
        )));
    }
    let turn = safe_turn_id(turn_id)
        .ok_or_else(|| SshError::Russh("invalid attachment turn id".to_string()))?;
    let safe_name = sanitize_attachment_name(name);

    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);

    // `$HOME` expands on the host; the final line prints the resolved absolute
    // path so the frontend can reference it regardless of the remote home.
    let cmd = format!(
        r#"set -e
umask 077
dir="$HOME/{ATTACHMENT_BASE}/{turn}"
mkdir -p "$dir"
chmod 700 "$HOME/.portbay" "$HOME/{ATTACHMENT_BASE}" "$dir" 2>/dev/null || true
base64 -d > "$dir/{safe_name}"
chmod 600 "$dir/{safe_name}" 2>/dev/null || true
printf '%s\n' "$dir/{safe_name}""#
    );

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't open upload channel: {e}")))?;
    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't start the upload: {e}")))?;
    channel
        .data(encoded.as_bytes())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't send the attachment: {e}")))?;
    channel
        .eof()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't finish the upload: {e}")))?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut code: Option<u32> = None;
    let mut channel = channel;
    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::Data { data } => stdout.push_str(&String::from_utf8_lossy(&data)),
            russh::ChannelMsg::ExtendedData { data, .. } => {
                stderr.push_str(&String::from_utf8_lossy(&data))
            }
            russh::ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
            russh::ChannelMsg::Eof | russh::ChannelMsg::Close => break,
            _ => {}
        }
    }

    if code.unwrap_or(0) != 0 {
        let detail = stderr.trim();
        return Err(SshError::Russh(if detail.is_empty() {
            "the host rejected the attachment upload".to_string()
        } else {
            format!("attachment upload failed: {detail}")
        }));
    }
    let path = stdout
        .trim()
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    if path.is_empty() {
        return Err(SshError::Russh(
            "the host did not report the attachment path".to_string(),
        ));
    }
    Ok(path)
}

/// Best-effort removal of one turn's staged attachments after the agent has
/// consumed them. This keeps pasted screenshots and other one-off context from
/// lingering on live hosts. If the agent needed a durable file, it should have
/// copied/moved it into the project during the turn.
pub async fn cleanup_attachment_turn(session: &SshSession, turn_id: &str) {
    let Some(turn) = safe_turn_id(turn_id) else {
        return;
    };
    let cmd = format!(
        r#"rm -rf "$HOME/{ATTACHMENT_BASE}/{turn}" 2>/dev/null || true
rmdir "$HOME/{ATTACHMENT_BASE}" "$HOME/.portbay" 2>/dev/null || true"#
    );
    let _ = exec_on(session, &cmd, None).await;
}

/// Best-effort removal of this session's staged attachments (on session close).
pub async fn cleanup_attachments(session: &SshSession) {
    let cmd = format!(r#"rm -rf "$HOME/{ATTACHMENT_BASE}" 2>/dev/null || true"#);
    let _ = exec_on(session, &cmd, None).await;
}

/// Best-effort sweep, on agent open, of attachment staging left behind by a
/// previous app run (crash, force quit, dropped network — the paths where the
/// per-turn and on-close cleanups never got to run). Only turn directories
/// older than an hour are removed, so an in-flight turn from another window
/// connected to the same host account is never touched; anything the agent
/// copied into a project lives outside the staging base and is untouched by
/// definition.
pub async fn sweep_stale_attachments(session: &SshSession) {
    let cmd = format!(
        r#"d="$HOME/{ATTACHMENT_BASE}"
[ -d "$d" ] || exit 0
find "$d" -mindepth 1 -maxdepth 1 -mmin +60 -exec rm -rf {{}} + 2>/dev/null || true
rmdir "$d" "$HOME/.portbay" 2>/dev/null || true"#
    );
    let _ = exec_on(session, &cmd, None).await;
}

/// An *officially-installed* agent CLI we drive over the SSH session. We never
/// patch, wrap, or re-prompt these — the host's own binary owns reasoning, tool
/// execution, and safety, and auto-updates itself. PortBay only transports the
/// turn and renders the provider's own event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliProvider {
    /// Anthropic Claude Code (`claude`).
    Claude,
    /// OpenAI Codex (`codex`).
    Codex,
}

impl CliProvider {
    /// Map the frontend's provider id to a CLI provider (`None` for non-CLI ids
    /// like `ollama`).
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// Claude Code's official `--permission-mode` values we expose. Anything else is
/// dropped (the CLI falls back to its own default) so we never inject a mode the
/// provider doesn't recognise.
fn claude_permission_mode(mode: &str) -> Option<&'static str> {
    match mode {
        "plan" => Some("plan"),
        "acceptEdits" => Some("acceptEdits"),
        "bypassPermissions" => Some("bypassPermissions"),
        "default" => Some("default"),
        _ => None,
    }
}

/// Translate the chat mode (carried in Claude's `--permission-mode` vocabulary,
/// the one param the frontend sends for both providers) into Codex's official
/// `codex exec` sandbox flags. The mapping keeps the three UI modes meaningful:
/// - `plan` / `default` (Normal, Gather) → `--sandbox read-only`: the agent can
///   read and reason but cannot write or run commands that escape the sandbox.
/// - `acceptEdits` (Agent) → `--full-auto`: workspace-write sandbox, edits and
///   commands run without prompting (which Codex can't do in `exec` anyway).
/// - `bypassPermissions` (full-auto override) →
///   `--dangerously-bypass-approvals-and-sandbox`: no sandbox, full host access.
///
/// `None`/unknown returns the empty string so Codex falls back to its own
/// default — we never inject a flag the installed binary might not recognise.
fn codex_sandbox_flags(permission_mode: Option<&str>) -> &'static str {
    match permission_mode {
        Some("plan") | Some("default") => " --sandbox read-only",
        Some("acceptEdits") => " --full-auto",
        Some("bypassPermissions") => " --dangerously-bypass-approvals-and-sandbox",
        _ => "",
    }
}

/// A Claude session id is safe to splice into the command line only if it's the
/// restricted charset the CLI actually emits (uuid-ish). Reject anything else.
fn safe_session_id(id: &str) -> Option<&str> {
    if !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Some(id)
    } else {
        None
    }
}

/// A model name is spliced onto the command line (`--model <m>`), so accept only
/// the charset real model ids/aliases use (e.g. `opus`, `claude-opus-4-6`,
/// `o4-mini`). Reject anything else rather than shell-quote.
fn safe_model(model: &str) -> Option<&str> {
    let m = model.trim();
    if !m.is_empty()
        && m.len() <= 64
        && m.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
    {
        Some(m)
    } else {
        None
    }
}

/// Open an exec channel that drives the host's official agent CLI in its
/// non-interactive streaming mode, feed one user turn on stdin, and return the
/// channel so the caller can stream the provider's own events. User text travels
/// on stdin (never on the command line), so nothing the user types is shell-
/// interpreted. Auth comes from the host's own `claude login` / `codex login` —
/// no keys live in PortBay.
///
/// - **Claude Code:** `claude -p --input-format stream-json --output-format
///   stream-json --verbose --include-partial-messages [--permission-mode M]
///   [--resume ID]`; the user turn is a stream-json `user` message on stdin.
/// - **Codex:** `codex exec --json` with the prompt staged in a temp file and
///   passed as a single quoted argument (`"$(cat …)"`).
///
/// Wrap a remote command so it runs inside `cwd`. A leading `~` expands to the
/// host's `$HOME`; the rest of the path is single-quoted so spaces and shell
/// metacharacters are inert. Returns `cmd` unchanged when no working directory
/// is set (or it's just the home shorthand `~`), so the default behaviour stays
/// byte-identical to before. Used so the agent (and approved commands) resolve
/// project-relative paths against the directory the user is actually working in.
pub fn wrap_in_cwd(cmd: &str, cwd: Option<&str>) -> String {
    let dir = match cwd.map(str::trim) {
        Some(d) if !d.is_empty() && d != "~" && d != "~/" => d,
        _ => return cmd.to_string(),
    };
    let sq = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let target = if let Some(rest) = dir.strip_prefix("~/") {
        format!("\"$HOME\"/{}", sq(rest))
    } else {
        sq(dir)
    };
    // Subshell so a multi-statement command still runs entirely inside `cwd`.
    format!("cd {target} && (\n{cmd}\n)")
}

pub async fn open_agent_cli_channel(
    session: &SshSession,
    provider: CliProvider,
    prompt: &str,
    permission_mode: Option<&str>,
    resume_id: Option<&str>,
    model: Option<&str>,
    cwd: Option<&str>,
) -> Result<Channel<Msg>> {
    let (cmd, stdin_bytes): (String, Vec<u8>) = match provider {
        CliProvider::Claude => {
            let mut cmd = String::from(
                "claude -p --input-format stream-json --output-format stream-json \
                 --verbose --include-partial-messages",
            );
            if let Some(mode) = permission_mode.and_then(claude_permission_mode) {
                cmd.push_str(" --permission-mode ");
                cmd.push_str(mode);
            }
            if let Some(m) = model.and_then(safe_model) {
                cmd.push_str(" --model ");
                cmd.push_str(m);
            }
            if let Some(id) = resume_id.and_then(safe_session_id) {
                cmd.push_str(" --resume ");
                cmd.push_str(id);
            }
            // stream-json input: one `user` message, then EOF. content-as-string
            // is valid Anthropic message shape; serde keeps the text shell-safe.
            let msg = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": prompt },
            });
            let mut bytes = serde_json::to_vec(&msg)
                .map_err(|e| SshError::Russh(format!("couldn't encode agent turn: {e}")))?;
            bytes.push(b'\n');
            (cmd, bytes)
        }
        CliProvider::Codex => {
            // The prompt is staged in a temp file we write over stdin, then passed
            // as one double-quoted argument so arbitrary text never reaches the
            // shell parser. `codex exec --json` reuses the host's `codex login`.
            // `--skip-git-repo-check` lets it run when the working/home dir isn't
            // a git repo; without it codex aborts ("Not inside a trusted
            // directory...") before it ever reads the prompt.
            let model_flag = match model.and_then(safe_model) {
                Some(m) => format!(" --model {m}"),
                None => String::new(),
            };
            // The chat mode rides the same `permission_mode` param as Claude;
            // translate it to Codex's sandbox flags so Gather is read-only and
            // Agent can actually write/run (see `codex_sandbox_flags`).
            let sandbox_flags = codex_sandbox_flags(permission_mode);
            let cmd = format!(
                "umask 077; t=$(mktemp 2>/dev/null || echo /tmp/portbay-codex.$$); cat > \"$t\"; \
                 trap 'rm -f \"$t\"' EXIT; \
                 codex exec{sandbox_flags} --skip-git-repo-check --json{model_flag} \"$(cat \"$t\")\"",
            );
            (cmd, prompt.as_bytes().to_vec())
        }
    };

    // Run the agent in the chosen working directory so it can place dropped-in
    // attachments and resolve project-relative paths like a local checkout.
    let cmd = wrap_in_cwd(&cmd, cwd);

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't open agent channel: {e}")))?;
    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't start the agent CLI: {e}")))?;
    channel
        .data(stdin_bytes.as_slice())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't send the agent turn: {e}")))?;
    channel
        .eof()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't finish the agent turn: {e}")))?;
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_detect_reads_models_and_tools() {
        let stdout = "###TAGS\n{\"models\":[{\"name\":\"llama3.1:8b\"},{\"name\":\"qwen2.5:7b\"}]}\n###CURL\nyes\n###LLM\nyes\n";
        let info = parse_detect(stdout, 11434);
        assert!(info.has_curl);
        assert!(info.has_llm);
        assert_eq!(info.ollama_models, vec!["llama3.1:8b", "qwen2.5:7b"]);
        assert_eq!(info.port, 11434);
    }

    #[test]
    fn parse_detect_reads_cli_agents() {
        // ollama with no models, but both agentic CLIs are installed.
        let stdout = "###TAGS\n\n###CLAUDE\nyes\n###CODEX\nyes\n";
        let info = parse_detect(stdout, 11434);
        assert!(info.has_claude);
        assert!(info.has_codex);
        assert!(info.ollama_models.is_empty());
    }

    #[test]
    fn parse_detect_cli_agents_default_false() {
        let info = parse_detect(
            "###TAGS\n{\"models\":[{\"name\":\"m\"}]}\n###CURL\nyes\n",
            11434,
        );
        assert!(!info.has_claude);
        assert!(!info.has_codex);
    }

    #[test]
    fn parse_detect_handles_no_model_and_no_tools() {
        let info = parse_detect("###TAGS\n\n###CURL\n\n###LLM\n", 11434);
        assert!(!info.has_curl);
        assert!(!info.has_llm);
        assert!(info.ollama_models.is_empty());
    }

    #[test]
    fn parse_detect_survives_garbage_tags() {
        let info = parse_detect("###TAGS\nnot json\n###CURL\nyes\n", 11434);
        assert!(info.has_curl);
        assert!(info.ollama_models.is_empty());
    }

    #[test]
    fn parse_detect_falls_back_to_ollama_list_when_http_empty() {
        // No curl, HTTP returned nothing, but `ollama list` shows two models.
        let stdout = "###PORT\n11434\n###TAGS\n\n###LIST\nNAME            ID    SIZE    MODIFIED\nllama3.1:8b     abc   4.7 GB  2 days ago\nqwen2.5:7b      def   4.7 GB  3 days ago\n###CURL\n###WGET\nyes\n###OLLAMA\nyes\n###LLM\n";
        let info = parse_detect(stdout, 11434);
        assert!(!info.has_curl);
        assert!(info.has_wget);
        assert!(info.has_ollama);
        assert_eq!(info.ollama_models, vec!["llama3.1:8b", "qwen2.5:7b"]);
    }

    #[test]
    fn parse_detect_reads_resolved_port_from_probe() {
        let stdout = "###PORT\n11500\n###TAGS\n{\"models\":[{\"name\":\"m\"}]}\n###CURL\nyes\n";
        let info = parse_detect(stdout, 11434);
        assert_eq!(info.port, 11500, "honours OLLAMA_HOST's resolved port");
        assert_eq!(info.ollama_models, vec!["m"]);
    }

    #[test]
    fn parse_ollama_list_skips_header_and_dedupes() {
        let text = "NAME    ID  SIZE\nfoo:7b  a   1G\nfoo:7b  a   1G\nbar:1b  b   1G\n";
        assert_eq!(parse_ollama_list(text), vec!["foo:7b", "bar:1b"]);
    }

    #[test]
    fn sanitize_attachment_name_strips_paths_and_traversal() {
        // A directory part or traversal is reduced to a safe basename.
        assert_eq!(sanitize_attachment_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_attachment_name("/abs/path/log.txt"), "log.txt");
        assert_eq!(sanitize_attachment_name("a\\b\\c.png"), "c.png");
        // Shell-significant characters are dropped; spaces become underscores.
        assert_eq!(sanitize_attachment_name("na;me `rm`.txt"), "name_rm.txt");
        assert_eq!(sanitize_attachment_name("my shot.png"), "my_shot.png");
        // Degenerate names fall back rather than producing an empty path.
        assert_eq!(sanitize_attachment_name("..."), "file");
        assert_eq!(sanitize_attachment_name(""), "file");
    }

    #[test]
    fn codex_sandbox_flags_map_each_mode() {
        // Read-only for the non-editing modes; full-auto for Agent; full bypass
        // only for the explicit override; nothing (Codex default) when unset.
        assert_eq!(codex_sandbox_flags(Some("plan")), " --sandbox read-only");
        assert_eq!(codex_sandbox_flags(Some("default")), " --sandbox read-only");
        assert_eq!(codex_sandbox_flags(Some("acceptEdits")), " --full-auto");
        assert_eq!(
            codex_sandbox_flags(Some("bypassPermissions")),
            " --dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(codex_sandbox_flags(None), "");
        assert_eq!(codex_sandbox_flags(Some("nonsense")), "");
    }

    #[test]
    fn safe_model_accepts_ids_and_rejects_injection() {
        assert_eq!(safe_model("opus"), Some("opus"));
        assert_eq!(safe_model("claude-opus-4-6"), Some("claude-opus-4-6"));
        assert_eq!(safe_model("o4-mini"), Some("o4-mini"));
        assert_eq!(safe_model("gpt-4.1"), Some("gpt-4.1"));
        assert_eq!(safe_model("  sonnet  "), Some("sonnet"));
        assert_eq!(safe_model("a b"), None);
        assert_eq!(safe_model("$(rm -rf)"), None);
        assert_eq!(safe_model(""), None);
    }

    #[test]
    fn safe_turn_id_accepts_uuid_and_rejects_injection() {
        assert_eq!(
            safe_turn_id("3b1f9c2a-7d4e-4a1b-9c2a-7d4e4a1b9c2a"),
            Some("3b1f9c2a-7d4e-4a1b-9c2a-7d4e4a1b9c2a")
        );
        assert_eq!(safe_turn_id("ok_id-1"), Some("ok_id-1"));
        assert_eq!(safe_turn_id("../escape"), None);
        assert_eq!(safe_turn_id("has space"), None);
        assert_eq!(safe_turn_id("$(whoami)"), None);
        assert_eq!(safe_turn_id(""), None);
    }
}
