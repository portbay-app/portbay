//! Unified dictation Context Store — the single resolver BOTH biasing seams read.
//!
//! Before this module, term sources were siloed: the rewrite merged custom
//! terms + learned jargon + (name/hostname-only) project terms at rewrite time,
//! and the recognizer was never biased at all. That let a misheard project name
//! slip through unrecoverably (the trigger bug: dictating "PortBay" into a fresh
//! app transcribed it as "port bay", and only the rewrite — Polish-gated — could
//! have fixed it).
//!
//! The fix is one resolution: every term source is gathered, ordered by
//! priority, and handed to whoever needs it. The recognizer takes a larger slice
//! ([`RECOGNIZER_CAP`]) for its bias prompt; the rewrite takes its smaller cap.
//! Because both consume the SAME ordering from the SAME sources, the two seams
//! never drift — a term biased into recognition is always available to the
//! rewrite for spelling, and vice versa.
//!
//! Sources, in priority order (the cap drops from the tail):
//!   1. user custom terms (stated intent beats every inference),
//!   2. profile/person (the account display name — see [`profile_terms`]),
//!   3. surface-local terms (what's visible where the user is dictating),
//!   4. learned personal jargon (per-context/per-project ranked — `dictation_vocab`),
//!   5. project terms (name, hostname, stack, runtime, services — [`project_vocab`]).
//!
//! Privacy posture is unchanged: every source is local, terms only, never prose.
//! Resolution does file I/O (registry + entitlement cache + learned store), so
//! callers run it OFF the main thread (see `[[tauri-async-worker-starvation]]`).

use std::collections::{HashMap, HashSet};

use crate::dictation::squash;
use crate::registry::types::{Project, ProjectType};

/// How many resolved terms the recognizer bias may carry — larger than the
/// rewrite's `VOCAB_CAP` because an ASR prompt tolerates a longer glossary than
/// a small rewrite model does. The Swift side further caps the TOKEN count it
/// derives from these (the Whisper prompt-context ceiling).
pub const RECOGNIZER_CAP: usize = 64;

// ---------------------------------------------------------------------------
// Project vocabulary (enriched)
// ---------------------------------------------------------------------------

/// A keyword for a project's stack from its kind — the genuinely useful,
/// occasionally-manglable ones. `Static`/`Custom` carry no stack signal, so
/// they contribute none. Derived from the Debug name (which is the variant
/// identifier) so a new `ProjectType` variant doesn't need a new arm here.
fn project_kind_keyword(kind: ProjectType) -> Option<String> {
    let name = format!("{kind:?}");
    // Defensive: a future data-carrying variant would Debug as `Name(..)`.
    let word = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match word.as_str() {
        "" | "static" | "custom" => None,
        other => Some(other.to_string()),
    }
}

/// The biasable terms one project contributes: its name + hostname (what ASR
/// reliably mangles), plus the stack/runtime/service identifiers the spec calls
/// for. Trimmed, blanks dropped; dedup + cap happen later in the merge. Pure, so
/// the enrichment is unit-testable without a registry.
pub fn project_vocab(project: &Project) -> Vec<String> {
    let mut terms = vec![project.name.clone(), project.hostname.clone()];
    if let Some(keyword) = project_kind_keyword(project.kind) {
        terms.push(keyword);
    }
    if let Some(runtime) = &project.runtime {
        if !runtime.lang.trim().is_empty() {
            terms.push(runtime.lang.clone());
        }
    }
    terms.extend(project.services.iter().cloned());
    if let Some(workspace) = &project.workspace {
        if !workspace.package.trim().is_empty() {
            terms.push(workspace.package.clone());
        }
    }
    terms.extend(project.tags.iter().cloned());
    terms
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Profile / person
// ---------------------------------------------------------------------------

/// The profile/person vocabulary source: the user's own name, so dictation
/// renders it correctly cased. Sourced from the signed account (the §10 decision
/// — `Account.display_name`, falling back to `login` only when it isn't an
/// email). A no-op when signed out / anonymous: an absent profile source is
/// never an error, just no profile term.
pub fn profile_terms() -> Vec<String> {
    profile_terms_from(crate::entitlements::current().account)
}

/// Pure core of [`profile_terms`], split out so the display-name vs login-vs-email
/// selection is testable without a cached entitlement.
fn profile_terms_from(account: Option<crate::entitlements::Account>) -> Vec<String> {
    let Some(account) = account else {
        return Vec::new();
    };
    let name = account
        .display_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        // `login` is an email for email-auth accounts and a handle for GitHub —
        // an email isn't a name worth biasing, so only fall back to a non-email
        // login.
        .or_else(|| {
            let login = account.login.trim();
            (!login.is_empty() && !login.contains('@')).then(|| login.to_string())
        });
    name.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// The raw term sources to merge, already gathered from their stores. Kept as a
/// struct so the priority order lives in exactly one place ([`resolve`]) and the
/// recognizer and rewrite can't accidentally order them differently.
pub struct Sources {
    pub custom: Vec<String>,
    pub profile: Vec<String>,
    /// Surface-local terms (the field/card the user is dictating into). Empty for
    /// the recognizer (no live text exists yet at session arm).
    pub surface: Vec<String>,
    pub learned: Vec<String>,
    pub projects: Vec<String>,
}

/// Merge the sources into one ordered list, highest priority first. Dedup + cap
/// are the caller's job (the rewrite runs `dictation::push_vocabulary`, the
/// recognizer runs [`dedup_cap`]) — both over THIS ordering, so the two biasing
/// seams stay consistent.
pub fn resolve(sources: Sources) -> Vec<String> {
    let Sources {
        custom,
        profile,
        surface,
        learned,
        projects,
    } = sources;
    let mut out = Vec::with_capacity(
        custom.len() + profile.len() + surface.len() + learned.len() + projects.len(),
    );
    out.extend(custom);
    out.extend(profile);
    out.extend(surface);
    out.extend(learned);
    out.extend(projects);
    out
}

/// Trim, drop blanks, dedup (case-insensitively, first occurrence wins), and cap
/// — the recognizer's consumer of [`resolve`]'s ordering. Mirrors what
/// `push_vocabulary` does for the rewrite, kept separate because the recognizer's
/// cap differs and it has no prompt to append to.
pub fn dedup_cap(terms: Vec<String>, cap: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    terms
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
        .take(cap)
        .collect()
}

/// Whether to actually SEND the resolved bias to the recognizer. Default ON —
/// the 2026-06-08 "empty transcripts with a bias prompt" regression was
/// re-diagnosed as a MODEL capability problem, not a code bug: it reproduced
/// only on `whisper-large-v3-turbo`, whose distilled decoder (like
/// Distil-Whisper's) was trained without previous-text conditioning, so a
/// `<|startofprev|>` prompt derails it — upstream guidance for turbo/distil
/// is exactly "don't condition on previous text". Those families are now
/// excluded per-model in [`engine_supports_text_bias`] instead of gating the
/// feature off for everyone. `PORTBAY_STT_BIAS=0` is the field kill switch if
/// a regression ever shows on a supposedly-safe model.
pub fn recognizer_bias_enabled() -> bool {
    !matches!(
        std::env::var("PORTBAY_STT_BIAS").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

/// Whether an STT engine can take a text-bias prompt. WhisperKit accepts
/// `DecodingOptions.promptTokens`; FluidAudio/Parakeet's TDT decoder has no
/// text-prompt seam, so it degrades to rewrite-only — we never send a bias the
/// engine can't apply (the engine-capability gate, §5.4). Keyed off the catalog
/// id (the only engine signal the Rust side has; the catalog lives in the
/// sidecar). Conservative: only the known-supporting family returns true —
/// and within Whisper, `turbo`/`distil` decoders are excluded: they were
/// distilled WITHOUT previous-text conditioning and return empty/degenerate
/// output when a `<|startofprev|>` prompt is attached (observed live on
/// large-v3-turbo 2026-06-08; matches Distil-Whisper's own guidance).
/// tiny/base/small are the models WhisperKit's prompt tests cover.
pub fn engine_supports_text_bias(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    id.contains("whisper") && !id.contains("turbo") && !id.contains("distil")
}

// ---------------------------------------------------------------------------
// Always-on term correction (the Polish-off safety net)
// ---------------------------------------------------------------------------

/// Smallest squashed term length a MULTI-word window may collapse to. Long
/// enough that "red is" → "redis" (5 chars) can't fire by accident, while
/// "port bay" → "PortBay" (7) still does. Single-word fixes are exempt (same
/// letters, just casing/punctuation).
const MULTIWORD_MIN_SQUASH: usize = 6;

/// The result of a term-correction pass — the corrected text plus the local
/// counters the instrumentation hooks record.
pub struct Correction {
    pub text: String,
    /// Substitutions made: a known term the recognizer got wrong that we fixed
    /// (a recognizer "miss" we recovered downstream).
    pub applied: usize,
    /// Known terms already spelled exactly right in the input (a recognizer
    /// "hit") — the denominator for a local known-term accuracy signal.
    pub already_correct: usize,
}

/// A conservative, deterministic spelling fix for the Polish-OFF anywhere path.
/// The recognizer may still mishear a known term — or be an engine that can't
/// take a bias prompt at all (Parakeet) — so this is the safety net: for each
/// resolved term, any run of 1–3 spoken words whose squash EXACTLY equals the
/// term's squash is replaced with the term's canonical spelling.
///
/// Exact-squash only (no fuzzy edit distance) so it can never mangle unrelated
/// speech: "port bay" → "PortBay" because the letters match exactly, but
/// ordinary prose is untouched. Whitespace (including the newlines voice
/// commands insert) and wrapping punctuation are preserved. Multi-word collapses
/// are gated by [`MULTIWORD_MIN_SQUASH`] so short coincidences can't fire.
pub fn correct_terms(text: &str, terms: &[String]) -> Correction {
    // squashed-term -> canonical spelling (first term at a given squash wins).
    let mut by_squash: HashMap<String, String> = HashMap::new();
    for term in terms {
        let term = term.trim();
        let squashed = squash(term);
        if squashed.chars().count() < 3 {
            continue;
        }
        by_squash
            .entry(squashed)
            .or_insert_with(|| term.to_string());
    }
    if by_squash.is_empty() {
        return Correction {
            text: text.to_string(),
            applied: 0,
            already_correct: 0,
        };
    }

    // Tokenize into (leading-whitespace, word-core) pairs, preserving the exact
    // whitespace so reconstruction is byte-faithful apart from corrected spans.
    let mut tokens: Vec<(String, String)> = Vec::new();
    let mut lead = String::new();
    let mut core = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !core.is_empty() {
                tokens.push((std::mem::take(&mut lead), std::mem::take(&mut core)));
            }
            lead.push(ch);
        } else {
            core.push(ch);
        }
    }
    let trailing_ws = if core.is_empty() {
        lead // text ended on whitespace (no final word)
    } else {
        tokens.push((std::mem::take(&mut lead), std::mem::take(&mut core)));
        String::new()
    };

    let mut out: Vec<(String, String)> = Vec::with_capacity(tokens.len());
    let mut applied = 0usize;
    let mut already_correct = 0usize;
    let mut i = 0;
    while i < tokens.len() {
        let mut matched_width = 0;
        for width in (1..=3).rev() {
            if i + width > tokens.len() {
                continue;
            }
            let joined = tokens[i..i + width]
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let squashed = squash(&joined);
            if squashed.chars().count() < 3 {
                continue;
            }
            if width >= 2 && squashed.chars().count() < MULTIWORD_MIN_SQUASH {
                continue;
            }
            let Some(canonical) = by_squash.get(&squashed) else {
                continue;
            };
            // Rebuild the original span (inner whitespace included) so we only
            // count/replace when the canonical spelling actually differs.
            let original_span: String = tokens[i..i + width]
                .iter()
                .enumerate()
                .map(|(k, (ws, c))| {
                    if k == 0 {
                        c.clone()
                    } else {
                        format!("{ws}{c}")
                    }
                })
                .collect();
            // Preserve wrapping punctuation around the matched span.
            let first = &tokens[i].1;
            let last = &tokens[i + width - 1].1;
            let lead_punct: String = first.chars().take_while(|c| !c.is_alphanumeric()).collect();
            let trail_punct: String = {
                let rev: String = last
                    .chars()
                    .rev()
                    .take_while(|c| !c.is_alphanumeric())
                    .collect();
                rev.chars().rev().collect()
            };
            let replacement = format!("{lead_punct}{canonical}{trail_punct}");
            if replacement == original_span {
                // Already spelled right — keep the span verbatim, count the hit.
                out.extend_from_slice(&tokens[i..i + width]);
                already_correct += 1;
            } else {
                out.push((tokens[i].0.clone(), replacement));
                applied += 1;
            }
            matched_width = width;
            break;
        }
        if matched_width == 0 {
            out.push(tokens[i].clone());
            i += 1;
        } else {
            i += matched_width;
        }
    }

    let mut result = String::with_capacity(text.len());
    for (lead, core) in &out {
        result.push_str(lead);
        result.push_str(core);
    }
    result.push_str(&trailing_ws);
    Correction {
        text: result,
        applied,
        already_correct,
    }
}

// ---------------------------------------------------------------------------
// Local instrumentation (data only — no UI yet)
// ---------------------------------------------------------------------------

/// Process-local counters for the dictation intelligence loop. Deliberately
/// in-memory and side-effect-free to read: a future debug surface or the
/// reserved `stats` block can drain [`snapshot`] without this module taking on a
/// persistence dependency. No prose, no PII — pure counts.
pub mod instrument {
    use std::sync::atomic::{AtomicU64, Ordering};

    static TERMS_BIASED: AtomicU64 = AtomicU64::new(0);
    static CORRECTIONS_APPLIED: AtomicU64 = AtomicU64::new(0);
    static KNOWN_TERMS_PRESENT: AtomicU64 = AtomicU64::new(0);

    /// Record that `n` terms were sent to the recognizer as a bias prompt.
    pub fn record_bias(n: usize) {
        TERMS_BIASED.fetch_add(n as u64, Ordering::Relaxed);
    }

    /// Record one correction pass: `applied` recovered recognizer misses,
    /// `present` known terms already spelled correctly (hits).
    pub fn record_correction(applied: usize, present: usize) {
        CORRECTIONS_APPLIED.fetch_add(applied as u64, Ordering::Relaxed);
        KNOWN_TERMS_PRESENT.fetch_add(present as u64, Ordering::Relaxed);
    }

    /// `(terms_biased, corrections_applied, known_terms_present)` since launch.
    pub fn snapshot() -> (u64, u64, u64) {
        (
            TERMS_BIASED.load(Ordering::Relaxed),
            CORRECTIONS_APPLIED.load(Ordering::Relaxed),
            KNOWN_TERMS_PRESENT.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlements::Account;

    fn account(display: Option<&str>, login: &str) -> Account {
        Account {
            github_id: None,
            login: login.to_string(),
            display_name: display.map(str::to_string),
            avatar_url: None,
        }
    }

    #[test]
    fn profile_prefers_display_name_then_non_email_login() {
        assert_eq!(
            profile_terms_from(Some(account(Some("Nour Beiruti"), "nour@example.com"))),
            vec!["Nour Beiruti".to_string()]
        );
        // No display name, GitHub-handle login → use the handle.
        assert_eq!(
            profile_terms_from(Some(account(None, "tribalhouse"))),
            vec!["tribalhouse".to_string()]
        );
        // No display name, email login → no profile term (an email isn't a name).
        assert!(profile_terms_from(Some(account(None, "nour@example.com"))).is_empty());
        // Signed out → nothing, never an error.
        assert!(profile_terms_from(None).is_empty());
        // Blank display name falls through to the login rule.
        assert_eq!(
            profile_terms_from(Some(account(Some("   "), "handle"))),
            vec!["handle".to_string()]
        );
    }

    #[test]
    fn resolve_keeps_priority_order() {
        let merged = resolve(Sources {
            custom: vec!["refactor".into()],
            profile: vec!["Nour".into()],
            surface: vec!["card-title".into()],
            learned: vec!["russh-sftp".into()],
            projects: vec!["PortBay".into()],
        });
        assert_eq!(
            merged,
            vec!["refactor", "Nour", "card-title", "russh-sftp", "PortBay"]
        );
    }

    #[test]
    fn dedup_cap_is_case_insensitive_and_order_preserving() {
        let capped = dedup_cap(
            vec![
                "  PortBay ".into(),
                "portbay".into(), // dup (case-insensitive)
                "".into(),        // blank dropped
                "russh-sftp".into(),
                "tailwind".into(),
            ],
            2,
        );
        assert_eq!(capped, vec!["PortBay", "russh-sftp"]);
    }

    #[test]
    fn engine_capability_gate() {
        // Standard Whisper decoders take a prompt.
        assert!(engine_supports_text_bias("whisper-tiny"));
        assert!(engine_supports_text_bias("whisper-base"));
        assert!(engine_supports_text_bias("whisper-small"));
        // turbo/distil decoders were distilled without previous-text
        // conditioning — a <|startofprev|> prompt yields empty output
        // (the 2026-06-08 live regression) → no bias sent.
        assert!(!engine_supports_text_bias("whisper-large-v3-turbo"));
        assert!(!engine_supports_text_bias("whisper-distil-large-v3"));
        // Parakeet (TDT) has no text-prompt seam → no bias sent.
        assert!(!engine_supports_text_bias("parakeet-tdt-v3"));
        assert!(!engine_supports_text_bias(""));
    }

    #[test]
    fn recognizer_bias_defaults_on_with_kill_switch() {
        // No parallel test touches PORTBAY_STT_BIAS (same pattern as the
        // stt.rs PORTBAY_STT_BIN test).
        std::env::remove_var("PORTBAY_STT_BIAS");
        assert!(recognizer_bias_enabled());
        std::env::set_var("PORTBAY_STT_BIAS", "0");
        assert!(!recognizer_bias_enabled());
        std::env::set_var("PORTBAY_STT_BIAS", "1");
        assert!(recognizer_bias_enabled());
        std::env::remove_var("PORTBAY_STT_BIAS");
    }

    #[test]
    fn correct_terms_fixes_misheard_known_terms() {
        let terms = vec!["PortBay".to_string(), "russh-sftp".to_string()];
        // Multi-word mishearing → canonical spelling; period preserved.
        let c = correct_terms("deploy port bay now.", &terms);
        assert_eq!(c.text, "deploy PortBay now.");
        assert_eq!(c.applied, 1);
        // Single-word case fix counts as applied (the recognizer missed casing).
        let c = correct_terms("ship portbay today", &terms);
        assert_eq!(c.text, "ship PortBay today");
        assert_eq!(c.applied, 1);
    }

    #[test]
    fn correct_terms_preserves_correct_spelling_and_whitespace() {
        let terms = vec!["PortBay".to_string()];
        // Already correct → untouched, counted as a hit, newlines preserved.
        let c = correct_terms("PortBay ships\nthe release", &terms);
        assert_eq!(c.text, "PortBay ships\nthe release");
        assert_eq!(c.applied, 0);
        assert_eq!(c.already_correct, 1);
    }

    #[test]
    fn correct_terms_does_not_mangle_unrelated_or_short_collisions() {
        // "redis" is too short (5) for a multi-word collapse: "red is" stays.
        let terms = vec!["redis".to_string()];
        let c = correct_terms("the light is red is it on", &terms);
        assert_eq!(c.text, "the light is red is it on");
        assert_eq!(c.applied, 0);
        // No known term anywhere → exact passthrough.
        let c = correct_terms("just some ordinary prose here", &[]);
        assert_eq!(c.text, "just some ordinary prose here");
        assert_eq!(c.applied, 0);
    }
}
