//! Human-in-the-loop approval queue for agent-issued database writes.
//!
//! PortBay's differentiator is that an AI agent can touch the database *safely*:
//! the agent (through the `portbay_db_execute` MCP tool) can propose a write or
//! DDL statement, but it does not run until a human approves the exact statement
//! in the PortBay app.
//!
//! The MCP server runs as its own process, so the agent side and the GUI side
//! can't share in-memory state or Tauri events. They rendez-vous through a
//! small file queue under `<app-data>/db-approvals/`:
//!
//! 1. The MCP tool writes `<id>.pending.json` and blocks ([`await_decision`]).
//! 2. The GUI polls [`list_pending`], shows an approve/deny modal, and writes
//!    the verdict with [`resolve`] (`<id>.decision.json`).
//! 3. The MCP tool reads the decision, removes both files, and either runs the
//!    statement or returns a "denied" error. A request that is never answered
//!    times out and is cleaned up.
//!
//! This is the *runtime* enforcement of the same "hard-to-reverse actions pause
//! for a human" principle the dispatch protocol states in prose — see the
//! blast-radius tiering in [`crate::context::adapters::PROTOCOL_MD`]. Keep the
//! two consistent: the protocol tells an agent to stop and ask before a guarded
//! action; this queue is what makes the database case unskippable.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Subdirectory of app-data holding the queue files.
pub const DIR_NAME: &str = "db-approvals";

/// How often [`await_decision`] re-checks for a verdict.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// `<app-data>/db-approvals/`.
pub fn approvals_dir(app_data: &Path) -> PathBuf {
    app_data.join(DIR_NAME)
}

/// A write/DDL statement an agent wants to run, awaiting human approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingWrite {
    pub id: String,
    pub instance_id: String,
    pub engine: String,
    pub schema: Option<String>,
    pub sql: String,
    /// Where the request came from, e.g. `"mcp-agent"`.
    pub origin: String,
    pub created_at_ms: u64,
}

/// The human verdict on a [`PendingWrite`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub approved: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Milliseconds since the Unix epoch (0 if the clock is before the epoch).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A process-unique request id: `w<millis>-<counter>`. Safe as a filename.
pub fn new_id() -> String {
    format!("w{}-{}", now_ms(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Reject ids that aren't simple slugs, so a caller-supplied id can never
/// escape the queue directory (path traversal) when we build a filename.
fn safe_id(id: &str) -> AppResult<&str> {
    if !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Ok(id)
    } else {
        Err(AppError::BadInput("invalid approval id".into()))
    }
}

fn pending_path(dir: &Path, id: &str) -> AppResult<PathBuf> {
    Ok(dir.join(format!("{}.pending.json", safe_id(id)?)))
}

fn decision_path(dir: &Path, id: &str) -> AppResult<PathBuf> {
    Ok(dir.join(format!("{}.decision.json", safe_id(id)?)))
}

/// Write a pending request into the queue.
pub fn enqueue(dir: &Path, req: &PendingWrite) -> AppResult<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| AppError::Internal(format!("create approvals dir: {e}")))?;
    let body = serde_json::to_vec_pretty(req)
        .map_err(|e| AppError::Internal(format!("serialize approval: {e}")))?;
    std::fs::write(pending_path(dir, &req.id)?, body)
        .map_err(|e| AppError::Internal(format!("write approval: {e}")))
}

/// Every request still awaiting a verdict, newest first.
pub fn list_pending(dir: &Path) -> Vec<PendingWrite> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out; // no dir yet ⇒ nothing pending
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_pending = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".pending.json"))
            .unwrap_or(false);
        if !is_pending {
            continue;
        }
        if let Ok(body) = std::fs::read_to_string(&path) {
            if let Ok(req) = serde_json::from_str::<PendingWrite>(&body) {
                out.push(req);
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
    out
}

/// Record the human verdict for a request.
pub fn resolve(dir: &Path, id: &str, decision: &Decision) -> AppResult<()> {
    let pending = pending_path(dir, id)?;
    if !pending.is_file() {
        return Err(AppError::NotFound(format!("approval:{id}")));
    }
    let body = serde_json::to_vec_pretty(decision)
        .map_err(|e| AppError::Internal(format!("serialize decision: {e}")))?;
    std::fs::write(decision_path(dir, id)?, body)
        .map_err(|e| AppError::Internal(format!("write decision: {e}")))
}

/// Remove both queue files for a request (best-effort).
fn cleanup(dir: &Path, id: &str) {
    if let Ok(p) = pending_path(dir, id) {
        let _ = std::fs::remove_file(p);
    }
    if let Ok(p) = decision_path(dir, id) {
        let _ = std::fs::remove_file(p);
    }
}

/// Block until the request is approved/denied or `timeout` elapses, then clean
/// up the queue files. A timeout returns `BadInput` (the agent should treat it
/// as "not approved").
pub async fn await_decision(dir: &Path, id: &str, timeout: Duration) -> AppResult<Decision> {
    let decision_file = decision_path(dir, id)?;
    let deadline = SystemTime::now() + timeout;
    loop {
        // A successful parse means the verdict is fully written. A read/parse
        // failure is treated as "not ready yet" (the file may be missing or
        // half-written by the GUI) and we keep polling until the deadline,
        // rather than aborting on a transient partial read.
        if let Ok(body) = std::fs::read_to_string(&decision_file) {
            if let Ok(decision) = serde_json::from_str::<Decision>(&body) {
                cleanup(dir, id);
                return Ok(decision);
            }
        }
        if SystemTime::now() >= deadline {
            cleanup(dir, id);
            return Err(AppError::BadInput(
                "the database write was not approved in time".into(),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("pb-dbapproval-{}-{}", std::process::id(), new_id()))
    }

    fn sample(id: &str) -> PendingWrite {
        PendingWrite {
            id: id.into(),
            instance_id: "app-db".into(),
            engine: "postgres".into(),
            schema: None,
            sql: "DELETE FROM users WHERE id = 1".into(),
            origin: "mcp-agent".into(),
            created_at_ms: now_ms(),
        }
    }

    #[test]
    fn enqueue_then_list_then_resolve_roundtrips() {
        let dir = tmp();
        let req = sample("w1-0");
        enqueue(&dir, &req).unwrap();
        let pending = list_pending(&dir);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sql, req.sql);

        resolve(
            &dir,
            "w1-0",
            &Decision {
                approved: true,
                reason: None,
            },
        )
        .unwrap();
        assert!(decision_path(&dir, "w1-0").unwrap().is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_unknown_id_is_not_found() {
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let err = resolve(
            &dir,
            "nope",
            &Decision {
                approved: false,
                reason: None,
            },
        );
        assert!(matches!(err, Err(AppError::NotFound(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ids_with_path_separators_are_rejected() {
        let dir = tmp();
        assert!(pending_path(&dir, "../escape").is_err());
        assert!(pending_path(&dir, "a/b").is_err());
        assert!(pending_path(&dir, "ok-123_W").is_ok());
    }

    #[tokio::test]
    async fn await_decision_times_out_and_cleans_up() {
        let dir = tmp();
        enqueue(&dir, &sample("w2-0")).unwrap();
        let res = await_decision(&dir, "w2-0", Duration::from_millis(120)).await;
        assert!(res.is_err());
        assert!(!pending_path(&dir, "w2-0").unwrap().is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn await_decision_returns_the_verdict() {
        let dir = tmp();
        enqueue(&dir, &sample("w3-0")).unwrap();
        resolve(
            &dir,
            "w3-0",
            &Decision {
                approved: true,
                reason: Some("ok".into()),
            },
        )
        .unwrap();
        let res = await_decision(&dir, "w3-0", Duration::from_secs(2))
            .await
            .unwrap();
        assert!(res.approved);
        // files cleaned up
        assert!(!decision_path(&dir, "w3-0").unwrap().is_file());
        std::fs::remove_dir_all(&dir).ok();
    }
}
