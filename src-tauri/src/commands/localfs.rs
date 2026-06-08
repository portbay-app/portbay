//! Local filesystem listing for the deploy picker.
//!
//! The deploy view needs to show the user what's on *their* machine (which
//! sub-directory to sync) before pushing it to a remote host. These commands
//! enumerate local directories; the recursive [`walk_files`] helper is shared
//! with [`crate::commands::deploy`] to build the upload set.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// One local file or directory entry, mirroring the SFTP entry shape so the UI
/// can reuse the same row rendering for local and remote listings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

fn entry_for(path: &Path) -> AppResult<LocalEntry> {
    let meta = std::fs::metadata(path)
        .map_err(|e| AppError::BadInput(format!("{}: {e}", path.display())))?;
    Ok(LocalEntry {
        name: path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string(),
        path: path.display().to_string(),
        is_dir: meta.is_dir(),
        size: if meta.is_dir() { 0 } else { meta.len() },
    })
}

/// List a local directory (dirs first, then name), skipping dotfiles' noise is
/// left to the caller — everything is returned so the picker can show it.
#[tauri::command]
pub async fn local_list_dir(path: String) -> AppResult<Vec<LocalEntry>> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(AppError::BadInput(format!("not a folder: {path}")));
    }
    let mut out: Vec<LocalEntry> = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| AppError::BadInput(format!("{path}: {e}")))? {
        let entry = entry.map_err(|e| AppError::BadInput(format!("{path}: {e}")))?;
        if let Ok(e) = entry_for(&entry.path()) {
            out.push(e);
        }
    }
    out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(out)
}

/// Stat a single local path.
#[tauri::command]
pub async fn local_stat(path: String) -> AppResult<LocalEntry> {
    entry_for(&PathBuf::from(&path))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSearchResult {
    pub entries: Vec<LocalEntry>,
    pub scanned: u64,
    /// True when the walk stopped at a result/scan/depth cap.
    pub truncated: bool,
}

const LOCAL_SEARCH_MAX_RESULTS: usize = 500;
const LOCAL_SEARCH_MAX_SCANNED: u64 = 150_000;
const LOCAL_SEARCH_MAX_DEPTH: usize = 24;

/// Recursive name search under a local folder — the local twin of
/// `sftp_search`, powering the file pane's deep-search toggle. Plain queries
/// match as a case-insensitive substring; `*` / `?` switch to a glob over the
/// whole name (`*.zip`). Hidden directories aren't descended into (they're
/// mostly caches and would burn the scan budget), symlinked dirs are skipped
/// (cycle guard), and the walk is bounded by result/scan/depth caps. Read-only
/// metadata, same exposure class as [`local_list_dir`]; reading file contents
/// still requires the SFTP approval flow.
#[tauri::command]
pub async fn local_search(root: String, query: String) -> AppResult<LocalSearchResult> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(LocalSearchResult { entries: Vec::new(), scanned: 0, truncated: false });
    }
    let pattern = if q.contains('*') || q.contains('?') {
        q
    } else {
        format!("*{q}*")
    };
    let root_path = PathBuf::from(&root);
    if !root_path.is_dir() {
        return Err(AppError::BadInput(format!("not a folder: {root}")));
    }

    // Plain blocking walk on a worker thread — local disks are fast and the
    // caps keep worst cases (huge homedirs) bounded.
    tokio::task::spawn_blocking(move || {
        let mut queue: std::collections::VecDeque<(PathBuf, usize)> =
            std::collections::VecDeque::new();
        queue.push_back((root_path, 0));
        let mut entries: Vec<LocalEntry> = Vec::new();
        let mut scanned: u64 = 0;
        let mut truncated = false;

        'walk: while let Some((dir, depth)) = queue.pop_front() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue; // unreadable directory — skip, not fatal
            };
            for entry in read.flatten() {
                scanned += 1;
                if scanned > LOCAL_SEARCH_MAX_SCANNED {
                    truncated = true;
                    break 'walk;
                }
                let path = entry.path();
                let name = match path.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let Ok(meta) = entry.metadata() else { continue };
                if crate::commands::sftp::glob_match(&pattern, &name.to_lowercase()) {
                    entries.push(LocalEntry {
                        name: name.clone(),
                        path: path.display().to_string(),
                        is_dir: meta.is_dir(),
                        size: if meta.is_dir() { 0 } else { meta.len() },
                    });
                    if entries.len() >= LOCAL_SEARCH_MAX_RESULTS {
                        truncated = true;
                        break 'walk;
                    }
                }
                let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
                if meta.is_dir()
                    && !is_symlink
                    && !name.starts_with('.')
                    && depth < LOCAL_SEARCH_MAX_DEPTH
                {
                    queue.push_back((path, depth + 1));
                }
            }
        }
        Ok(LocalSearchResult { entries, scanned, truncated })
    })
    .await
    .map_err(|e| AppError::Internal(format!("search task panicked: {e}")))?
}

/// A file to upload during a deploy: its absolute local path and the POSIX
/// relative path it should land at under the remote root.
pub struct WalkedFile {
    pub abs: PathBuf,
    pub rel: String,
}

/// Wire shape of one walked file for the frontend folder-upload flow.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkedLocalFile {
    /// Absolute local path.
    pub path: String,
    /// POSIX-style path relative to the walked root (maps onto the remote side).
    pub rel: String,
    pub size: u64,
}

/// Recursively enumerate every file under a local folder, for uploading a
/// whole directory over SFTP. Listing is read-only metadata (names + sizes),
/// the same exposure class as [`local_list_dir`]; actually *reading* the files
/// still requires the path to be in the SFTP approved set (the folder picker
/// or the host-side access prompt put it there).
#[tauri::command]
pub async fn local_walk_files(root: String) -> AppResult<Vec<WalkedLocalFile>> {
    let root_path = PathBuf::from(&root);
    let files = walk_files(&root_path, &[])?;
    Ok(files
        .into_iter()
        .map(|f| {
            let size = std::fs::metadata(&f.abs).map(|m| m.len()).unwrap_or(0);
            WalkedLocalFile {
                path: f.abs.display().to_string(),
                rel: f.rel,
                size,
            }
        })
        .collect())
}

/// Recursively collect every file under `root`, skipping any whose relative
/// path contains an excluded segment (e.g. `node_modules`, `.git`). Symlinks
/// are followed by `read_dir`/`metadata`; cycles are bounded by `max_depth`.
/// Returns POSIX-style relative paths so they map cleanly onto the remote side.
pub fn walk_files(root: &Path, exclude: &[String]) -> AppResult<Vec<WalkedFile>> {
    if !root.is_dir() {
        return Err(AppError::BadInput(format!(
            "deploy source folder not found: {}",
            root.display()
        )));
    }
    let mut out = Vec::new();
    walk_into(root, root, exclude, 0, &mut out)?;
    Ok(out)
}

const MAX_WALK_DEPTH: usize = 64;

fn walk_into(
    root: &Path,
    dir: &Path,
    exclude: &[String],
    depth: usize,
    out: &mut Vec<WalkedFile>,
) -> AppResult<()> {
    if depth > MAX_WALK_DEPTH {
        return Ok(());
    }
    let entries =
        std::fs::read_dir(dir).map_err(|e| AppError::Internal(format!("walk {dir:?}: {e}")))?;
    for entry in entries {
        let entry = entry.map_err(|e| AppError::Internal(format!("walk {dir:?}: {e}")))?;
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if exclude.iter().any(|x| x == name) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            walk_into(root, &path, exclude, depth + 1, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| name.to_string());
            out.push(WalkedFile { abs: path, rel });
        }
    }
    Ok(())
}
