//! Learned dictation vocabulary — the personal-jargon layer.
//!
//! The static vocabulary sources (registry project names, surface tokens —
//! see `commands::dictation::merged_vocabulary`) only know what's visible
//! RIGHT NOW. This store learns what the user actually says over time: after
//! every accepted rewrite, the technical terms in the polished text are
//! recorded with the surface context they were dictated into. The more
//! someone dictates "redeploy `portbay-landing`" into agent prompts, the
//! higher that term ranks in future agent-prompt rewrites — so the spelling
//! reference adapts to each user's own task language.
//!
//! Safe by construction:
//!   • Terms come from REWRITTEN output only — spellings the model preserved
//!     verbatim or corrected via vocabulary — never from raw transcripts,
//!     whose ASR manglings are exactly what we don't want to learn. (The
//!     manglings also fail the technical-shape test: "port bay landing" is
//!     three dictionary words.)
//!   • Only identifier-shaped tokens are kept (same heuristic as the
//!     frontend's `$lib/dictation/vocabulary`); prose never enters the store.
//!   • The store is a small local JSON next to the registry; nothing leaves
//!     the machine, matching the rest of the dictation privacy posture.
//!
//! Ranking: context-weighted frequency with a recency window. A term used in
//! the SAME context scores double; terms unused for 90 days fall out of the
//! prompt (but stay in the store until pruned by the size cap).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::dictation::RewriteContext;

/// How many learned terms one rewrite may contribute to the prompt — leaves
/// room under `dictation::VOCAB_CAP` for surface + registry terms.
pub const LEARNED_CAP: usize = 12;

/// Store ceiling; lowest-scoring terms are pruned past this.
const MAX_TERMS: usize = 400;

/// Terms unused this long stop appearing in prompts (stale jargon — renamed
/// projects, finished initiatives).
const RECENCY_WINDOW_SECS: u64 = 90 * 24 * 60 * 60;

/// Most terms a single rewrite may record — one giant dictation must not
/// flood the store.
const LEARN_BATCH_CAP: usize = 24;

/// On-disk schema version. v1 store files carried no version field at all, so
/// a missing key (see [`default_schema_version`]) reads as 1 and triggers
/// [`migrate`]. Bump this — and add a migration arm — whenever the persisted
/// shape changes.
const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Softening constant for the saturating salience curve (see [`salience`]).
/// At `count == K` a term sits at weight 0.5; the curve approaches but never
/// reaches 1.0, so no single term's weight pins the scale.
const SALIENCE_K: f32 = 8.0;

/// A term's 0..1 learned salience from its raw count — a smooth, bounded
/// stand-in for the richer weighting a later phase will compute (decay,
/// per-context promotion). Maintained on every write so the field is live, not
/// a placeholder; ranking itself still uses the proven integer [`score`].
fn salience(count: u32) -> f32 {
    let c = count as f32;
    (c / (c + SALIENCE_K)).clamp(0.0, 1.0)
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct LearnedTerm {
    /// Total accepted-rewrite occurrences.
    count: u32,
    /// Unix seconds of the last occurrence.
    last_used: u64,
    /// Unix seconds of the FIRST occurrence (schema v2). v1 files backfill it
    /// to `last_used` on [`migrate`]; `serde(default)` keeps them loading.
    #[serde(default)]
    first_seen: u64,
    /// Saturating 0..1 salience (schema v2), maintained on every learn/unlearn
    /// — see [`salience`]. The foundation writes it; the later
    /// style-memory / recognizer-promotion phase consumes it.
    #[serde(default)]
    weight: f32,
    /// Occurrences per rewrite context (wire names, e.g. "terminal_command").
    #[serde(default)]
    contexts: HashMap<String, u32>,
    /// Occurrences per project (registry project ids) — terms learned while a
    /// project's surfaces were active rank higher inside that project.
    /// `serde(default)` keeps pre-project store files loading unchanged.
    #[serde(default)]
    projects: HashMap<String, u32>,
}

/// Schema version of a store file with no `schema_version` key — the v1 shape.
fn default_schema_version() -> u32 {
    1
}

/// The empty reserved block (`{}`) used for `style` / `stats` on a fresh or
/// migrated store.
fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Serialize, Deserialize)]
struct Store {
    /// Persisted schema version. Absent in v1 files → 1 (see
    /// [`default_schema_version`]), which [`migrate`] lifts to
    /// [`CURRENT_SCHEMA_VERSION`].
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    terms: HashMap<String, LearnedTerm>,
    /// Reserved for the style-memory phase (phrasing/layout preferences learned
    /// from edits). Defined empty now so a later phase extends the document
    /// without another migration.
    #[serde(default = "empty_object")]
    style: serde_json::Value,
    /// Reserved for local accuracy/correction metrics (recognizer hit/miss,
    /// corrections applied). Counters live in-process today; this is their
    /// future durable home.
    #[serde(default = "empty_object")]
    stats: serde_json::Value,
}

impl Default for Store {
    /// A fresh store is born at the current version with empty reserved blocks
    /// — only files read from disk can be older (and get [`migrate`]d).
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            terms: HashMap::new(),
            style: empty_object(),
            stats: empty_object(),
        }
    }
}

/// Lift a freshly-loaded store to [`CURRENT_SCHEMA_VERSION`] in place. Idempotent
/// and silent: a store already at the current version is left untouched. The
/// upgraded shape reaches disk on the next [`persist`] (a learn/unlearn) — a
/// read-only session keeps the v1 file until something writes, which is fine
/// because migration is deterministic and re-run on every load.
fn migrate(store: &mut Store) {
    if store.schema_version >= CURRENT_SCHEMA_VERSION {
        return;
    }
    // v1 → v2: backfill the new per-term fields from what v1 recorded.
    for term in store.terms.values_mut() {
        if term.first_seen == 0 {
            term.first_seen = term.last_used;
        }
        if term.weight == 0.0 {
            term.weight = salience(term.count);
        }
    }
    if store.style.is_null() {
        store.style = empty_object();
    }
    if store.stats.is_null() {
        store.stats = empty_object();
    }
    store.schema_version = CURRENT_SCHEMA_VERSION;
}

/// In-memory cache per store path (keyed so tests with temp paths don't
/// collide). Loaded lazily, written through on every learn.
static CACHE: Lazy<Mutex<HashMap<PathBuf, Store>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// The store lives next to the registry JSON.
fn store_path(registry_path: &Path) -> PathBuf {
    registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("dictation_vocabulary.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn context_key(context: RewriteContext) -> &'static str {
    match context {
        RewriteContext::GeneralNote => "general_note",
        RewriteContext::TodoTask => "todo_task",
        RewriteContext::AgentPrompt => "agent_prompt",
        RewriteContext::TerminalCommand => "terminal_command",
        RewriteContext::GitCommit => "git_commit",
        RewriteContext::DeployNote => "deploy_note",
        RewriteContext::BugReport => "bug_report",
    }
}

// ---------------------------------------------------------------------------
// Technical-term extraction (Rust mirror of $lib/dictation/vocabulary.ts —
// keep the heuristics in sync)
// ---------------------------------------------------------------------------

/// Whether one token looks like a technical identifier worth remembering:
/// paths, flags, dotted/hyphenated/underscored names, camelCase, versions,
/// letter+digit mixes. Plain words are excluded.
fn is_technical(token: &str) -> bool {
    let len = token.chars().count();
    if !(3..=64).contains(&len) {
        return false;
    }
    // Pure numbers (incl. dotted/colon/comma-separated).
    if token.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ':')) {
        return false;
    }
    // Flags: -f, --force, --dry-run.
    if let Some(rest) = token.strip_prefix('-') {
        let rest = rest.strip_prefix('-').unwrap_or(rest);
        let mut chars = rest.chars();
        if chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return true;
        }
    }
    // Internal structural punctuation: alnum [-_./:@] alnum.
    let chars: Vec<char> = token.chars().collect();
    for w in chars.windows(3) {
        if w[0].is_ascii_alphanumeric()
            && matches!(w[1], '-' | '_' | '.' | '/' | ':' | '@')
            && w[2].is_ascii_alphanumeric()
        {
            return true;
        }
    }
    let has_alpha = chars.iter().any(|c| c.is_ascii_alphabetic());
    let has_digit = chars.iter().any(|c| c.is_ascii_digit());
    if has_alpha && has_digit {
        return true;
    }
    // camelCase / PascalCase hump.
    for w in chars.windows(2) {
        if w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase() {
            return true;
        }
    }
    false
}

/// Extract technical terms from text, in order, deduped case-insensitively.
/// `pub(crate)`: also the token source for `dictation::invented_technical_token`
/// (output tokens that look like identifiers but were never spoken).
pub(crate) fn extract_terms(text: &str, cap: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut terms = Vec::new();
    for raw in text.split_whitespace() {
        if terms.len() >= cap {
            break;
        }
        // Trim wrapping punctuation, keep internal structure.
        let token = raw
            .trim_start_matches(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '_' | '~' | '/' | '@' | '-'))
            })
            .trim_end_matches(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '+' | '~'))
            });
        if !is_technical(token) {
            continue;
        }
        if seen.insert(token.to_lowercase()) {
            terms.push(token.to_string());
        }
    }
    terms
}

// ---------------------------------------------------------------------------
// Store operations
// ---------------------------------------------------------------------------

fn with_store<R>(registry_path: &Path, f: impl FnOnce(&mut Store) -> R) -> R {
    let path = store_path(registry_path);
    let mut cache = CACHE.lock().expect("dictation vocab cache poisoned");
    let store = cache.entry(path.clone()).or_insert_with(|| {
        let mut store: Store = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        migrate(&mut store);
        store
    });
    f(store)
}

/// Best-effort persist; a failed write costs re-learning, never an error.
fn persist(registry_path: &Path, store: &Store) {
    let path = store_path(registry_path);
    match serde_json::to_vec(store) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                tracing::debug!(error = %e, "dictation vocab: persist failed");
            }
        }
        Err(e) => tracing::debug!(error = %e, "dictation vocab: serialize failed"),
    }
}

fn score(term: &LearnedTerm, context: Option<&str>, project: Option<&str>) -> u64 {
    let context_count = context
        .and_then(|c| term.contexts.get(c))
        .copied()
        .unwrap_or(0) as u64;
    let project_count = project
        .and_then(|p| term.projects.get(p))
        .copied()
        .unwrap_or(0) as u64;
    // Same-context and same-project uses each weigh double on top of the
    // global count (rank = ctx×2 + project×2 + total).
    term.count as u64 + context_count * 2 + project_count * 2
}

/// Record the technical terms of one accepted rewrite under its context and
/// (when the surface belongs to one) its project.
pub fn learn(registry_path: &Path, context: RewriteContext, project: Option<&str>, text: &str) {
    let terms = extract_terms(text, LEARN_BATCH_CAP);
    if terms.is_empty() {
        return;
    }
    let now = now_secs();
    let ctx = context_key(context);
    with_store(registry_path, |store| {
        for term in terms {
            let entry = store.terms.entry(term).or_default();
            entry.count = entry.count.saturating_add(1);
            entry.last_used = now;
            if entry.first_seen == 0 {
                entry.first_seen = now;
            }
            *entry.contexts.entry(ctx.to_string()).or_default() += 1;
            if let Some(project) = project {
                *entry.projects.entry(project.to_string()).or_default() += 1;
            }
            entry.weight = salience(entry.count);
        }
        // Prune the long tail past the ceiling: lowest score, oldest first.
        if store.terms.len() > MAX_TERMS {
            let mut ranked: Vec<(String, u64, u64)> = store
                .terms
                .iter()
                .map(|(t, e)| (t.clone(), score(e, None, None), e.last_used))
                .collect();
            ranked.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));
            for (term, _, _) in ranked.into_iter().take(store.terms.len() - MAX_TERMS) {
                store.terms.remove(&term);
            }
        }
        persist(registry_path, store);
    });
}

/// Demote the terms of one UNDONE rewrite — the symmetric inverse of
/// [`learn`]: the user rejecting a polish (⌘Z) must not leave its terms
/// reinforced in the store. Same extractor over the same text the accepted
/// rewrite learned from, so the keys match exactly; counts floor at zero and
/// a term with no occurrences left is dropped entirely. Unknown terms are
/// no-ops (the store may have pruned them, or the rewrite never learned —
/// over-unlearning degrades to a rank drop, never an error).
pub fn unlearn(
    registry_path: &Path,
    context: RewriteContext,
    project: Option<&str>,
    text: &str,
) {
    let terms = extract_terms(text, LEARN_BATCH_CAP);
    if terms.is_empty() {
        return;
    }
    let ctx = context_key(context);
    with_store(registry_path, |store| {
        for term in terms {
            let Some(entry) = store.terms.get_mut(&term) else {
                continue;
            };
            entry.count = entry.count.saturating_sub(1);
            if let Some(count) = entry.contexts.get_mut(ctx) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    entry.contexts.remove(ctx);
                }
            }
            if let Some(project) = project {
                if let Some(count) = entry.projects.get_mut(project) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        entry.projects.remove(project);
                    }
                }
            }
            entry.weight = salience(entry.count);
            if entry.count == 0 {
                store.terms.remove(&term);
            }
        }
        persist(registry_path, store);
    });
}

/// Forget the entire learned store — the privacy "reset what dictation has
/// learned" path (Settings → AI). Drops the in-memory cache entry and deletes
/// the on-disk JSON; the next access re-creates an empty current-version store.
/// Best-effort: a missing file is success, a failed delete just leaves the
/// (now empty) cache to be re-persisted on the next learn.
pub fn reset(registry_path: &Path) {
    let path = store_path(registry_path);
    CACHE
        .lock()
        .expect("dictation vocab cache poisoned")
        .remove(&path);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::debug!(error = %e, "dictation vocab: reset delete failed"),
    }
}

/// The highest-ranked learned terms for a rewrite, same-context and
/// same-project first. `None` context (Edit Mode) ranks without the context
/// boost; `None` project (surfaces outside any project) without the project
/// boost.
pub fn top_terms(
    registry_path: &Path,
    context: Option<RewriteContext>,
    project: Option<&str>,
) -> Vec<String> {
    let cutoff = now_secs().saturating_sub(RECENCY_WINDOW_SECS);
    let ctx = context.map(context_key);
    with_store(registry_path, |store| {
        let mut ranked: Vec<(&String, u64, u64)> = store
            .terms
            .iter()
            .filter(|(_, e)| e.last_used >= cutoff)
            .map(|(t, e)| (t, score(e, ctx, project), e.last_used))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        ranked
            .into_iter()
            .take(LEARNED_CAP)
            .map(|(t, _, _)| t.clone())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_registry() -> PathBuf {
        let dir = tempfile::tempdir().expect("tempdir");
        // Leak the tempdir so the path stays valid for the test process —
        // the per-path cache means each test gets an isolated store anyway.
        let path = dir.path().join("registry.json");
        std::mem::forget(dir);
        path
    }

    #[test]
    fn extracts_identifier_shapes_only() {
        let terms = extract_terms(
            "Bump the russh-sftp version in portbay-landing, check /var/log/app.log with --dry-run please",
            24,
        );
        assert_eq!(
            terms,
            vec!["russh-sftp", "portbay-landing", "/var/log/app.log", "--dry-run"]
        );
        assert!(extract_terms("just plain words spoken here", 24).is_empty());
        assert!(extract_terms("8080 0.1.4 3,000", 24).is_empty());
    }

    #[test]
    fn learn_then_rank_prefers_same_context() {
        let reg = temp_registry();
        // "deploy-tool" used twice in terminal commands, "spec-term" once in
        // todo tasks.
        learn(&reg, RewriteContext::TerminalCommand, None, "run deploy-tool now");
        learn(&reg, RewriteContext::TerminalCommand, None, "run deploy-tool again");
        learn(&reg, RewriteContext::TodoTask, None, "write the spec-term doc");

        let terminal = top_terms(&reg, Some(RewriteContext::TerminalCommand), None);
        assert_eq!(terminal.first().map(String::as_str), Some("deploy-tool"));
        assert!(terminal.contains(&"spec-term".to_string()));

        // For todo tasks the context weighting flips the order.
        let todo = top_terms(&reg, Some(RewriteContext::TodoTask), None);
        assert_eq!(todo.first().map(String::as_str), Some("spec-term"));

        // Edit Mode (no context): global frequency wins.
        let neutral = top_terms(&reg, None, None);
        assert_eq!(neutral.first().map(String::as_str), Some("deploy-tool"));
    }

    #[test]
    fn unlearn_demotes_and_drops_rejected_terms() {
        let reg = temp_registry();
        // deploy-tool: 2× terminal + 1× todo; spec-term: 2× todo.
        learn(&reg, RewriteContext::TerminalCommand, None, "run deploy-tool now");
        learn(&reg, RewriteContext::TerminalCommand, None, "run deploy-tool again");
        learn(&reg, RewriteContext::TodoTask, None, "schedule deploy-tool today");
        learn(&reg, RewriteContext::TodoTask, None, "write the spec-term doc");
        learn(&reg, RewriteContext::TodoTask, None, "review the spec-term doc");
        assert_eq!(
            top_terms(&reg, None, None).first().map(String::as_str),
            Some("deploy-tool")
        );

        // Both terminal rewrites undone: their reinforcement must vanish —
        // rank drops strictly below the never-undone term.
        unlearn(&reg, RewriteContext::TerminalCommand, None, "run deploy-tool now");
        unlearn(&reg, RewriteContext::TerminalCommand, None, "run deploy-tool again");
        assert_eq!(
            top_terms(&reg, None, None).first().map(String::as_str),
            Some("spec-term"),
            "undone uses no longer outrank accepted ones"
        );
        // The one accepted todo use survives.
        assert!(top_terms(&reg, Some(RewriteContext::TodoTask), None)
            .contains(&"deploy-tool".to_string()));

        // Undoing the last remaining use drops the term entirely.
        unlearn(&reg, RewriteContext::TodoTask, None, "schedule deploy-tool today");
        let all = top_terms(&reg, None, None);
        assert!(!all.contains(&"deploy-tool".to_string()), "count hit zero → dropped");
        assert!(all.contains(&"spec-term".to_string()), "other terms untouched");

        // Unknown terms / over-unlearning are no-ops, never errors.
        unlearn(&reg, RewriteContext::TodoTask, None, "never-learned-term ghost-tool-9");
        unlearn(&reg, RewriteContext::TodoTask, None, "schedule deploy-tool today");
        assert!(top_terms(&reg, None, None).contains(&"spec-term".to_string()));
    }

    #[test]
    fn project_scoping_boosts_terms_inside_their_project() {
        let reg = temp_registry();
        // alpha-tool: 2 uses inside project A; beta-tool: 3 global uses with
        // no project (e.g. SSH surfaces).
        learn(&reg, RewriteContext::TodoTask, Some("proj-a"), "ship alpha-tool v1");
        learn(&reg, RewriteContext::TodoTask, Some("proj-a"), "test alpha-tool again");
        learn(&reg, RewriteContext::TodoTask, None, "run beta-tool now");
        learn(&reg, RewriteContext::TodoTask, None, "run beta-tool again");
        learn(&reg, RewriteContext::TodoTask, None, "run beta-tool more");

        // Inside project A the project boost flips the order (2 + 2×2 = 6 > 3)…
        let inside = top_terms(&reg, Some(RewriteContext::TodoTask), Some("proj-a"));
        assert!(
            inside.iter().position(|t| t == "alpha-tool")
                < inside.iter().position(|t| t == "beta-tool"),
            "project-local term must lead inside its project: {inside:?}"
        );
        // …outside any project, global frequency wins (3 > 2).
        let outside = top_terms(&reg, Some(RewriteContext::TodoTask), None);
        assert!(
            outside.iter().position(|t| t == "beta-tool")
                < outside.iter().position(|t| t == "alpha-tool"),
            "global frequency wins outside the project: {outside:?}"
        );
        // An unknown project gets no boost — same as outside.
        let other = top_terms(&reg, Some(RewriteContext::TodoTask), Some("proj-z"));
        assert!(
            other.iter().position(|t| t == "beta-tool")
                < other.iter().position(|t| t == "alpha-tool")
        );

        // Project counts unlearn symmetrically.
        unlearn(&reg, RewriteContext::TodoTask, Some("proj-a"), "ship alpha-tool v1");
        unlearn(&reg, RewriteContext::TodoTask, Some("proj-a"), "test alpha-tool again");
        assert!(
            !top_terms(&reg, Some(RewriteContext::TodoTask), Some("proj-a"))
                .contains(&"alpha-tool".to_string()),
            "both uses undone → dropped"
        );
    }

    #[test]
    fn cap_and_raw_prose_are_respected() {
        let reg = temp_registry();
        for i in 0..20 {
            learn(
                &reg,
                RewriteContext::GeneralNote, None,
                &format!("touch file-{i:02}.txt"),
            );
        }
        let terms = top_terms(&reg, Some(RewriteContext::GeneralNote), None);
        assert_eq!(terms.len(), LEARNED_CAP);
        // Prose-only learning is a no-op.
        learn(&reg, RewriteContext::GeneralNote, None, "water the plants tomorrow");
        assert!(!top_terms(&reg, None, None).iter().any(|t| t == "plants"));
    }

    #[test]
    fn persists_to_disk_next_to_registry() {
        let reg = temp_registry();
        learn(&reg, RewriteContext::GitCommit, None, "fix sftp-pane crash");
        let on_disk = std::fs::read_to_string(store_path(&reg)).expect("store written");
        assert!(on_disk.contains("sftp-pane"));
        assert!(on_disk.contains("git_commit"));
    }

    #[test]
    fn fresh_store_persists_v2_shape_and_round_trips() {
        let reg = temp_registry();
        learn(&reg, RewriteContext::GitCommit, None, "ship portbay-stt v2");
        let on_disk = std::fs::read_to_string(store_path(&reg)).expect("store written");
        // A fresh store is written at the current version with the reserved
        // blocks present, and the new per-term fields are serialized.
        assert!(on_disk.contains("\"schema_version\":2"), "got: {on_disk}");
        assert!(on_disk.contains("\"style\":{}"));
        assert!(on_disk.contains("\"stats\":{}"));
        assert!(on_disk.contains("\"first_seen\""));
        assert!(on_disk.contains("\"weight\""));
        // And it deserializes back without loss (round-trip).
        let parsed: Store = serde_json::from_str(&on_disk).expect("round-trips");
        assert_eq!(parsed.schema_version, CURRENT_SCHEMA_VERSION);
        let term = parsed.terms.get("portbay-stt").expect("term kept");
        assert_eq!(term.count, 1);
        assert!(term.first_seen > 0, "first_seen backfilled on learn");
        assert!(term.weight > 0.0 && term.weight < 1.0, "salience in (0,1)");
    }

    #[test]
    fn migrates_v1_file_silently() {
        let reg = temp_registry();
        // A hand-written v1 file: no schema_version, no first_seen/weight, and
        // the legacy `terms` map shape. It must load, migrate, and rank. Use a
        // recent `last_used` so the term stays inside the ranking recency window.
        let last_used = now_secs() - 1000;
        let v1 = format!(
            r#"{{ "terms": {{ "russh-sftp": {{ "count": 3, "last_used": {last_used},
                "contexts": {{ "git_commit": 2 }}, "projects": {{}} }} }} }}"#
        );
        std::fs::write(store_path(&reg), v1).unwrap();

        // Reading through the normal path migrates in memory: the term ranks…
        let top = top_terms(&reg, Some(RewriteContext::GitCommit), None);
        assert!(top.contains(&"russh-sftp".to_string()), "v1 term survives migration");

        // …and a subsequent write upgrades the on-disk shape to v2 with the
        // backfilled fields (first_seen ← last_used, weight ← salience(count)).
        learn(&reg, RewriteContext::GitCommit, None, "touch other-term");
        let on_disk = std::fs::read_to_string(store_path(&reg)).unwrap();
        let parsed: Store = serde_json::from_str(&on_disk).unwrap();
        assert_eq!(parsed.schema_version, CURRENT_SCHEMA_VERSION);
        let migrated = parsed.terms.get("russh-sftp").expect("kept");
        assert_eq!(migrated.first_seen, last_used, "first_seen backfilled from last_used");
        assert!((migrated.weight - salience(3)).abs() < 1e-6, "weight backfilled from count");
    }

    #[test]
    fn reset_clears_terms_and_file() {
        let reg = temp_registry();
        learn(&reg, RewriteContext::GitCommit, None, "fix sftp-pane crash");
        assert!(store_path(&reg).exists());
        assert!(!top_terms(&reg, None, None).is_empty());

        reset(&reg);

        assert!(!store_path(&reg).exists(), "store file deleted");
        assert!(top_terms(&reg, None, None).is_empty(), "in-memory store cleared");
        // The store is usable again after a reset (re-created at current version).
        learn(&reg, RewriteContext::GitCommit, None, "fresh after-reset-term");
        assert!(top_terms(&reg, None, None).contains(&"after-reset-term".to_string()));
    }
}
