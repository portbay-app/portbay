//! Smart Dictation IPC — rewrite a dictated transcript, cancel one in
//! flight, probe the provider for the settings panel.
//!
//! Failure philosophy: a rewrite that can't happen is a *normal outcome*,
//! not an error. Every command here resolves `Ok` with a status the frontend
//! can branch on; throwing would route through `safeInvoke`'s toast and nag
//! the user every time Ollama isn't running, when the correct behaviour is
//! to silently keep the raw transcript they can already see in the field.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::Notify;

use crate::dictation::{
    build_edit_prompt, build_edit_user, build_prompt, build_user, output_budget,
    sanitize_output, InputSource, ProviderConfig, ProviderStatus, RewriteContext, RewriteError,
    RewriteMode, RewriteProvider,
};
use crate::error::AppResult;
use crate::state::AppState;

/// In-flight rewrites by frontend-generated request id. A `Notify` per
/// request rather than aborting tasks: the rewrite future observes the
/// signal at its own await points and resolves `Cancelled` cleanly (same
/// pattern as `ssh::agent`'s chat abort).
static IN_FLIGHT: Lazy<Mutex<HashMap<String, Arc<Notify>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn register(request_id: &str) -> Arc<Notify> {
    let cancel = Arc::new(Notify::new());
    IN_FLIGHT
        .lock()
        .expect("dictation registry poisoned")
        .insert(request_id.to_string(), cancel.clone());
    cancel
}

fn unregister(request_id: &str) {
    IN_FLIGHT
        .lock()
        .expect("dictation registry poisoned")
        .remove(request_id);
}

/// Unregister-on-drop for an in-flight rewrite. The explicit unregister at the
/// end of a rewrite covers the happy path; this guard covers the case where
/// the whole `run_rewrite` future is dropped before completing — the anywhere
/// path races it against a timeout (`tokio::time::timeout`), and a dropped
/// future would otherwise leave its id in the registry forever.
struct InFlightGuard<'a> {
    request_id: &'a str,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        unregister(self.request_id);
    }
}

/// What a rewrite attempt produced. `rewritten` is the only status that
/// carries text; everything else tells the frontend to keep the raw
/// transcript (with `detail` for logs/status copy, never toasts).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteOutcome {
    /// "rewritten" | "failed" | "cancelled" | "no_model"
    pub status: &'static str,
    pub text: Option<String>,
    pub detail: Option<String>,
}

/// Workspace vocabulary for the rewrite prompts — the project terms ASR
/// reliably mangles ("port bay landing" → `portbay-landing`), now enriched
/// beyond name+hostname with stack/runtime/service identifiers (see
/// `dictation_context::project_vocab`). Registry read is a small local file;
/// rewrites are infrequent and already gated, so per-call loading keeps this
/// stateless. Best-effort: an unreadable registry just means no vocabulary.
fn workspace_vocabulary(state: &AppState) -> Vec<String> {
    registry_project_terms(&state.registry_path, &state.domain_suffix)
}

/// Shared registry → project-term extraction, path-based so it runs both on the
/// rewrite path (via `workspace_vocabulary`, holding `&AppState`) and inside the
/// recognizer resolver's `spawn_blocking` closure (which can't carry the state).
/// Best-effort: an unreadable registry just means no project terms.
fn registry_project_terms(registry_path: &std::path::Path, domain_suffix: &str) -> Vec<String> {
    let Ok(registry) = crate::registry::store::load_or_default(registry_path, domain_suffix) else {
        return Vec::new();
    };
    registry
        .list_projects()
        .iter()
        .flat_map(crate::dictation_context::project_vocab)
        .collect()
}

/// How many user-curated custom terms enter the prompt. Sub-cap of the
/// 40-term prompt cap: custom terms sit at the head of the merge, so an
/// unbounded list would starve every automatic source below it.
pub const CUSTOM_TERMS_CAP: usize = 12;

/// The user-curated custom terms from Settings → AI → Smart Dictation —
/// plain words and niche brands no automatic source can supply ("refactor",
/// "Shopify"; the harvests only collect identifier-shaped tokens by design).
fn custom_terms(state: &AppState) -> Vec<String> {
    sanitize_custom_terms(state.preferences_snapshot().dictation.custom_terms)
}

/// Trim, drop blanks, cap — order kept (the user ranked them by listing
/// order, and the prompt cap drops from the tail).
fn sanitize_custom_terms(terms: Vec<String>) -> Vec<String> {
    terms
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .take(CUSTOM_TERMS_CAP)
        .collect()
}

/// Merge the vocabulary sources for the rewrite, through the unified Context
/// Store so the rewrite and the recognizer bias draw from ONE ordering (see
/// `dictation_context::resolve` for the priority rationale). `push_vocabulary`
/// then dedups and drops from the tail at `VOCAB_CAP`. The added profile source
/// (the account display name) is what lets a dictated personal name come back
/// correctly cased.
fn merged_vocabulary(
    extra: Option<Vec<String>>,
    context: Option<RewriteContext>,
    project: Option<&str>,
    state: &AppState,
) -> Vec<String> {
    crate::dictation_context::resolve(crate::dictation_context::Sources {
        custom: custom_terms(state),
        profile: crate::dictation_context::profile_terms(),
        surface: extra.unwrap_or_default(),
        learned: crate::dictation_vocab::top_terms(&state.registry_path, context, project),
        projects: workspace_vocabulary(state),
    })
}

/// Resolve the recognizer bias snapshot for a capture session — the SAME
/// sources and ordering the rewrite uses (`merged_vocabulary`), capped to
/// `dictation_context::RECOGNIZER_CAP`. There is no live transcript at session
/// arm, so the surface source is empty. The registry / entitlement / learned-store
/// reads run OFF the calling worker (`spawn_blocking`) so a cold capture start
/// never stalls the async runtime (see `[[tauri-async-worker-starvation]]`).
pub async fn recognizer_terms(
    state: &AppState,
    context: Option<RewriteContext>,
    project: Option<String>,
) -> Vec<String> {
    // Cheap in-memory snapshot first; the file I/O moves to the blocking pool.
    let custom = custom_terms(state);
    let registry_path = state.registry_path.clone();
    let domain_suffix = state.domain_suffix.clone();
    tokio::task::spawn_blocking(move || {
        let merged = crate::dictation_context::resolve(crate::dictation_context::Sources {
            custom,
            profile: crate::dictation_context::profile_terms(),
            surface: Vec::new(),
            learned: crate::dictation_vocab::top_terms(
                &registry_path,
                context,
                project.as_deref(),
            ),
            projects: registry_project_terms(&registry_path, &domain_suffix),
        });
        crate::dictation_context::dedup_cap(merged, crate::dictation_context::RECOGNIZER_CAP)
    })
    .await
    .unwrap_or_default()
}

/// Webview-side breadcrumbs for the rewrite layer, surfaced through the
/// backend log (the WKWebView console is invisible in `tauri dev` output).
/// Debug-level only — silent at the production default filter.
#[tauri::command]
pub fn dictation_trace(msg: String) {
    tracing::debug!(%msg, "dictation frontend");
}

/// Rewrite one dictated transcript. The frontend passes only the text the
/// dictation session *inserted* (it diffs the field around the session), so
/// surrounding user-typed content never reaches the model.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn dictation_rewrite(
    state: State<'_, AppState>,
    request_id: String,
    text: String,
    context: RewriteContext,
    extra_vocabulary: Option<Vec<String>>,
    // Registry project id when the surface belongs to one (task cards) —
    // scopes the learned vocabulary's ranking and learning. Absent for
    // project-less surfaces (SSH workspace).
    project: Option<String>,
    mode: RewriteMode,
    provider: ProviderConfig,
    // What transcribed the speech, pinned at session start on the frontend.
    // Absent (older frontends / the Writing Tools path) = `Raw`, which builds
    // the shipped v16 prompt unchanged. See `InputSource`.
    source: Option<InputSource>,
) -> AppResult<RewriteOutcome> {
    Ok(run_rewrite(
        state.inner(),
        &request_id,
        &text,
        context,
        extra_vocabulary,
        project.as_deref(),
        mode,
        &provider,
        source.unwrap_or_default(),
        // In-app surfaces splice the result atomically into a field the user
        // can already see the raw text in — live token preview adds little and
        // risks flicker (see the Gap 2 scope note), so no progress sink here.
        None,
    )
    .await)
}

/// The rewrite engine, shared by the `dictation_rewrite` command (in-app
/// surfaces) and the system-wide paste path (`dictation_anywhere`). Routing
/// everything through here keeps the no-invention guards, vocabulary merge,
/// sanitizer, and jargon-learning identical on both — there is deliberately
/// no second rewrite path. Never errors: every outcome resolves to a status
/// the caller branches on (a rewrite that can't happen keeps the raw text).
#[allow(clippy::too_many_arguments)]
pub async fn run_rewrite(
    state: &AppState,
    request_id: &str,
    text: &str,
    context: RewriteContext,
    extra_vocabulary: Option<Vec<String>>,
    project: Option<&str>,
    mode: RewriteMode,
    provider: &ProviderConfig,
    source: InputSource,
    // Display-only streaming sink (see `dictation::ProgressSink`); `None` for
    // the atomic in-app path, `Some` for the anywhere notch's "Polishing…"
    // preview. The final pasted text is validated/sanitized regardless.
    progress: Option<crate::dictation::ProgressSink<'_>>,
) -> RewriteOutcome {
    // Observability (debug-only — failures stay silent in the UI by design,
    // but diagnosing "field stayed raw" needs to distinguish frontend-skip
    // from provider trouble).
    tracing::debug!(
        ?context,
        ?mode,
        ?source,
        provider = %provider.kind,
        chars = text.chars().count(),
        "dictation rewrite requested"
    );

    // Defence in depth: the frontend gates on length too, but never run a
    // model over something too short to be worth touching ("yes", "ok").
    if text.trim().chars().count() < 8 {
        return RewriteOutcome {
            status: "failed",
            text: None,
            detail: Some("transcript too short to rewrite".into()),
        };
    }

    let cancel = register(request_id);
    let _in_flight = InFlightGuard { request_id };
    // Anchor-filter the vocabulary to terms plausibly spoken: an unfiltered
    // block measurably perturbs vocab-irrelevant rewrites (probed t20/t21 —
    // see `anchored_vocabulary`). The spelling reference only earns its
    // prompt space when the speech might actually match a term.
    let vocabulary = crate::dictation::anchored_vocabulary(
        text,
        merged_vocabulary(extra_vocabulary, Some(context), project, state),
    );
    // The exact model input, for offline reproduction (debug-only, local log;
    // this subsystem keeps breadcrumbs deliberately). Three rounds of "user's
    // in-app output diverges from every offline probe" were undiagnosable
    // because the REAL transcript + vocab list were unknowable after the
    // fact — paste these two lines into scripts/probe-afm/ and the rewrite
    // is exactly reproducible (greedy sampling, no randomness).
    tracing::debug!(transcript = %text, vocabulary = ?vocabulary, ?context, "dictation rewrite input");
    let system = build_prompt(
        mode,
        context,
        &vocabulary,
        crate::dictation::PromptFlavor::for_provider(&provider.kind),
        source,
    );
    // Framed, not raw — the "Transcript:" prefix keeps instruction-shaped
    // speech being cleaned rather than answered (see `build_user`).
    let user = build_user(text);
    let budget = output_budget(text);
    let result = provider
        .build()
        .rewrite(&system, &user, budget, &cancel, progress)
        .await;
    unregister(request_id);
    // The explicit unregister above keeps the happy-path timing unchanged;
    // the guard is now a redundant no-op (its purpose is the dropped-future
    // case, which never reaches this line).
    drop(_in_flight);

    let outcome = outcome_from(result, text);
    // Vocabulary terms are a spelling reference, never content: an output
    // term with no spoken anchor means the model filled a gap it couldn't
    // parse with workspace jargon (observed live on a garbled transcript) —
    // keep the raw transcript instead.
    if outcome.status == "rewritten" {
        if let Some(term) =
            crate::dictation::vocabulary_injection(outcome.text.as_deref().unwrap_or(""), text, &vocabulary)
        {
            tracing::debug!(%term, "dictation rewrite injected an unspoken vocabulary term; keeping raw");
            return RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some(format!("output used unspoken vocabulary term '{term}'")),
            };
        }
        // Invented-fact guard: a color/day/month in the output that was
        // never spoken is fabrication (observed live: "from blue to yellow"
        // for speech that only said red and yellow). Keep the raw transcript.
        if let Some(word) = crate::dictation::introduced_fact_word(
            outcome.text.as_deref().unwrap_or(""),
            text,
        ) {
            tracing::debug!(%word, "dictation rewrite invented a fact word; keeping raw");
            return RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some(format!("output introduced unspoken fact word '{word}'")),
            };
        }
        // Invented-identifier guard: a technical-shaped token in the output
        // with no spoken anchor (observed live: a fabricated filename).
        if let Some(token) = crate::dictation::invented_technical_token(
            outcome.text.as_deref().unwrap_or(""),
            text,
            &vocabulary,
        ) {
            tracing::debug!(%token, "dictation rewrite invented a technical token; keeping raw");
            return RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some(format!("output introduced unspoken technical token '{token}'")),
            };
        }
    }
    tracing::debug!(status = outcome.status, "dictation rewrite finished");

    // Learn the user's jargon from the POLISHED text (correct spellings —
    // never from raw transcripts, whose ASR manglings must not enter the
    // store). Off the response path: the splice must not wait on disk.
    if outcome.status == "rewritten" {
        if let Some(text) = outcome.text.clone() {
            let registry_path = state.registry_path.clone();
            let project = project.map(str::to_string);
            tokio::task::spawn_blocking(move || {
                crate::dictation_vocab::learn(&registry_path, context, project.as_deref(), &text);
            });
        }
    }
    outcome
}

/// Map a provider result + sanitization to the wire outcome. `reference` is
/// the text the output is validated against (the transcript for rewrites,
/// the selection for edits — it bounds runaway growth).
fn outcome_from(result: Result<String, RewriteError>, reference: &str) -> RewriteOutcome {
    match result {
        Ok(raw) => match sanitize_output(&raw, reference) {
            Some(clean) => RewriteOutcome {
                status: "rewritten",
                text: Some(clean),
                detail: None,
            },
            None => RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some("model output failed validation".into()),
            },
        },
        Err(RewriteError::Cancelled) => RewriteOutcome {
            status: "cancelled",
            text: None,
            detail: None,
        },
        Err(RewriteError::NoModel) => RewriteOutcome {
            status: "no_model",
            text: None,
            detail: Some("no model available for the configured provider".into()),
        },
        Err(RewriteError::Provider(e)) | Err(RewriteError::BadOutput(e)) => {
            tracing::debug!(error = %e, "dictation rewrite failed; keeping raw transcript");
            RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some(e),
            }
        }
    }
}

/// Voice Edit Mode: the user selected text and dictated an *instruction*
/// about it ("make this more concise"). Transform `selection` per
/// `instruction`; same failure philosophy as rewriting — anything but
/// "rewritten" tells the frontend to restore the pre-edit field, so the
/// selection is never lost to a failed edit.
#[tauri::command]
pub async fn dictation_edit(
    state: State<'_, AppState>,
    request_id: String,
    selection: String,
    instruction: String,
    extra_vocabulary: Option<Vec<String>>,
    project: Option<String>,
    provider: ProviderConfig,
) -> AppResult<RewriteOutcome> {
    if selection.trim().is_empty() || instruction.trim().chars().count() < 3 {
        return Ok(RewriteOutcome {
            status: "failed",
            text: None,
            detail: Some("selection or instruction too short to edit".into()),
        });
    }

    let cancel = register(&request_id);
    // Edit Mode has no surface context — learned terms rank on global
    // frequency (plus the project boost when the surface has one; edits
    // don't feed the store either way). Anchor against selection AND
    // instruction — a term may be referenced by either.
    let vocabulary = crate::dictation::anchored_vocabulary(
        &format!("{selection} {instruction}"),
        merged_vocabulary(extra_vocabulary, None, project.as_deref(), &state),
    );
    // Same reproduction breadcrumb as the rewrite path (debug-only).
    tracing::debug!(%instruction, vocabulary = ?vocabulary, "dictation edit input");
    let system = build_edit_prompt(&vocabulary);
    let user = build_edit_user(&selection, &instruction);
    // Budget scales with the selection — the instruction shapes, the
    // selection sizes the output.
    let budget = output_budget(&selection);
    let result = provider
        .build()
        .rewrite(&system, &user, budget, &cancel, None)
        .await;
    unregister(&request_id);

    let outcome = outcome_from(result, &selection);
    // Same injection guard as rewriting — the anchor here is the selection
    // plus the instruction (a term may come from either).
    if outcome.status == "rewritten" {
        let anchor_text = format!("{selection} {instruction}");
        if let Some(term) = crate::dictation::vocabulary_injection(
            outcome.text.as_deref().unwrap_or(""),
            &anchor_text,
            &vocabulary,
        ) {
            tracing::debug!(%term, "dictation edit injected an unspoken vocabulary term; restoring");
            return Ok(RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some(format!("output used unspoken vocabulary term '{term}'")),
            });
        }
        // Invented-fact guard, anchored against selection + instruction —
        // either may legitimately carry the color/day/month.
        if let Some(word) = crate::dictation::introduced_fact_word(
            outcome.text.as_deref().unwrap_or(""),
            &anchor_text,
        ) {
            tracing::debug!(%word, "dictation edit invented a fact word; restoring");
            return Ok(RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some(format!("output introduced unspoken fact word '{word}'")),
            });
        }
        // Invented-identifier guard, same anchor.
        if let Some(token) = crate::dictation::invented_technical_token(
            outcome.text.as_deref().unwrap_or(""),
            &anchor_text,
            &vocabulary,
        ) {
            tracing::debug!(%token, "dictation edit invented a technical token; restoring");
            return Ok(RewriteOutcome {
                status: "failed",
                text: None,
                detail: Some(format!("output introduced unspoken technical token '{token}'")),
            });
        }
    }
    // "The model returned the original" is the documented unclear-instruction
    // fallback — report it as kept, not as a rewrite, so the UI doesn't offer
    // a meaningless Undo.
    if outcome.status == "rewritten" && outcome.text.as_deref() == Some(selection.trim()) {
        return Ok(RewriteOutcome {
            status: "failed",
            text: None,
            detail: Some("instruction made no change".into()),
        });
    }
    Ok(outcome)
}

/// Undo-aware learning: the user ⌘Z'd a rewrite, so the polish was REJECTED —
/// demote the terms it reinforced (the splice's `learn` ran when the pipeline
/// accepted it; leaving the counts up would let rejected polish train the
/// store). `text` is the rewritten segment that was undone, `context` the
/// surface it was learned under. Same spawn-blocking pattern as `learn`: the
/// undo splice never waits on disk. Best-effort and idempotent — unknown
/// terms are no-ops.
#[tauri::command]
pub async fn dictation_unlearn(
    state: State<'_, AppState>,
    text: String,
    context: RewriteContext,
    project: Option<String>,
) -> AppResult<()> {
    let registry_path = state.registry_path.clone();
    tokio::task::spawn_blocking(move || {
        crate::dictation_vocab::unlearn(&registry_path, context, project.as_deref(), &text);
    });
    Ok(())
}

/// Forget everything dictation has learned — the privacy "reset learned
/// vocabulary" affordance (Settings → AI → Smart Dictation). Clears the whole
/// local learned store (terms + reserved style/stats blocks); the user-curated
/// custom terms (a separate pref) are left alone. Best-effort; the small file
/// delete rides `spawn_blocking` for symmetry with learn/unlearn so the command
/// never waits on disk.
#[tauri::command]
pub async fn dictation_reset_vocabulary(state: State<'_, AppState>) -> AppResult<()> {
    let registry_path = state.registry_path.clone();
    tokio::task::spawn_blocking(move || {
        crate::dictation_vocab::reset(&registry_path);
    });
    Ok(())
}

/// Cancel an in-flight rewrite. Best-effort and idempotent: an unknown id
/// (already finished, never started) is a no-op.
#[tauri::command]
pub async fn dictation_rewrite_cancel(request_id: String) -> AppResult<()> {
    if let Some(cancel) = IN_FLIGHT
        .lock()
        .expect("dictation registry poisoned")
        .get(&request_id)
    {
        cancel.notify_waiters();
    }
    Ok(())
}

/// Provider liveness + installed models, for Settings → AI → Smart Dictation.
#[tauri::command]
pub async fn dictation_provider_status(provider: ProviderConfig) -> AppResult<ProviderStatus> {
    Ok(provider.build().status().await)
}

/// Page the rewrite model in ahead of need — the frontend fires this when a
/// dictation session STARTS so the rewrite at session end doesn't pay
/// first-use model load. Best-effort and silent; see `dictation::prewarm`.
#[tauri::command]
pub async fn dictation_prewarm(provider: ProviderConfig) -> AppResult<()> {
    crate::dictation::prewarm(&provider);
    Ok(())
}

/// Running, dock-visible apps for the per-app rewrite-context editor (Settings
/// → Smart Dictation, the "Polish dictation everywhere" per-app overrides).
/// Empty off macOS or if the main-thread hop fails — the editor just shows no
/// picker rather than erroring.
#[tauri::command]
pub async fn dictation_list_apps(app: AppHandle) -> AppResult<Vec<crate::typing::AppInfo>> {
    #[cfg(target_os = "macos")]
    {
        // Two-stage so the webview never stalls: the main thread only does the
        // cheap NSWorkspace enumeration (+ native icon-PNG grab), then the
        // per-icon downscale (a 1024² decode + resample each) runs on a
        // blocking worker. Doing the resampling on the main-thread hop is what
        // made the "Polish everywhere" toggle jank when it loaded this list.
        let (tx, rx) = tokio::sync::oneshot::channel();
        let dispatched = app.run_on_main_thread(move || {
            let mtm = objc2::MainThreadMarker::new().expect("run_on_main_thread is the main thread");
            let _ = tx.send(crate::typing::collect_running_apps(mtm));
        });
        if dispatched.is_err() {
            return Ok(Vec::new());
        }
        let raw = rx.await.unwrap_or_default();
        let infos = tokio::task::spawn_blocking(move || crate::typing::finalize_app_infos(raw))
            .await
            .unwrap_or_default();
        Ok(infos)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_terms_are_trimmed_filtered_and_capped() {
        let raw: Vec<String> = vec!["  refactor  ", "", "   ", "Tailwind"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(sanitize_custom_terms(raw), vec!["refactor", "Tailwind"]);

        // Cap drops from the tail — the user's listing order is the ranking.
        let many: Vec<String> = (0..20).map(|i| format!("term-{i:02}")).collect();
        let capped = sanitize_custom_terms(many);
        assert_eq!(capped.len(), CUSTOM_TERMS_CAP);
        assert_eq!(capped.first().map(String::as_str), Some("term-00"));
        assert_eq!(capped.last().map(String::as_str), Some("term-11"));
    }
}
