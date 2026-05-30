//! Cross-project agent-activity notifications.
//!
//! Agent comments and status changes are written to each project's
//! `.portbay/audit.jsonl` — often by the MCP sidecar, a *separate* process that
//! can't emit Tauri events. So the app can't learn of them through the command
//! layer; it has to watch the logs. This module is that watcher: a background
//! scan (driven by [`NotificationCenter::scan`], ticked from `lib.rs`) reads
//! every registered project's audit log, turns newly-appended **notable** agent
//! activity into [`Notification`]s, and surfaces them two ways:
//!
//! - **in-app**: persisted to `<data_dir>/PortBay/notifications.json` and emitted
//!   on the `portbay://notifications` channel, so the topbar bell shows them even
//!   when the terminal is closed or the user is on another project;
//! - **desktop**: a native banner, but only for cards the user is watching
//!   (`subscribed`) and only when the `desktop_notifications` preference is on.
//!
//! Policy (per product decision): notify on agent **comments**, **blocked**
//! (problems), and **warnings** (e.g. an acceptance check that failed). Happy
//! paths — Done, Review, dispatch, ordinary moves — never notify.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// The audit-log scanner is board-coupled: it reads `crate::context` (the board's
// audit/card layer, injected only in official `tasks`-feature builds). The
// public OSS build keeps the bell shell (store + command surface) but compiles
// the scanner out entirely.
#[cfg(feature = "tasks")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(feature = "tasks")]
use tauri::{AppHandle, Emitter, Manager};
#[cfg(feature = "tasks")]
use crate::context::audit::{self, Actor, AuditEntry};
#[cfg(feature = "tasks")]
use crate::context::board::{self, BoardStatus};
#[cfg(feature = "tasks")]
use crate::registry::store;
#[cfg(feature = "tasks")]
use crate::registry::Registry;
#[cfg(feature = "tasks")]
use crate::state::AppState;

/// Tauri event channel a freshly-recorded notification is emitted on.
pub const NOTIFICATIONS_CHANNEL: &str = "portbay://notifications";

/// How often the scanner wakes to diff each project's audit log. Audit files
/// are small and unchanged ones are skipped via an mtime cache, so this stays
/// cheap; a few seconds keeps the bell near-real-time without busy-reading.
#[cfg(feature = "tasks")]
const SCAN_INTERVAL: Duration = Duration::from_secs(4);

/// Spawn the background notification scanner. Returns immediately; the task runs
/// for the lifetime of the app handle, scanning every registered project's
/// audit log on each tick and surfacing new agent activity.
#[cfg(feature = "tasks")]
pub fn spawn_scanner(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(SCAN_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let state: tauri::State<AppState> = app.state();
            let Ok(reg) = store::load_or_default(&state.registry_path, &state.domain_suffix) else {
                continue;
            };
            let desktop = state.preferences_snapshot().desktop_notifications;
            let mut center = state
                .notifications
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            center.scan(&reg, &app, desktop);
        }
    });
}

/// Persisted filename under `<data_dir>/PortBay/`.
const FILENAME: &str = "notifications.json";

/// Cap on stored notifications. Old entries fall off the end so the file can't
/// grow without bound across long sessions; the bell is a recency surface.
const MAX_ITEMS: usize = 200;

/// What kind of agent activity a notification represents. The frontend maps
/// these to tones (comment → info, warning → warn, blocked → error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationKind {
    /// An agent left a comment on the card.
    Comment,
    /// An agent moved the card to Blocked (it hit a problem).
    Blocked,
    /// PortBay flagged a problem on the card (e.g. acceptance check failed).
    Warning,
}

#[cfg(feature = "tasks")]
impl NotificationKind {
    /// One-line desktop-banner verb for this kind.
    fn banner_verb(self) -> &'static str {
        match self {
            NotificationKind::Comment => "commented on",
            NotificationKind::Blocked => "blocked",
            NotificationKind::Warning => "flagged",
        }
    }
}

/// One surfaced piece of agent activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    /// Deterministic id (`project|card|at|action`) — also the dedupe key.
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub card_id: String,
    pub card_title: String,
    pub kind: NotificationKind,
    /// The agent's name when known (e.g. `claude`), else `None` for System.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Comment text or the note attached to the audit entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// ISO-8601 timestamp of the originating audit entry.
    pub at: String,
    /// Unix millis when PortBay recorded the notification (display ordering).
    pub created_ms: u64,
    pub read: bool,
}

/// Durable state: the per-project scan cursor plus the rolling item list.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    /// project_id → the highest audit `at` already considered. Prevents both
    /// re-notifying old activity and replaying a project's whole history the
    /// first time we ever scan it.
    cursors: HashMap<String, String>,
    /// Newest-first.
    items: Vec<Notification>,
}

/// Owns the persisted store + an in-memory mtime cache (so unchanged audit logs
/// are skipped cheaply each tick). Lives behind a `Mutex` in `AppState`.
pub struct NotificationCenter {
    path: Option<PathBuf>,
    store: Store,
    /// project_id → last-seen audit-file mtime; lets a tick skip projects whose
    /// log hasn't changed. In-memory only (not persisted). Board scanner only.
    #[cfg(feature = "tasks")]
    mtimes: HashMap<String, SystemTime>,
}

impl NotificationCenter {
    /// Load from disk, falling back to an empty center on any error (boot must
    /// not depend on this file being intact).
    pub fn load() -> Self {
        let path = data_path();
        let store = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<Store>(&raw).ok())
            .unwrap_or_default();
        Self {
            path,
            store,
            #[cfg(feature = "tasks")]
            mtimes: HashMap::new(),
        }
    }

    /// The current notifications, newest-first.
    pub fn list(&self) -> Vec<Notification> {
        self.store.items.clone()
    }

    /// Mark one notification read. No-op if the id is unknown.
    pub fn mark_read(&mut self, id: &str) {
        let mut changed = false;
        for n in &mut self.store.items {
            if n.id == id && !n.read {
                n.read = true;
                changed = true;
            }
        }
        if changed {
            self.persist();
        }
    }

    /// Mark every notification read.
    pub fn mark_all_read(&mut self) {
        let mut changed = false;
        for n in &mut self.store.items {
            if !n.read {
                n.read = true;
                changed = true;
            }
        }
        if changed {
            self.persist();
        }
    }

    /// Drop all notifications (history wipe). Cursors are kept so cleared
    /// activity doesn't get re-notified on the next scan.
    pub fn clear(&mut self) {
        if !self.store.items.is_empty() {
            self.store.items.clear();
            self.persist();
        }
    }

    /// Scan every registered project's audit log, surfacing any new notable
    /// agent activity. `desktop_enabled` mirrors the user's preference; when on,
    /// watched (`subscribed`) cards also get a native banner. Board scanner only.
    #[cfg(feature = "tasks")]
    pub fn scan(&mut self, reg: &Registry, app: &AppHandle, desktop_enabled: bool) {
        let mut fresh: Vec<Notification> = Vec::new();
        for project in reg.list_projects() {
            let pid = project.id.as_str().to_string();
            let path = project.path.as_path();
            let audit_file = crate::context::paths::audit_path(path);

            // Skip projects whose audit log hasn't changed since last tick.
            if let Ok(meta) = std::fs::metadata(&audit_file) {
                if let Ok(mtime) = meta.modified() {
                    if self.mtimes.get(&pid) == Some(&mtime) {
                        continue;
                    }
                    self.mtimes.insert(pid.clone(), mtime);
                }
            } else {
                // No audit file yet — nothing to scan.
                continue;
            }

            let entries = match audit::read(path) {
                Ok(e) => e,
                Err(_) => continue,
            };

            // First time we see this project: anchor the cursor at the latest
            // entry so we never replay history into a notification flood.
            let Some(cursor) = self.store.cursors.get(&pid).cloned() else {
                let anchor = entries.last().map(|e| e.at.clone()).unwrap_or_default();
                self.store.cursors.insert(pid.clone(), anchor);
                continue;
            };

            let mut max_at = cursor.clone();
            for entry in &entries {
                if entry.at.as_str() < cursor.as_str() {
                    continue;
                }
                if entry.at.as_str() > max_at.as_str() {
                    max_at = entry.at.clone();
                }
                let Some(classified) = classify(entry) else {
                    continue;
                };
                // Dedupe key. Blocked + Warning share one key per (card, at) so an
                // acceptance failure — which logs both an agent `move→Blocked` and
                // a System reason `note` at the same timestamp — collapses into a
                // single notification rather than two. Comments key separately.
                let id = dedupe_id(&pid, &entry.card_id, &entry.at, classified.kind);
                if self.store.items.iter().any(|n| n.id == id) {
                    continue; // already surfaced in a prior scan
                }
                // Already produced this tick: keep one, upgrading it with the more
                // specific reason (the System note carries "acceptance check failed";
                // the agent move that preceded it has no body).
                if let Some(existing) = fresh.iter_mut().find(|n| n.id == id) {
                    if existing.body.is_none() && classified.body.is_some() {
                        existing.kind = classified.kind;
                        existing.body = classified.body;
                    }
                    continue;
                }
                // Resolve the card for a human title + watch state; fall back to
                // the id if it was deleted between the write and this scan.
                let (card_title, subscribed) = match board::read_card(path, &entry.card_id) {
                    Ok(pc) => (pc.card.title, pc.card.subscribed),
                    Err(_) => (entry.card_id.clone(), false),
                };
                let notif = Notification {
                    id,
                    project_id: pid.clone(),
                    project_name: project.name.clone(),
                    card_id: entry.card_id.clone(),
                    card_title,
                    kind: classified.kind,
                    agent: classified.agent,
                    body: classified.body,
                    at: entry.at.clone(),
                    created_ms: now_ms(),
                    read: false,
                };
                if desktop_enabled && subscribed {
                    notify_desktop(&notif);
                }
                fresh.push(notif);
            }
            self.store.cursors.insert(pid, max_at);
        }

        if fresh.is_empty() {
            return;
        }
        // Newest-first; trim to the cap.
        for n in fresh.iter().rev() {
            self.store.items.insert(0, n.clone());
        }
        if self.store.items.len() > MAX_ITEMS {
            self.store.items.truncate(MAX_ITEMS);
        }
        self.persist();
        // Emit oldest-first so the frontend's prepend ends up newest-first.
        for n in fresh.iter().rev() {
            let _ = app.emit(NOTIFICATIONS_CHANNEL, n);
        }
    }

    /// Persist atomically (temp file + rename), matching `Preferences::save`.
    /// Best-effort: a write failure is logged, never propagated.
    fn persist(&self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&self.store) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// What `classify` extracts from a notable audit entry.
#[cfg(feature = "tasks")]
struct Classified {
    kind: NotificationKind,
    agent: Option<String>,
    body: Option<String>,
}

/// Stable dedupe id / event key for a notification. Blocked and Warning share a
/// key (so the two audit entries an acceptance failure writes at one timestamp
/// collapse into a single notification); comments get their own. The id is also
/// the cross-scan dedupe key, so the same event is never surfaced twice.
#[cfg(feature = "tasks")]
fn dedupe_id(project_id: &str, card_id: &str, at: &str, kind: NotificationKind) -> String {
    let action = match kind {
        NotificationKind::Comment => "comment",
        NotificationKind::Blocked | NotificationKind::Warning => "blocked",
    };
    format!("{project_id}|{card_id}|{at}|{action}")
}

/// Decide whether an audit entry is notable, and how. Returns `None` for
/// everything that isn't an agent comment, an agent block, or a System warning
/// — happy paths stay silent.
#[cfg(feature = "tasks")]
fn classify(entry: &AuditEntry) -> Option<Classified> {
    let agent_name = match &entry.actor {
        Actor::Agent { agent, .. } => Some(agent.clone()),
        _ => None,
    };
    let is_agent = agent_name.is_some();

    match entry.action.as_str() {
        "comment" if is_agent => Some(Classified {
            kind: NotificationKind::Comment,
            agent: agent_name,
            body: entry.note.clone(),
        }),
        "move" if is_agent && entry.to == Some(BoardStatus::Blocked) => Some(Classified {
            kind: NotificationKind::Blocked,
            agent: agent_name,
            body: entry.note.clone(),
        }),
        // PortBay-recorded problem (e.g. acceptance check failed → Blocked).
        "note" if matches!(entry.actor, Actor::System) && entry.to == Some(BoardStatus::Blocked) => {
            Some(Classified {
                kind: NotificationKind::Warning,
                agent: None,
                body: entry.note.clone(),
            })
        }
        _ => None,
    }
}

/// Native macOS banner for a watched card, fire-and-forget. Matches the
/// `osascript` approach used elsewhere in the app; no-op off macOS.
#[cfg(feature = "tasks")]
fn notify_desktop(n: &Notification) {
    #[cfg(target_os = "macos")]
    {
        let who = n.agent.as_deref().unwrap_or("PortBay");
        let body = format!("{who} {} “{}”", n.kind.banner_verb(), n.card_title)
            .replace(['"', '\\'], "'");
        let script = format!("display notification \"{body}\" with title \"PortBay\"");
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = n;
}

/// `<data_dir>/PortBay/notifications.json`, creating the dir on first call.
/// `None` if the platform has no data dir (notifications then stay in-memory).
fn data_path() -> Option<PathBuf> {
    let mut dir = dirs::data_dir()?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(FILENAME))
}

#[cfg(feature = "tasks")]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(all(test, feature = "tasks"))]
mod tests {
    use super::*;

    fn agent_entry(action: &str, to: Option<BoardStatus>, note: Option<&str>) -> AuditEntry {
        AuditEntry {
            at: "2026-05-30T12:00:00Z".into(),
            card_id: "t_1".into(),
            action: action.into(),
            from: None,
            to,
            actor: Actor::agent("r_1", "claude"),
            note: note.map(|s| s.to_string()),
        }
    }

    #[test]
    fn agent_comment_is_notable() {
        let c = classify(&agent_entry("comment", None, Some("done with auth"))).unwrap();
        assert_eq!(c.kind, NotificationKind::Comment);
        assert_eq!(c.agent.as_deref(), Some("claude"));
        assert_eq!(c.body.as_deref(), Some("done with auth"));
    }

    #[test]
    fn agent_block_is_notable() {
        let c = classify(&agent_entry("move", Some(BoardStatus::Blocked), Some("missing key"))).unwrap();
        assert_eq!(c.kind, NotificationKind::Blocked);
    }

    #[test]
    fn happy_paths_are_silent() {
        // Agent finishing (Done) or sending to Review must not notify.
        assert!(classify(&agent_entry("move", Some(BoardStatus::Done), None)).is_none());
        assert!(classify(&agent_entry("move", Some(BoardStatus::Review), None)).is_none());
        assert!(classify(&agent_entry("move", Some(BoardStatus::Todo), None)).is_none());
        assert!(classify(&agent_entry("claim", None, None)).is_none());
    }

    #[test]
    fn human_comment_is_silent() {
        let mut e = agent_entry("comment", None, Some("note to self"));
        e.actor = Actor::Human;
        assert!(classify(&e).is_none());
    }

    #[test]
    fn blocked_and_warning_share_a_dedupe_key() {
        // The agent move→Blocked and the System reason note an acceptance
        // failure writes at the same timestamp must collapse to one event…
        let mv = dedupe_id("p", "t_1", "2026-05-30T12:00:00Z", NotificationKind::Blocked);
        let note = dedupe_id("p", "t_1", "2026-05-30T12:00:00Z", NotificationKind::Warning);
        assert_eq!(mv, note);
        // …while a comment at the same instant stays a distinct notification.
        let comment = dedupe_id("p", "t_1", "2026-05-30T12:00:00Z", NotificationKind::Comment);
        assert_ne!(mv, comment);
    }

    #[test]
    fn system_block_note_is_a_warning() {
        let mut e = agent_entry("note", Some(BoardStatus::Blocked), Some("acceptance check failed"));
        e.actor = Actor::System;
        let c = classify(&e).unwrap();
        assert_eq!(c.kind, NotificationKind::Warning);
        assert!(c.agent.is_none());
    }
}
