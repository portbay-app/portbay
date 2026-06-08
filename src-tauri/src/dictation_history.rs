//! Recent "dictate anywhere" transcripts — the never-lose-a-dictation net.
//!
//! A failed or misdirected paste used to be the end of the words: the
//! transcript went to the target app and nowhere else, so an app that ate
//! the synthetic ⌘V (secure fields, some VMs) or a focus slip destroyed the
//! dictation. Every shipping dictation tool keeps a short history for
//! exactly this reason (freeflow's "Paste Again" + menu list, FluidVoice's
//! history view); this module is PortBay's: a small ring of recent raw
//! transcripts, persisted across restarts, surfaced two ways —
//!
//! - the tray menu's "Paste Last Dictation" item (re-delivers into the
//!   frontmost app, or onto the clipboard when that's PortBay itself), and
//! - the Smart Dictation settings panel's recent list (copy / clear).
//!
//! Privacy posture: entries are raw transcripts the user spoke into OTHER
//! apps, stored locally under the app-data dir like every preference,
//! capped at [`CAP`] entries, one click to clear. Nothing leaves the
//! machine. Only system-wide ("anywhere") sessions are recorded — in-app
//! dictations land in fields PortBay owns, where the text survives on its
//! own (and ⌘Z already covers the rewrite layer).

use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// Keep the last N dictations. Small on purpose: this is a rescue net, not
/// an archive — matching freeflow's 10-item menu, with headroom.
const CAP: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// Monotonic per-run-unique id (max existing + 1 on insert).
    pub id: u64,
    /// Unix epoch milliseconds of the session end.
    pub at_ms: u64,
    /// The text that was actually delivered to the app — the polished
    /// rewrite when "Polish dictation everywhere" produced one, otherwise the
    /// raw transcript.
    pub text: String,
    /// The pre-polish transcript, kept ONLY when a rewrite changed the text,
    /// so an over-eager polish stays recoverable (paste-again can fall back
    /// to it). `None` when nothing polished it — `text` is already the raw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// The app the words were dictated into, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    /// Whether the paste was delivered (false = it failed and the words
    /// only survive here / on the clipboard).
    pub inserted: bool,
}

/// In-memory store, lazily hydrated from disk on first touch. `None` =
/// not loaded yet. Newest entry LAST (chronological); `list` reverses.
static STORE: Lazy<Mutex<Option<Vec<HistoryEntry>>>> = Lazy::new(|| Mutex::new(None));

fn history_path() -> Option<std::path::PathBuf> {
    // Tests must never touch (or hydrate from) the user's real history.
    #[cfg(test)]
    {
        Some(std::env::temp_dir().join(format!(
            "portbay-dictation-history-test-{}.json",
            std::process::id()
        )))
    }
    #[cfg(not(test))]
    {
        dirs::data_dir().map(|d| d.join("PortBay").join("dictation-history.json"))
    }
}

fn load_from_disk() -> Vec<HistoryEntry> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<HistoryEntry>>(&bytes).ok())
        .unwrap_or_default()
}

/// Best-effort persist — a write failure costs cross-restart durability,
/// not the in-memory net, so it's logged and swallowed.
fn persist(entries: &[HistoryEntry]) {
    let Some(path) = history_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_vec_pretty(entries) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(error = %e, "dictation: history persist failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "dictation: history encode failed"),
    }
}

fn with_store<T>(f: impl FnOnce(&mut Vec<HistoryEntry>) -> T) -> T {
    let mut slot = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let entries = slot.get_or_insert_with(load_from_disk);
    f(entries)
}

/// Record one finished session. Caller refreshes the tray item afterwards
/// (`crate::tray::refresh_dictation_item`) — kept out of here so the store
/// has no Tauri dependency and tests stay plain.
pub fn record(text: &str, raw: Option<String>, app_name: Option<String>, inserted: bool) {
    let at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    with_store(|entries| {
        let id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        entries.push(HistoryEntry {
            id,
            at_ms,
            text: text.to_string(),
            raw,
            app_name,
            inserted,
        });
        if entries.len() > CAP {
            let excess = entries.len() - CAP;
            entries.drain(..excess);
        }
        persist(entries);
    });
}

/// All entries, newest first (display order).
pub fn list() -> Vec<HistoryEntry> {
    with_store(|entries| {
        let mut out = entries.clone();
        out.reverse();
        out
    })
}

/// The most recent transcript, if any.
pub fn latest() -> Option<HistoryEntry> {
    with_store(|entries| entries.last().cloned())
}

/// Entry by id (the paste command's targeted form).
pub fn get(id: u64) -> Option<HistoryEntry> {
    with_store(|entries| entries.iter().find(|e| e.id == id).cloned())
}

/// Drop everything, memory and disk.
pub fn clear() {
    with_store(|entries| {
        entries.clear();
        persist(entries);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // The store is process-global and the harness runs tests on threads —
    // serialize the two tests on a lock, and clear() between phases.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn record_list_latest_clear_roundtrip() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        assert!(list().is_empty());
        assert!(latest().is_none());

        record("first words", None, Some("Notes".into()), true);
        record("second words", Some("secnd wrds".into()), None, false);

        let all = list();
        assert_eq!(all.len(), 2);
        // Newest first.
        assert_eq!(all[0].text, "second words");
        assert!(!all[0].inserted);
        // The pre-polish transcript rides along when a rewrite changed it.
        assert_eq!(all[0].raw.as_deref(), Some("secnd wrds"));
        assert_eq!(all[1].text, "first words");
        assert_eq!(all[1].raw, None);
        assert_eq!(all[1].app_name.as_deref(), Some("Notes"));

        let last = latest().expect("latest after record");
        assert_eq!(last.text, "second words");
        assert_eq!(get(last.id).expect("get by id").text, "second words");

        // Ids are unique and increasing.
        assert!(all[0].id > all[1].id);

        clear();
        assert!(list().is_empty());
    }

    #[test]
    fn ring_caps_at_limit_dropping_oldest() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        for i in 0..(CAP + 5) {
            record(&format!("entry {i}"), None, None, true);
        }
        let all = list();
        assert_eq!(all.len(), CAP);
        // The oldest five fell off; the newest survives at the front.
        assert_eq!(all[0].text, format!("entry {}", CAP + 4));
        assert_eq!(all[all.len() - 1].text, "entry 5");
        clear();
    }
}
