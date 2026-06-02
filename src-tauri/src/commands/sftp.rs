//! SFTP file-manager commands over a saved [`SshConnection`].
//!
//! Each command resolves the connection from the registry, borrows (or opens) a
//! cached SFTP session from [`SftpManager`], and runs one operation. Sessions
//! are cached per connection so browsing doesn't re-handshake on every click.
//!
//! Transfers are whole-file (read into memory, then write) — simple and robust
//! for source trees; a streaming path for very large files is a follow-up, and
//! [`MAX_TRANSFER_BYTES`] guards against accidentally loading a huge file.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::projects::load_registry;
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::SshConnectionId;
use crate::state::AppState;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        Some(crate::ssh::EventInteractor::new(app)),
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
#[tauri::command]
pub async fn sftp_upload(state: State<'_, AppState>, input: SftpTransferInput) -> AppResult<u64> {
    let meta = std::fs::metadata(&input.local_path)?;
    if meta.len() > MAX_TRANSFER_BYTES {
        return Err(AppError::BadInput(format!(
            "`{}` is larger than the {} MiB whole-file transfer limit",
            input.local_path,
            MAX_TRANSFER_BYTES / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(&input.local_path)?;
    let sftp = session(&state, &input.connection_id).await?;
    write_remote(&sftp, input.remote_path, &bytes).await?;
    Ok(bytes.len() as u64)
}

/// Download a remote file to the local path.
#[tauri::command]
pub async fn sftp_download(state: State<'_, AppState>, input: SftpTransferInput) -> AppResult<u64> {
    let sftp = session(&state, &input.connection_id).await?;
    let attrs = sftp
        .metadata(input.remote_path.clone())
        .await
        .map_err(sftp_err)?;
    if attrs.size.unwrap_or(0) > MAX_TRANSFER_BYTES {
        return Err(AppError::BadInput(format!(
            "remote file is larger than the {} MiB whole-file transfer limit",
            MAX_TRANSFER_BYTES / (1024 * 1024)
        )));
    }
    let bytes = sftp.read(input.remote_path).await.map_err(sftp_err)?;
    std::fs::write(&input.local_path, &bytes)?;
    Ok(bytes.len() as u64)
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpProgress {
    pub id: String,
    pub transferred: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

/// Emit a progress event, throttled by the caller (it only calls on meaningful
/// deltas / completion) so the channel isn't flooded on large files.
fn emit_progress(app: &AppHandle, id: &str, transferred: u64, total: u64, done: bool, error: Option<String>) {
    let _ = app.emit(
        SFTP_PROGRESS_CHANNEL,
        SftpProgress { id: id.to_string(), transferred, total, done, error },
    );
}

/// Threshold for the next progress emit: ~1% of the file, floored at 1 MiB so a
/// tiny file emits start+done and a huge one emits ~100 updates, never thousands.
fn progress_step(total: u64) -> u64 {
    (total / 100).max(1024 * 1024)
}

/// Stream a file in either direction, chunked, emitting throttled progress on
/// [`SFTP_PROGRESS_CHANNEL`]. Unlike [`sftp_upload`]/[`sftp_download`] this never
/// buffers the whole file in memory, so there's no size ceiling — it's the path
/// the transfer queue uses. Concurrent `sftp_transfer` calls multiplex over the
/// one cached SFTP session, giving real parallel channels over a single login.
#[tauri::command]
pub async fn sftp_transfer(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SftpTransferJob,
) -> AppResult<u64> {
    let result = match input.direction.as_str() {
        "upload" => transfer_upload(&app, &state, &input).await,
        "download" => transfer_download(&app, &state, &input).await,
        other => Err(AppError::BadInput(format!("unknown transfer direction `{other}`"))),
    };
    if let Err(e) = &result {
        emit_progress(&app, &input.id, 0, 0, true, Some(e.to_string()));
    }
    result
}

async fn transfer_upload(
    app: &AppHandle,
    state: &State<'_, AppState>,
    input: &SftpTransferJob,
) -> AppResult<u64> {
    let mut local = tokio::fs::File::open(&input.local_path).await?;
    let total = local.metadata().await.map(|m| m.len()).unwrap_or(0);
    let sftp = session(state, &input.connection_id).await?;
    let mut remote = sftp.create(input.remote_path.clone()).await.map_err(sftp_err)?;

    emit_progress(app, &input.id, 0, total, false, None);
    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;
    let step = progress_step(total);
    loop {
        let n = local.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote.write_all(&buf[..n]).await.map_err(sftp_err)?;
        transferred += n as u64;
        if transferred - last_emit >= step {
            last_emit = transferred;
            emit_progress(app, &input.id, transferred, total, false, None);
        }
    }
    remote.shutdown().await.map_err(sftp_err)?;
    emit_progress(app, &input.id, transferred, total, true, None);
    Ok(transferred)
}

async fn transfer_download(
    app: &AppHandle,
    state: &State<'_, AppState>,
    input: &SftpTransferJob,
) -> AppResult<u64> {
    let sftp = session(state, &input.connection_id).await?;
    let total = sftp
        .metadata(input.remote_path.clone())
        .await
        .map(|a| a.size.unwrap_or(0))
        .unwrap_or(0);
    let mut remote = sftp.open(input.remote_path.clone()).await.map_err(sftp_err)?;
    let mut local = tokio::fs::File::create(&input.local_path).await?;

    emit_progress(app, &input.id, 0, total, false, None);
    let mut buf = vec![0u8; TRANSFER_CHUNK];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;
    let step = progress_step(total);
    loop {
        let n = remote.read(&mut buf).await.map_err(sftp_err)?;
        if n == 0 {
            break;
        }
        local.write_all(&buf[..n]).await?;
        transferred += n as u64;
        if transferred - last_emit >= step {
            last_emit = transferred;
            emit_progress(app, &input.id, transferred, total, false, None);
        }
    }
    local.flush().await?;
    emit_progress(app, &input.id, transferred, total, true, None);
    Ok(transferred)
}

/// Drop the cached SFTP session for a connection.
#[tauri::command]
pub async fn sftp_disconnect(state: State<'_, AppState>, connection_id: String) -> AppResult<()> {
    state.sftp.lock().await.disconnect(&connection_id);
    Ok(())
}
