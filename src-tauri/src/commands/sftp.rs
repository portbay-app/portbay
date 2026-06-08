//! SFTP file-manager commands over a saved [`SshConnection`].
//!
//! Each command resolves the connection from the registry, borrows (or opens) a
//! cached SFTP session from [`SftpManager`], and runs one operation. Sessions
//! are cached per connection so browsing doesn't re-handshake on every click.
//!
//! Transfers are whole-file (read into memory, then write) — simple and robust
//! for source trees; a streaming path for very large files is a follow-up, and
//! [`MAX_TRANSFER_BYTES`] guards against accidentally loading a huge file.
//!
//! # Local-path security model
//!
//! A compromised webview must not be able to read or write arbitrary local files
//! by supplying a crafted `local_path` to a transfer command. The defence:
//!
//! 1. The legitimate UI always picks local paths via native OS dialogs (or real
//!    OS drag-drop). Three new backend commands — `sftp_pick_upload_files`,
//!    `sftp_pick_download_dir`, `sftp_pick_save_path` — run those dialogs
//!    host-side (Rust), canonicalize the result, insert it into
//!    `AppState::sftp_approved_paths`, and return the path string.
//!
//! 2. `sftp_upload`, `sftp_download`, and both directions of `sftp_transfer`
//!    call [`ensure_local_path_approved`] before any I/O. That helper verifies
//!    the canonicalized path is an exact member of the approved set *or* a
//!    strict descendant of an approved directory entry. Anything else → the
//!    command returns [`AppError::BadInput`] before touching the filesystem.
//!
//! 3. Native OS file drops are recorded at the host level: the window's
//!    `DragDrop` event (in `lib.rs` `.on_window_event`) inserts each dropped
//!    path into the approved set before the webview sees it, so drag-upload
//!    continues to work under the approval policy.

use std::collections::HashMap;
use std::io::SeekFrom;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::commands::projects::load_registry;
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::SshConnectionId;
use crate::state::AppState;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// Channel the streaming transfer command emits progress on.
pub const SFTP_PROGRESS_CHANNEL: &str = "portbay://sftp-progress";

/// Streaming chunk size (256 KiB) — small enough to report progress smoothly,
/// large enough that per-chunk overhead is negligible.
const TRANSFER_CHUNK: usize = 256 * 1024;

/// Whole-file transfer ceiling (1 GiB). Above this we refuse rather than try to
/// buffer the whole file in memory; streaming is a future refinement.
const MAX_TRANSFER_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// POSIX mode bits (e.g. 0o644), when the server reports them.
    pub permissions: Option<u32>,
    /// Modification time, seconds since the Unix epoch, when reported.
    pub mtime_secs: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpPathInput {
    pub connection_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpRenameInput {
    pub connection_id: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpChmodInput {
    pub connection_id: String,
    pub path: String,
    /// POSIX mode bits as a number (e.g. 0o644 = 420).
    pub mode: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpWriteInput {
    pub connection_id: String,
    pub path: String,
    pub contents: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpTransferInput {
    pub connection_id: String,
    pub local_path: String,
    pub remote_path: String,
}

fn sftp_err(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("SFTP error: {e}"))
}

/// Write `bytes` to a remote path with create+truncate semantics.
///
/// The high-level `SftpSession::write` opens with a bare `WRITE` flag — it
/// neither creates reliably nor truncates, so overwriting a file with shorter
/// content would leave trailing bytes. `create()` (CREATE|TRUNCATE|WRITE) is the
/// correct "replace the file" behaviour our editor + uploads need.
async fn write_remote(sftp: &SftpSession, path: String, bytes: &[u8]) -> AppResult<()> {
    let mut file = sftp.create(path).await.map_err(sftp_err)?;
    file.write_all(bytes).await.map_err(sftp_err)?;
    file.shutdown().await.map_err(sftp_err)?;
    Ok(())
}

/// Resolve the connection, load its password if needed, and return a live SFTP
/// session (cached per connection). Incidental file ops never carry a host-key
/// interactor — the cold connect (and its trust prompt) happens in
/// [`sftp_connect`], which the file browser calls first.
async fn session(state: &State<'_, AppState>, connection_id: &str) -> AppResult<Arc<SftpSession>> {
    session_with(state, connection_id, None, None, None).await
}

/// Like [`session`], but `password_override` / `passphrase_override` are one-shot
/// secrets from the credential prompt: when present (non-blank) they're used to
/// open a new session and never persisted, taking precedence over the keychain.
/// Once a session is cached and live, later calls reuse it without a secret.
/// `interactor` surfaces an untrusted host-key decision on a cold connect.
async fn session_with(
    state: &State<'_, AppState>,
    connection_id: &str,
    password_override: Option<&str>,
    passphrase_override: Option<&str>,
    interactor: Option<Arc<dyn crate::ssh::interaction::SshInteractor>>,
) -> AppResult<Arc<SftpSession>> {
    let registry = load_registry(state)?;
    let raw = registry
        .get_ssh_connection(&SshConnectionId::new(connection_id))
        .ok_or_else(|| AppError::BadInput(format!("SSH connection `{connection_id}` not found")))?;
    // Fold in a borrowed identity (user / key / auth) before connecting.
    let conn = registry.effective_ssh_connection(raw);
    let nonblank = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let password = match nonblank(password_override) {
        Some(p) => Some(p),
        None => load_stored_password(&conn.id)?,
    };
    let passphrase = match passphrase_override {
        Some(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        // An explicit *empty* override means the user chose "Skip" on the
        // passphrase prompt: forward it as a declined passphrase (`Some("")`)
        // so the backend skips the key and asks for the password instead of
        // re-prompting — and don't silently fall back to a stored passphrase.
        Some(_) => Some(String::new()),
        None => load_stored_key_passphrase(&conn.id)?,
    };
    let proxy_password = load_stored_proxy_password(&conn.id)?;
    let mut mgr = state.sftp.lock().await;
    mgr.session_for(
        &conn,
        password.as_deref(),
        proxy_password.as_deref(),
        passphrase.as_deref(),
        interactor,
    )
    .await
    .map_err(AppError::Ssh)
}

/// Validate that `raw` refers to a local path the user chose through a
/// host-mediated dialog or a real OS drag-drop (i.e., it is present in
/// `AppState::sftp_approved_paths`).
///
/// Why here rather than in the frontend: a compromised webview can supply any
/// string as `local_path`; the approval set lives in trusted Rust state that
/// the renderer cannot write to directly. Every SFTP transfer command calls
/// this before touching the local filesystem.
///
/// Canonicalization rules:
/// - If the path exists on disk, call `std::fs::canonicalize`.
/// - If the path does not yet exist (a download destination), canonicalize the
///   parent and re-append the final component. The parent must exist; if it
///   doesn't, return `BadInput` (the path is implausible).
///
/// Approval rules:
/// - The canonical path is an exact member of the approved set, OR
/// - The canonical path is a strict descendant of an approved directory entry
///   in the set (prefix match on canonical forms — safe against `../` escapes
///   because both sides are canonical).
pub(crate) fn ensure_local_path_approved(
    state: &AppState,
    raw: &str,
) -> AppResult<std::path::PathBuf> {
    let raw_path = std::path::Path::new(raw);

    // Produce a canonical form. For a file that doesn't exist yet (download
    // destination), canonicalize the parent so we can still do a prefix check.
    let canonical = if raw_path.exists() {
        std::fs::canonicalize(raw_path)
            .map_err(|e| AppError::BadInput(format!("cannot resolve local path `{raw}`: {e}")))?
    } else {
        let parent = raw_path.parent().ok_or_else(|| {
            AppError::BadInput(format!("local path `{raw}` has no parent directory"))
        })?;
        let canon_parent = std::fs::canonicalize(parent).map_err(|_| {
            AppError::BadInput(format!(
                "local path `{raw}` was not chosen through a file dialog — \
                 pick the destination again"
            ))
        })?;
        let file_name = raw_path.file_name().ok_or_else(|| {
            AppError::BadInput(format!("local path `{raw}` has no file name component"))
        })?;
        canon_parent.join(file_name)
    };

    // Consult the approved set.
    let approved = state
        .sftp_approved_paths
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // Exact match: the user picked this exact file.
    if approved.contains(&canonical) {
        return Ok(canonical);
    }

    // Ancestor match: the user approved a directory; this path is inside it.
    // Both sides are canonical so no symlink or `../` trick can escape.
    for entry in approved.iter() {
        if canonical.starts_with(entry) && canonical != *entry {
            return Ok(canonical);
        }
    }

    Err(AppError::BadInput(format!(
        "local path `{raw}` was not chosen through a file dialog — \
         pick the destination again"
    )))
}

/// Insert a canonicalized path into the approved set, ignoring any I/O errors
/// (the file may not exist yet for save-dialog destinations — we insert the
/// canonical form produced by `ensure_local_path_approved`'s parent-canonicalize
/// branch in that case, but here we're inserting paths that definitely exist).
fn approve_path(state: &AppState, path: std::path::PathBuf) {
    state
        .sftp_approved_paths
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path);
}

/// Open a native multi-file picker (host-side, so the renderer cannot
/// influence which files are offered). Each picked path is canonicalized and
/// inserted into the session's approved set so a subsequent `sftp_upload` call
/// will succeed. Returns the picked paths as strings; empty on cancel.
///
/// Despite the `sftp_` prefix this is a generic host-approved local-file pick:
/// any flow that reads renderer-supplied local paths (SFTP uploads *and* agent
/// chat attachments via `ssh_agent_upload_path`) shares this picker so the
/// chosen paths are recorded in the approved set before any read.
///
/// The blocking dialog API must not run on an async worker thread (it spins a
/// run-loop that conflicts with tokio's multi-thread scheduler). We drive it
/// inside `spawn_blocking`.
#[tauri::command]
pub async fn sftp_pick_upload_files(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    // Clone the AppHandle so we can move it into spawn_blocking.
    let app_clone = app.clone();
    let paths: Vec<std::path::PathBuf> =
        tokio::task::spawn_blocking(move || -> Vec<std::path::PathBuf> {
            app_clone
                .dialog()
                .file()
                .blocking_pick_files()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|f| f.into_path().ok())
                .filter_map(|p| std::fs::canonicalize(&p).ok())
                .collect()
        })
        .await
        .map_err(|e| AppError::Internal(format!("dialog task panicked: {e}")))?;

    let mut result = Vec::with_capacity(paths.len());
    for p in paths {
        let s = p.to_string_lossy().into_owned();
        approve_path(&state, p);
        result.push(s);
    }
    Ok(result)
}

/// Open a native folder picker for choosing a download destination directory.
/// The chosen directory is inserted into the approved set; all files written
/// inside it during this session are automatically approved (descendant check
/// in [`ensure_local_path_approved`]). Returns `None` on cancel.
#[tauri::command]
pub async fn sftp_pick_download_dir(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<String>> {
    let app_clone = app.clone();
    let picked: Option<std::path::PathBuf> =
        tokio::task::spawn_blocking(move || -> Option<std::path::PathBuf> {
            app_clone
                .dialog()
                .file()
                .blocking_pick_folder()
                .and_then(|f| f.into_path().ok())
                .and_then(|p| std::fs::canonicalize(&p).ok())
        })
        .await
        .map_err(|e| AppError::Internal(format!("dialog task panicked: {e}")))?;

    Ok(picked.map(|p| {
        let s = p.to_string_lossy().into_owned();
        approve_path(&state, p);
        s
    }))
}

/// Open a native folder picker for choosing a local folder to upload. The
/// chosen directory is canonicalized and inserted into the approved set, so
/// every file the renderer subsequently enumerates under it (via
/// `local_walk_files`) passes the descendant check in
/// [`ensure_local_path_approved`]. Returns `None` on cancel.
#[tauri::command]
pub async fn sftp_pick_upload_dir(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<String>> {
    let app_clone = app.clone();
    let picked: Option<std::path::PathBuf> =
        tokio::task::spawn_blocking(move || -> Option<std::path::PathBuf> {
            app_clone
                .dialog()
                .file()
                .blocking_pick_folder()
                .and_then(|f| f.into_path().ok())
                .and_then(|p| std::fs::canonicalize(&p).ok())
        })
        .await
        .map_err(|e| AppError::Internal(format!("dialog task panicked: {e}")))?;

    Ok(picked.map(|p| {
        let s = p.to_string_lossy().into_owned();
        approve_path(&state, p);
        s
    }))
}

/// Ask the user — via a **host-rendered** native confirm dialog — to grant
/// upload access to renderer-named local paths (the in-app local file pane and
/// in-app drag-and-drop name paths without going through an OS picker).
///
/// This preserves the local-path security model: the renderer can *request*
/// approval for any path, but only a real user click on the native dialog
/// (which the webview cannot synthesize) inserts it into the approved set. The
/// dialog lists the exact paths being granted. Directories grant their whole
/// subtree (descendant rule in [`ensure_local_path_approved`]), and grants
/// last for the app session. Returns `true` when the user allowed.
#[tauri::command]
pub async fn sftp_request_local_access(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
    host_label: String,
) -> AppResult<bool> {
    // Canonicalize first so the user confirms the real targets (no `../` games)
    // and the approved entries match what transfer-time checks will produce.
    let mut canonical: Vec<std::path::PathBuf> = Vec::with_capacity(paths.len());
    for raw in &paths {
        let p = std::fs::canonicalize(raw)
            .map_err(|e| AppError::BadInput(format!("cannot resolve local path `{raw}`: {e}")))?;
        canonical.push(p);
    }
    if canonical.is_empty() {
        return Ok(false);
    }

    const LISTED: usize = 6;
    let mut listing: Vec<String> = canonical
        .iter()
        .take(LISTED)
        .map(|p| p.display().to_string())
        .collect();
    if canonical.len() > LISTED {
        listing.push(format!("… and {} more", canonical.len() - LISTED));
    }
    let message = format!(
        "Allow PortBay to read and upload the following to “{host_label}”?\n\n{}\n\nFolders grant everything inside them, for this session.",
        listing.join("\n")
    );

    let app_clone = app.clone();
    let allowed = tokio::task::spawn_blocking(move || {
        app_clone
            .dialog()
            .message(message)
            .title("Allow upload access?")
            .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
                "Allow".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|e| AppError::Internal(format!("dialog task panicked: {e}")))?;

    if allowed {
        for p in canonical {
            approve_path(&state, p);
        }
    }
    Ok(allowed)
}

/// Open a native save dialog for choosing a single download destination.
/// `default_name` is suggested as the file name. The chosen path is
/// canonicalized (parent dir + file name, since the file doesn't yet exist)
/// and inserted into the approved set. Returns `None` on cancel.
#[tauri::command]
pub async fn sftp_pick_save_path(
    app: AppHandle,
    state: State<'_, AppState>,
    default_name: String,
) -> AppResult<Option<String>> {
    let app_clone = app.clone();
    let default_name_clone = default_name.clone();
    let picked: Option<std::path::PathBuf> =
        tokio::task::spawn_blocking(move || -> Option<std::path::PathBuf> {
            app_clone
                .dialog()
                .file()
                .set_file_name(&default_name_clone)
                .blocking_save_file()
                .and_then(|f| f.into_path().ok())
        })
        .await
        .map_err(|e| AppError::Internal(format!("dialog task panicked: {e}")))?;

    if let Some(p) = picked {
        // The file doesn't exist yet — canonicalize the parent and re-append
        // the final component so the approved key matches what I/O will produce.
        let canonical = if p.exists() {
            std::fs::canonicalize(&p)
                .map_err(|e| AppError::BadInput(format!("cannot resolve save path: {e}")))?
        } else {
            let parent = p
                .parent()
                .ok_or_else(|| AppError::BadInput("save path has no parent directory".into()))?;
            let canon_parent = std::fs::canonicalize(parent)
                .map_err(|e| AppError::BadInput(format!("cannot resolve save directory: {e}")))?;
            let name = p
                .file_name()
                .ok_or_else(|| AppError::BadInput("save path has no file name".into()))?;
            canon_parent.join(name)
        };
        let s = canonical.to_string_lossy().into_owned();
        approve_path(&state, canonical);
        Ok(Some(s))
    } else {
        Ok(None)
    }
}

/// Open (and cache) the SFTP session for a connection, returning its home dir.
/// The file browser calls this first, wrapped in the credential prompt, so a
/// password/passphrase-needing host is asked **once** with a one-shot secret;
/// every later `sftp_*` call reuses the cached session without a prompt.
#[tauri::command]
pub async fn sftp_connect(
    state: State<'_, AppState>,
    app: AppHandle,
    connection_id: String,
    password: Option<String>,
    passphrase: Option<String>,
) -> AppResult<String> {
    let sftp = session_with(
        &state,
        &connection_id,
        password.as_deref(),
        passphrase.as_deref(),
        Some(crate::ssh::EventInteractor::shared(app)),
    )
    .await?;
    sftp.canonicalize(".").await.map_err(sftp_err)
}

fn entry_from(name: String, path: String, attrs: &FileAttributes) -> SftpEntry {
    SftpEntry {
        name,
        path,
        is_dir: attrs.is_dir(),
        is_symlink: attrs.is_symlink(),
        size: attrs.size.unwrap_or(0),
        permissions: attrs.permissions,
        mtime_secs: attrs.mtime,
    }
}

/// The connection's default/home directory (canonical absolute path of `.`).
#[tauri::command]
pub async fn sftp_home_dir(state: State<'_, AppState>, connection_id: String) -> AppResult<String> {
    let sftp = session(&state, &connection_id).await?;
    sftp.canonicalize(".").await.map_err(sftp_err)
}

/// Channel deep-search batches are emitted on.
pub const SFTP_SEARCH_CHANNEL: &str = "portbay://sftp-search";

/// Walk guards: stop a runaway search before it melts a slow link. The caller
/// surfaces `truncated: true` so the user knows the result set is partial.
const SEARCH_MAX_SCANNED: u64 = 200_000;
const SEARCH_DEFAULT_MAX_RESULTS: u32 = 500;
const SEARCH_DEFAULT_MAX_DEPTH: u32 = 24;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpSearchInput {
    pub connection_id: String,
    /// Caller-assigned id, echoed in every batch event and used to cancel.
    pub id: String,
    /// Directory to search under (recursively).
    pub root: String,
    /// Plain text = case-insensitive substring; `*` / `?` = glob over the
    /// whole name (e.g. `*.zip`).
    pub query: String,
    pub max_results: Option<u32>,
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpSearchBatch {
    pub id: String,
    /// New hits since the previous batch (the frontend accumulates).
    pub hits: Vec<SftpEntry>,
    /// Total directory entries examined so far.
    pub scanned: u64,
    pub done: bool,
    /// True when the walk stopped at a result/scan/depth cap.
    pub truncated: bool,
}

/// Case-insensitive glob match supporting `*` (any run) and `?` (any one),
/// via the classic two-pointer scan with `*` backtracking. Both inputs must
/// already be lowercase. Shared with the local-filesystem search.
pub(crate) fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Recursive "find files" over the cached SFTP session — the deep search every
/// desktop SFTP client ships. Walks breadth-first from `root`, matching names
/// against the query, and streams hits in batches on [`SFTP_SEARCH_CHANNEL`]
/// (so results appear as they're found on slow trees). Unreadable directories
/// are skipped, symlinked directories aren't descended into (cycle guard), and
/// the walk is bounded by result/scan/depth caps. Cancellable via
/// [`sftp_search_cancel`]; a cancel still emits the final `done` batch.
#[tauri::command]
pub async fn sftp_search(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SftpSearchInput,
) -> AppResult<()> {
    let sftp = session(&state, &input.connection_id).await?;
    let q = input.query.trim().to_lowercase();
    let max_results = input
        .max_results
        .unwrap_or(SEARCH_DEFAULT_MAX_RESULTS)
        .min(5_000) as usize;
    let max_depth = input.max_depth.unwrap_or(SEARCH_DEFAULT_MAX_DEPTH);
    // No wildcard → substring search (wrap as *q* for the matcher).
    let pattern = if q.contains('*') || q.contains('?') {
        q.clone()
    } else {
        format!("*{q}*")
    };

    let cancel = register_cancel(&input.id);
    let mut queue: std::collections::VecDeque<(String, u32)> = std::collections::VecDeque::new();
    queue.push_back((input.root.clone(), 0));
    let mut pending: Vec<SftpEntry> = Vec::new();
    let mut scanned: u64 = 0;
    let mut found = 0usize;
    let mut truncated = false;
    let mut last_emit = std::time::Instant::now();

    if !q.is_empty() {
        'walk: while let Some((dir, depth)) = queue.pop_front() {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            // Unreadable (permissions) directories are skipped, not fatal.
            let Ok(read) = sftp.read_dir(dir).await else {
                continue;
            };
            for e in read {
                if cancel.load(Ordering::SeqCst) {
                    break 'walk;
                }
                scanned += 1;
                let entry = entry_from(e.file_name(), e.path(), &e.metadata());
                if glob_match(&pattern, &entry.name.to_lowercase()) {
                    pending.push(entry.clone());
                    found += 1;
                    if found >= max_results {
                        truncated = true;
                        break 'walk;
                    }
                }
                if entry.is_dir && !entry.is_symlink && depth < max_depth {
                    queue.push_back((entry.path, depth + 1));
                }
                if scanned >= SEARCH_MAX_SCANNED {
                    truncated = true;
                    break 'walk;
                }
                // Stream what we have every ~250 ms so the UI fills in live.
                if last_emit.elapsed().as_millis() >= 250 {
                    let _ = app.emit(
                        SFTP_SEARCH_CHANNEL,
                        &SftpSearchBatch {
                            id: input.id.clone(),
                            hits: std::mem::take(&mut pending),
                            scanned,
                            done: false,
                            truncated: false,
                        },
                    );
                    last_emit = std::time::Instant::now();
                }
            }
        }
    }

    deregister_cancel(&input.id);
    let _ = app.emit(
        SFTP_SEARCH_CHANNEL,
        &SftpSearchBatch {
            id: input.id,
            hits: pending,
            scanned,
            done: true,
            truncated,
        },
    );
    Ok(())
}

/// Stop an in-flight `sftp_search`. The walk stops at the next entry and still
/// emits its final `done` batch with whatever was found.
#[tauri::command]
pub async fn sftp_search_cancel(id: String) -> AppResult<()> {
    if let Ok(reg) = cancel_registry().lock() {
        if let Some(flag) = reg.get(&id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// List one remote directory. Entries are sorted dirs-first then by name.
#[tauri::command]
pub async fn sftp_list_dir(
    state: State<'_, AppState>,
    input: SftpPathInput,
) -> AppResult<Vec<SftpEntry>> {
    let sftp = session(&state, &input.connection_id).await?;
    let read_dir = sftp.read_dir(input.path.clone()).await.map_err(sftp_err)?;
    let mut out: Vec<SftpEntry> = read_dir
        .map(|e| entry_from(e.file_name(), e.path(), &e.metadata()))
        .collect();
    out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(out)
}

/// Stat a single remote path.
#[tauri::command]
pub async fn sftp_stat(state: State<'_, AppState>, input: SftpPathInput) -> AppResult<SftpEntry> {
    let sftp = session(&state, &input.connection_id).await?;
    let attrs = sftp.metadata(input.path.clone()).await.map_err(sftp_err)?;
    let name = input
        .path
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(&input.path)
        .to_string();
    Ok(entry_from(name, input.path, &attrs))
}

#[tauri::command]
pub async fn sftp_mkdir(state: State<'_, AppState>, input: SftpPathInput) -> AppResult<()> {
    let sftp = session(&state, &input.connection_id).await?;
    sftp.create_dir(input.path).await.map_err(sftp_err)
}

#[tauri::command]
pub async fn sftp_rename(state: State<'_, AppState>, input: SftpRenameInput) -> AppResult<()> {
    let sftp = session(&state, &input.connection_id).await?;
    sftp.rename(input.from, input.to).await.map_err(sftp_err)
}

#[tauri::command]
pub async fn sftp_remove_file(state: State<'_, AppState>, input: SftpPathInput) -> AppResult<()> {
    let sftp = session(&state, &input.connection_id).await?;
    sftp.remove_file(input.path).await.map_err(sftp_err)
}

#[tauri::command]
pub async fn sftp_remove_dir(state: State<'_, AppState>, input: SftpPathInput) -> AppResult<()> {
    let sftp = session(&state, &input.connection_id).await?;
    sftp.remove_dir(input.path).await.map_err(sftp_err)
}

#[tauri::command]
pub async fn sftp_chmod(state: State<'_, AppState>, input: SftpChmodInput) -> AppResult<()> {
    let sftp = session(&state, &input.connection_id).await?;
    let attrs = FileAttributes {
        size: None,
        uid: None,
        user: None,
        gid: None,
        group: None,
        permissions: Some(input.mode),
        atime: None,
        mtime: None,
    };
    sftp.set_metadata(input.path, attrs).await.map_err(sftp_err)
}

/// Read a remote text file (for edit-and-push). Errors on non-UTF-8 content.
#[tauri::command]
pub async fn sftp_read_text(state: State<'_, AppState>, input: SftpPathInput) -> AppResult<String> {
    let sftp = session(&state, &input.connection_id).await?;
    let bytes = sftp.read(input.path).await.map_err(sftp_err)?;
    if bytes.len() as u64 > MAX_TRANSFER_BYTES {
        return Err(AppError::BadInput(
            "file is too large to open in the editor".into(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| AppError::BadInput("this file isn't UTF-8 text — download it instead".into()))
}

/// Preview read ceiling (10 MiB). Previews are buffered + base64'd, so this is a
/// far tighter bound than [`MAX_TRANSFER_BYTES`] — bigger files say "download".
const MAX_PREVIEW_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpPreview {
    /// `"image"`, `"text"`, or `"binary"`.
    pub kind: String,
    /// MIME type for an image (e.g. `image/png`), when recognised.
    pub mime: Option<String>,
    /// Base64-encoded bytes for an image preview.
    pub base64: Option<String>,
    /// Decoded text for a text preview.
    pub text: Option<String>,
    pub size: u64,
}

/// Map a filename extension to an image MIME type, or `None` if not an image.
fn image_mime(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

/// Read a remote file for in-app preview: images come back base64-encoded with a
/// MIME type, UTF-8 files come back as text, and anything else is reported as
/// binary (size only). Files above [`MAX_PREVIEW_BYTES`] are refused so a stray
/// click on a huge file can't buffer it — the UI offers Download instead.
#[tauri::command]
pub async fn sftp_read_preview(
    state: State<'_, AppState>,
    input: SftpPathInput,
) -> AppResult<SftpPreview> {
    let sftp = session(&state, &input.connection_id).await?;
    let attrs = sftp.metadata(input.path.clone()).await.map_err(sftp_err)?;
    let size = attrs.size.unwrap_or(0);
    if size > MAX_PREVIEW_BYTES {
        return Err(AppError::BadInput(format!(
            "file is larger than the {} MiB preview limit — download it instead",
            MAX_PREVIEW_BYTES / (1024 * 1024)
        )));
    }

    let name = input
        .path
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(&input.path)
        .to_string();
    let bytes = sftp.read(input.path).await.map_err(sftp_err)?;

    if let Some(mime) = image_mime(&name) {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(SftpPreview {
            kind: "image".into(),
            mime: Some(mime.to_string()),
            base64: Some(b64),
            text: None,
            size,
        });
    }

    match String::from_utf8(bytes) {
        Ok(text) => Ok(SftpPreview {
            kind: "text".into(),
            mime: None,
            base64: None,
            text: Some(text),
            size,
        }),
        Err(_) => Ok(SftpPreview {
            kind: "binary".into(),
            mime: None,
            base64: None,
            text: None,
            size,
        }),
    }
}

/// Write a remote text file (edit-and-push / create).
#[tauri::command]
pub async fn sftp_write_text(state: State<'_, AppState>, input: SftpWriteInput) -> AppResult<()> {
    let sftp = session(&state, &input.connection_id).await?;
    write_remote(&sftp, input.path, input.contents.as_bytes()).await
}

/// Upload a local file to the remote path.
///
/// `input.local_path` must have been chosen through a host dialog
/// (`sftp_pick_upload_files`) or recorded via a real OS drag-drop —
/// `ensure_local_path_approved` enforces this before any I/O.
#[tauri::command]
pub async fn sftp_upload(state: State<'_, AppState>, input: SftpTransferInput) -> AppResult<u64> {
    // Validate before touching the filesystem. Returns the canonical path to
    // use for I/O so we don't re-use the renderer-supplied raw string.
    let local = ensure_local_path_approved(&state, &input.local_path)?;

    let meta = std::fs::metadata(&local)?;
    if meta.len() > MAX_TRANSFER_BYTES {
        return Err(AppError::BadInput(format!(
            "`{}` is larger than the {} MiB whole-file transfer limit",
            local.display(),
            MAX_TRANSFER_BYTES / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(&local)?;
    // Re-check after read: the file could have grown between stat and read
    // (e.g. an append-only log). Bail rather than silently upload more than
    // the advertised ceiling.
    if bytes.len() as u64 > MAX_TRANSFER_BYTES {
        return Err(AppError::BadInput(format!(
            "`{}` exceeds the {} MiB whole-file transfer limit",
            local.display(),
            MAX_TRANSFER_BYTES / (1024 * 1024)
        )));
    }
    let sftp = session(&state, &input.connection_id).await?;
    write_remote(&sftp, input.remote_path, &bytes).await?;
    Ok(bytes.len() as u64)
}

/// Download a remote file to the local path.
///
/// `input.local_path` must have been chosen through a host dialog
/// (`sftp_pick_save_path` or `sftp_pick_download_dir`) —
/// `ensure_local_path_approved` enforces this before writing.
///
/// Uses a chunked read loop instead of `sftp.read()` (whole-file) to close
/// a TOCTOU window: a growing remote file could exceed `MAX_TRANSFER_BYTES`
/// between the metadata check and the bulk read, potentially causing OOM. The
/// loop bails as soon as the running byte count exceeds the cap.
#[tauri::command]
pub async fn sftp_download(state: State<'_, AppState>, input: SftpTransferInput) -> AppResult<u64> {
    // Validate destination before any remote I/O.
    let local = ensure_local_path_approved(&state, &input.local_path)?;

    let sftp = session(&state, &input.connection_id).await?;
    let mut remote = sftp
        .open(input.remote_path.clone())
        .await
        .map_err(sftp_err)?;

    // Chunked read: accumulate into a Vec, bail once we exceed the cap.
    // This closes the TOCTOU where sftp.read() could buffer a file that
    // grew past MAX_TRANSFER_BYTES between the metadata stat and the read.
    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut data: Vec<u8> = Vec::new();
    loop {
        let n = remote.read(&mut buf).await.map_err(sftp_err)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() as u64 > MAX_TRANSFER_BYTES {
            return Err(AppError::BadInput(format!(
                "remote file exceeds the {} MiB whole-file transfer limit",
                MAX_TRANSFER_BYTES / (1024 * 1024)
            )));
        }
    }
    std::fs::write(&local, &data)?;
    Ok(data.len() as u64)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpTransferJob {
    pub connection_id: String,
    /// Caller-assigned id, echoed in every progress event for this transfer.
    pub id: String,
    /// `"upload"` (local → remote) or `"download"` (remote → local).
    pub direction: String,
    pub local_path: String,
    pub remote_path: String,
    /// Resume intent: 0 / absent = fresh transfer (truncates the staging part
    /// file); >0 = resume the prior partial. The value itself is advisory —
    /// the part file's actual size is the authoritative resume offset.
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpProgress {
    pub id: String,
    pub transferred: u64,
    pub total: u64,
    pub done: bool,
    /// True when the transfer stopped early on a cancel request — `transferred`
    /// is the resume offset, and it's not an error.
    pub paused: bool,
    pub error: Option<String>,
}

/// Cancel flags for in-flight `sftp_transfer` calls, keyed by transfer id. A
/// transfer registers a flag on start and polls it each chunk; `sftp_transfer_cancel`
/// flips it so the loop stops cleanly, leaving the partial bytes in place to resume
/// from. A process-global registry (rather than `AppState`) keeps this self-contained.
fn cancel_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REG: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel(id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut reg) = cancel_registry().lock() {
        reg.insert(id.to_string(), flag.clone());
    }
    flag
}

fn deregister_cancel(id: &str) {
    if let Ok(mut reg) = cancel_registry().lock() {
        reg.remove(id);
    }
}

/// Request cancellation of an in-flight transfer. The transfer stops at the next
/// chunk boundary and emits a `paused` progress event with its current offset;
/// the partial file is left in place so the queue can resume it.
#[tauri::command]
pub async fn sftp_transfer_cancel(id: String) -> AppResult<()> {
    if let Ok(reg) = cancel_registry().lock() {
        if let Some(flag) = reg.get(&id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// Emit a progress event, throttled by the caller (it only calls on meaningful
/// deltas / completion) so the channel isn't flooded on large files.
fn emit_progress(
    app: &AppHandle,
    id: &str,
    transferred: u64,
    total: u64,
    done: bool,
    paused: bool,
    error: Option<String>,
) {
    let _ = app.emit(
        SFTP_PROGRESS_CHANNEL,
        SftpProgress {
            id: id.to_string(),
            transferred,
            total,
            done,
            paused,
            error,
        },
    );
}

/// Threshold for the next progress emit: ~1% of the file, floored at 1 MiB so a
/// tiny file emits start+done and a huge one emits ~100 updates, never thousands.
fn progress_step(total: u64) -> u64 {
    (total / 100).max(1024 * 1024)
}

/// Staging suffix for in-flight queue transfers. Data streams into
/// `<destination>.portbaypart` and is renamed over the destination only after
/// the byte count verifies, so an interrupted upload never replaces a good
/// file on a live server and readers never see a torn file at the real path.
/// The part file is also what resume continues into; its size — not the
/// caller-remembered offset — is the authoritative resume point.
const PART_SUFFIX: &str = ".portbaypart";

/// Stream a file in either direction, chunked, emitting throttled progress on
/// [`SFTP_PROGRESS_CHANNEL`]. Unlike [`sftp_upload`]/[`sftp_download`] this never
/// buffers the whole file in memory, so there's no size ceiling — it's the path
/// the transfer queue uses. Concurrent `sftp_transfer` calls multiplex over the
/// one cached SFTP session, giving real parallel channels over a single login.
///
/// Both directions stage into a [`PART_SUFFIX`] sibling and atomically rename
/// over the destination after the transferred byte count verifies against the
/// destination side's own metadata — interruption, cancellation, or a
/// mid-transfer change of the source can never leave a silently-torn file at
/// the destination path.
#[tauri::command]
pub async fn sftp_transfer(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SftpTransferJob,
) -> AppResult<u64> {
    let cancel = register_cancel(&input.id);
    // Bytes confirmed moved so far, reported on the error path so a dropped
    // connection resumes from where it stopped instead of starting over —
    // checkpoints and datasets are tens of GB.
    let mut moved: u64 = input.offset;
    let result = match input.direction.as_str() {
        "upload" => transfer_upload(&app, &state, &input, &cancel, &mut moved).await,
        "download" => transfer_download(&app, &state, &input, &cancel, &mut moved).await,
        other => Err(AppError::BadInput(format!(
            "unknown transfer direction `{other}`"
        ))),
    };
    deregister_cancel(&input.id);
    if let Err(e) = &result {
        emit_progress(&app, &input.id, moved, 0, true, false, Some(e.to_string()));
    }
    result
}

async fn transfer_upload(
    app: &AppHandle,
    state: &State<'_, AppState>,
    input: &SftpTransferJob,
    cancel: &AtomicBool,
    moved: &mut u64,
) -> AppResult<u64> {
    // Validate before any I/O. The approved set is session-long, so a resumed
    // transfer (offset > 0, same path) continues working without re-prompting.
    let local_path = ensure_local_path_approved(state, &input.local_path)?;
    let mut local = tokio::fs::File::open(&local_path).await?;
    let total = local.metadata().await.map(|m| m.len()).unwrap_or(0);
    let sftp = session(state, &input.connection_id).await?;

    // Stream into the staging sibling; the destination is only touched by the
    // verified rename at the end.
    let part = format!("{}{PART_SUFFIX}", input.remote_path);

    // offset 0 → create+truncate a fresh part file. offset > 0 is only the
    // *intent* to resume — the authoritative offset is the part file's actual
    // size, so a stale offset can never seek past EOF and leave a zero-filled
    // hole (silent corruption) or rewrite bytes already acknowledged.
    let start = if input.offset > 0 {
        sftp.metadata(part.clone())
            .await
            .ok()
            .and_then(|a| a.size)
            .unwrap_or(0)
            .min(total)
    } else {
        0
    };
    let mut remote = if start > 0 {
        let mut f = sftp
            .open_with_flags(part.clone(), OpenFlags::WRITE | OpenFlags::CREATE)
            .await
            .map_err(sftp_err)?;
        f.seek(SeekFrom::Start(start)).await.map_err(sftp_err)?;
        local.seek(SeekFrom::Start(start)).await?;
        f
    } else {
        sftp.create(part.clone()).await.map_err(sftp_err)?
    };

    let mut transferred: u64 = start;
    *moved = transferred;
    emit_progress(app, &input.id, transferred, total, false, false, None);
    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut last_emit: u64 = transferred;
    let step = progress_step(total);
    loop {
        if cancel.load(Ordering::SeqCst) {
            remote.shutdown().await.map_err(sftp_err)?;
            emit_progress(app, &input.id, transferred, total, false, true, None);
            return Ok(transferred);
        }
        let n = local.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote.write_all(&buf[..n]).await.map_err(sftp_err)?;
        transferred += n as u64;
        *moved = transferred;
        if transferred - last_emit >= step {
            last_emit = transferred;
            emit_progress(app, &input.id, transferred, total, false, false, None);
        }
    }
    remote.shutdown().await.map_err(sftp_err)?;

    // End-to-end verification: the server's own size for the staged file must
    // account for every byte sent before it may replace the destination.
    let staged = sftp
        .metadata(part.clone())
        .await
        .map_err(sftp_err)?
        .size
        .unwrap_or(0);
    if staged != transferred {
        return Err(AppError::Internal(format!(
            "upload verification failed: the host reports {staged} of {transferred} bytes — \
             the destination was left untouched; resume the transfer to finish it"
        )));
    }

    // Replace the destination. The part file was created fresh, so carry the
    // destination's permission bits over first (a 0755 script must not come
    // back 0644), then remove + rename — SFTP v3 RENAME doesn't overwrite.
    // Replace-at-the-end means an existing file stays intact until this point.
    if let Ok(prior) = sftp.metadata(input.remote_path.clone()).await {
        if prior.permissions.is_some() {
            let attrs = FileAttributes {
                size: None,
                uid: None,
                user: None,
                gid: None,
                group: None,
                permissions: prior.permissions,
                atime: None,
                mtime: None,
            };
            let _ = sftp.set_metadata(part.clone(), attrs).await;
        }
        let _ = sftp.remove_file(input.remote_path.clone()).await;
    }
    sftp.rename(part, input.remote_path.clone())
        .await
        .map_err(sftp_err)?;

    emit_progress(app, &input.id, transferred, total, true, false, None);
    Ok(transferred)
}

async fn transfer_download(
    app: &AppHandle,
    state: &State<'_, AppState>,
    input: &SftpTransferJob,
    cancel: &AtomicBool,
    moved: &mut u64,
) -> AppResult<u64> {
    // Validate before any I/O. The approved set is session-long, so a resumed
    // transfer (offset > 0, same path) continues working without re-prompting.
    // The staging sibling lives in the same (approved) directory and is backend-
    // chosen, never renderer-supplied.
    let local_path = ensure_local_path_approved(state, &input.local_path)?;
    let part_path = {
        let mut name = local_path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(PART_SUFFIX);
        local_path.with_file_name(name)
    };

    let sftp = session(state, &input.connection_id).await?;
    let total = sftp
        .metadata(input.remote_path.clone())
        .await
        .map(|a| a.size.unwrap_or(0))
        .unwrap_or(0);
    let mut remote = sftp
        .open(input.remote_path.clone())
        .await
        .map_err(sftp_err)?;

    // offset 0 → create+truncate a fresh part file. offset > 0 is only the
    // *intent* to resume — the part file's actual size is the authoritative
    // offset (clamped to the remote size in case the file shrank), so a stale
    // offset can never seek past what was really written.
    let start = if input.offset > 0 {
        tokio::fs::metadata(&part_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
            .min(total)
    } else {
        0
    };
    let mut local = if start > 0 {
        let mut f = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&part_path)
            .await?;
        f.seek(SeekFrom::Start(start)).await?;
        remote
            .seek(SeekFrom::Start(start))
            .await
            .map_err(sftp_err)?;
        f
    } else {
        tokio::fs::File::create(&part_path).await?
    };

    let mut transferred: u64 = start;
    *moved = transferred;
    emit_progress(app, &input.id, transferred, total, false, false, None);
    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut last_emit: u64 = transferred;
    let step = progress_step(total);
    loop {
        if cancel.load(Ordering::SeqCst) {
            local.flush().await?;
            emit_progress(app, &input.id, transferred, total, false, true, None);
            return Ok(transferred);
        }
        let n = remote.read(&mut buf).await.map_err(sftp_err)?;
        if n == 0 {
            break;
        }
        local.write_all(&buf[..n]).await?;
        transferred += n as u64;
        *moved = transferred;
        if transferred - last_emit >= step {
            last_emit = transferred;
            emit_progress(app, &input.id, transferred, total, false, false, None);
        }
    }
    local.flush().await?;

    // Torn-read check: if the remote file changed size while it streamed (a
    // checkpoint still being written, a rotated log), the local copy is a mix
    // of two versions — surface that instead of delivering it silently.
    if let Ok(end_size) = sftp.metadata(input.remote_path.clone()).await {
        let end_size = end_size.size.unwrap_or(transferred);
        if end_size != transferred {
            return Err(AppError::Internal(format!(
                "download verification failed: the remote file is {end_size} bytes but \
                 {transferred} were read — it changed while downloading; retry when it's stable"
            )));
        }
    }

    // Atomically move the verified part file onto the destination — a reader
    // (or the user) never sees a half-written file at the real path.
    tokio::fs::rename(&part_path, &local_path).await?;
    emit_progress(app, &input.id, transferred, total, true, false, None);
    Ok(transferred)
}

/// Drop the cached SFTP session for a connection.
#[tauri::command]
pub async fn sftp_disconnect(state: State<'_, AppState>, connection_id: String) -> AppResult<()> {
    state.sftp.lock().await.disconnect(&connection_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // ── image_mime ───────────────────────────────────────────────────────────

    #[test]
    fn image_mime_known_extensions() {
        assert_eq!(super::image_mime("photo.png"), Some("image/png"));
        assert_eq!(super::image_mime("photo.PNG"), Some("image/png")); // case-insensitive
        assert_eq!(super::image_mime("photo.jpg"), Some("image/jpeg"));
        assert_eq!(super::image_mime("photo.jpeg"), Some("image/jpeg"));
        assert_eq!(super::image_mime("anim.gif"), Some("image/gif"));
        assert_eq!(super::image_mime("file.webp"), Some("image/webp"));
        assert_eq!(super::image_mime("file.bmp"), Some("image/bmp"));
        assert_eq!(super::image_mime("favicon.ico"), Some("image/x-icon"));
        assert_eq!(super::image_mime("photo.avif"), Some("image/avif"));
        assert_eq!(super::image_mime("logo.svg"), Some("image/svg+xml"));
    }

    #[test]
    fn image_mime_unknown_extensions_return_none() {
        assert!(super::image_mime("script.js").is_none());
        assert!(super::image_mime("archive.tar.gz").is_none());
        assert!(super::image_mime("noextension").is_none());
        assert!(super::image_mime(".gitignore").is_none());
        assert!(super::image_mime("file.txt").is_none());
        assert!(super::image_mime("data.json").is_none());
    }

    // ── progress_step ────────────────────────────────────────────────────────

    #[test]
    fn progress_step_floors_at_1_mib_for_tiny_files() {
        // A 1-byte file → 1% = 0 → floor kicks in, step = 1 MiB.
        assert_eq!(super::progress_step(0), 1024 * 1024);
        assert_eq!(super::progress_step(99), 1024 * 1024);
        assert_eq!(super::progress_step(1024 * 1024), 1024 * 1024);
    }

    #[test]
    fn progress_step_is_one_percent_for_large_files() {
        // A 200 MiB file → 1% = 2 MiB > floor.
        let two_hundred_mib = 200u64 * 1024 * 1024;
        let step = super::progress_step(two_hundred_mib);
        assert_eq!(step, two_hundred_mib / 100);
        assert!(step > 1024 * 1024, "step should exceed the 1 MiB floor");
    }

    #[test]
    fn progress_step_for_exactly_100_mib() {
        // 100 MiB → 1% = 1 MiB exactly — sits on the floor boundary.
        let one_hundred_mib = 100u64 * 1024 * 1024;
        assert_eq!(super::progress_step(one_hundred_mib), 1024 * 1024);
    }

    // ── SftpTransferJob serde ─────────────────────────────────────────────────

    #[test]
    fn sftp_transfer_job_deserialises_camel_case() {
        let json = r#"{
            "connectionId": "my-server",
            "id": "job-1",
            "direction": "upload",
            "localPath": "/home/user/file.txt",
            "remotePath": "/srv/file.txt"
        }"#;
        let job: super::SftpTransferJob = serde_json::from_str(json).unwrap();
        assert_eq!(job.connection_id, "my-server");
        assert_eq!(job.id, "job-1");
        assert_eq!(job.direction, "upload");
        assert_eq!(job.local_path, "/home/user/file.txt");
        assert_eq!(job.remote_path, "/srv/file.txt");
        // offset omitted → defaults to 0
        assert_eq!(job.offset, 0);
    }

    #[test]
    fn sftp_transfer_job_offset_defaults_to_zero() {
        let json = r#"{
            "connectionId": "c",
            "id": "j",
            "direction": "download",
            "localPath": "/tmp/out",
            "remotePath": "/data/file"
        }"#;
        let job: super::SftpTransferJob = serde_json::from_str(json).unwrap();
        assert_eq!(job.offset, 0);
    }

    #[test]
    fn sftp_transfer_job_explicit_resume_offset() {
        let json = r#"{
            "connectionId": "c",
            "id": "j",
            "direction": "download",
            "localPath": "/tmp/out",
            "remotePath": "/data/file",
            "offset": 131072
        }"#;
        let job: super::SftpTransferJob = serde_json::from_str(json).unwrap();
        assert_eq!(job.offset, 131_072);
    }

    // ── SftpProgress serde ───────────────────────────────────────────────────

    #[test]
    fn sftp_progress_serialises_all_fields() {
        let p = super::SftpProgress {
            id: "xfer-7".into(),
            transferred: 512,
            total: 1024,
            done: false,
            paused: false,
            error: None,
        };
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["id"], "xfer-7");
        assert_eq!(v["transferred"], 512);
        assert_eq!(v["total"], 1024);
        assert_eq!(v["done"], false);
        assert_eq!(v["paused"], false);
        assert!(v["error"].is_null());
    }

    #[test]
    fn sftp_progress_serialised_keys_are_camel_case() {
        let p = super::SftpProgress {
            id: "x".into(),
            transferred: 0,
            total: 0,
            done: true,
            paused: false,
            error: Some("oops".into()),
        };
        let json = serde_json::to_string(&p).unwrap();
        // Snake-case forms must not appear.
        assert!(!json.contains("\"done_flag\""));
        // camelCase field names that matter to the frontend.
        assert!(json.contains("\"transferred\""));
        assert!(json.contains("\"total\""));
        assert!(json.contains("\"done\""));
        assert!(json.contains("\"paused\""));
        assert!(json.contains("\"error\""));
    }

    // ── sftp_err ─────────────────────────────────────────────────────────────

    #[test]
    fn sftp_err_produces_internal_with_prefix() {
        let err = super::sftp_err("connection reset");
        let msg = err.to_string();
        assert!(msg.contains("SFTP error"), "got: {msg}");
        assert!(msg.contains("connection reset"), "got: {msg}");
    }

    // ── MAX_TRANSFER_BYTES constant ───────────────────────────────────────────

    #[test]
    fn max_transfer_bytes_is_one_gib() {
        assert_eq!(super::MAX_TRANSFER_BYTES, 1024 * 1024 * 1024);
    }

    // Minimal stub of the approval set so we can test `ensure_local_path_approved`
    // without constructing a full AppState (which requires Tauri setup).
    struct ApprovalSet {
        paths: Mutex<HashSet<std::path::PathBuf>>,
    }

    impl ApprovalSet {
        fn new() -> Self {
            Self {
                paths: Mutex::new(HashSet::new()),
            }
        }
        fn insert(&self, p: std::path::PathBuf) {
            self.paths.lock().unwrap().insert(p);
        }
    }

    /// Thin reimplementation of the helper's logic against our stub, so we
    /// can unit-test the approval rules without a Tauri AppState.
    fn check_approved(set: &ApprovalSet, raw: &str) -> Result<std::path::PathBuf, String> {
        let raw_path = std::path::Path::new(raw);
        let canonical = if raw_path.exists() {
            std::fs::canonicalize(raw_path).map_err(|e| e.to_string())?
        } else {
            let parent = raw_path
                .parent()
                .ok_or_else(|| format!("path `{raw}` has no parent"))?;
            let canon_parent = std::fs::canonicalize(parent).map_err(|_| {
                format!(
                    "local path `{raw}` was not chosen through a file dialog — \
                         pick the destination again"
                )
            })?;
            let file_name = raw_path
                .file_name()
                .ok_or_else(|| format!("path `{raw}` has no file name"))?;
            canon_parent.join(file_name)
        };

        let approved = set.paths.lock().unwrap();
        if approved.contains(&canonical) {
            return Ok(canonical);
        }
        for entry in approved.iter() {
            if canonical.starts_with(entry) && canonical != *entry {
                return Ok(canonical);
            }
        }
        Err(format!(
            "local path `{raw}` was not chosen through a file dialog — \
             pick the destination again"
        ))
    }

    #[test]
    fn exact_file_approval() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("upload.bin");
        std::fs::write(&file, b"data").unwrap();
        let canon = std::fs::canonicalize(&file).unwrap();

        let set = ApprovalSet::new();
        set.insert(canon.clone());

        let result = check_approved(&set, file.to_str().unwrap());
        assert!(result.is_ok(), "exact file must be approved: {result:?}");
        assert_eq!(result.unwrap(), canon);
    }

    #[test]
    fn descendant_of_approved_dir() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let file = sub.join("data.txt");
        std::fs::write(&file, b"content").unwrap();
        let canon_dir = std::fs::canonicalize(&sub).unwrap();

        let set = ApprovalSet::new();
        // Only the directory is approved, not the file itself.
        set.insert(canon_dir);

        let result = check_approved(&set, file.to_str().unwrap());
        assert!(result.is_ok(), "descendant must be approved: {result:?}");
    }

    #[test]
    fn unapproved_path_rejected() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("secret.key");
        std::fs::write(&file, b"secret").unwrap();

        let set = ApprovalSet::new(); // nothing approved

        let result = check_approved(&set, file.to_str().unwrap());
        assert!(result.is_err(), "unapproved path must be rejected");
        let msg = result.unwrap_err();
        assert!(msg.contains("not chosen through a file dialog"), "{msg}");
    }

    #[test]
    fn traversal_outside_approved_dir_rejected() {
        let outer = TempDir::new().unwrap();
        let inner = outer.path().join("approved");
        std::fs::create_dir(&inner).unwrap();
        // A file that exists in the outer directory (outside the approved subtree).
        let secret = outer.path().join("shadow.txt");
        std::fs::write(&secret, b"shadow").unwrap();
        let canon_inner = std::fs::canonicalize(&inner).unwrap();

        let set = ApprovalSet::new();
        set.insert(canon_inner);

        // Try to reference `shadow.txt` via a path that isn't under `inner`.
        let result = check_approved(&set, secret.to_str().unwrap());
        assert!(
            result.is_err(),
            "path outside approved dir must be rejected: {result:?}"
        );
    }

    #[test]
    fn nonexistent_file_under_approved_dir_uses_parent_canonicalization() {
        let dir = TempDir::new().unwrap();
        let canon_dir = std::fs::canonicalize(dir.path()).unwrap();

        let set = ApprovalSet::new();
        set.insert(canon_dir.clone());

        // File doesn't exist yet (download destination).
        let dest = dir.path().join("new_download.bin");
        assert!(!dest.exists());

        let result = check_approved(&set, dest.to_str().unwrap());
        assert!(
            result.is_ok(),
            "nonexistent file under approved dir: {result:?}"
        );
        assert_eq!(result.unwrap(), canon_dir.join("new_download.bin"));
    }

    #[test]
    fn nonexistent_parent_rejected() {
        // A path whose parent directory doesn't exist at all — should be rejected.
        let raw = "/tmp/portbay-test-nonexistent-dir-abc123/file.txt";
        // Ensure it really doesn't exist.
        let parent = std::path::Path::new(raw).parent().unwrap();
        assert!(!parent.exists(), "test precondition: parent must not exist");

        let set = ApprovalSet::new();
        let result = check_approved(&set, raw);
        assert!(result.is_err(), "nonexistent parent must be rejected");
    }
}
