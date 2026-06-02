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
use std::sync::Arc;

use russh::client::Msg;
use russh::Channel;
use serde::Serialize;

use crate::registry::SshConnection;
use crate::ssh::backend::{Result, SshError};
use crate::ssh::exec::{exec_on, ExecResult};
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
    /// Models discovered on the host — via ollama's `/api/tags`, falling back to
    /// `ollama list`. Empty = no server model found.
    pub ollama_models: Vec<String>,
    /// Loopback port the model API answered on (resolved from `OLLAMA_HOST`,
    /// else the default). Chat requests target this port.
    pub port: u16,
}

/// One cached, authenticated session backing the agent for a connection.
#[derive(Default)]
pub struct AgentManager {
    sessions: HashMap<String, Arc<SshSession>>,
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
    ) -> Result<Arc<SshSession>> {
        if let Some(cached) = self.sessions.get(conn.id.as_str()) {
            if !cached.is_closed() {
                return Ok(cached.clone());
            }
            self.sessions.remove(conn.id.as_str());
        }
        // The MCP agent is headless — no window to prompt — so keep the legacy
        // silent TOFU for host keys.
        let session =
            Arc::new(connect_session(conn, password, proxy_password, passphrase, None).await?);
        self.sessions
            .insert(conn.id.as_str().to_string(), session.clone());
        Ok(session)
    }

    pub fn disconnect(&mut self, conn_id: &str) {
        self.sessions.remove(conn_id);
    }

    pub fn disconnect_all(&mut self) {
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
    for line in stdout.lines() {
        match line.trim() {
            "###PORT" => section = "port",
            "###TAGS" => section = "tags",
            "###LIST" => section = "list",
            "###CURL" => section = "curl",
            "###WGET" => section = "wget",
            "###OLLAMA" => section = "ollama",
            "###LLM" => section = "llm",
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
        r#"t=$(mktemp 2>/dev/null || echo /tmp/portbay-agent.$$); cat > "$t"; trap 'rm -f "$t"' EXIT
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
}
