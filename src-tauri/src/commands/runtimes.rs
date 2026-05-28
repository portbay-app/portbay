//! IPC surface for the Languages container.
//!
//! Detect-first: PortBay reuses runtimes already on the machine and never
//! bundles or copies one. This module adds the user-controlled surface on top
//! of detection:
//!
//!   - `list_runtimes()` — every language, detected + manually-added versions,
//!     with the per-language default marked.
//!   - `add_runtime_by_path(lang, path)` — register an existing binary the
//!     detector didn't find (e.g. a custom-compiled PHP). Reuses it in place.
//!   - `remove_runtime_path(lang, version)` — drop a manual entry.
//!   - `set_default_runtime(lang, version)` — set/clear the default version a
//!     new project inherits for that language.
//!   - `install_runtime(lang, version)` — downloads a signed PortBay-managed
//!     runtime archive, verifies it, extracts it into Application Support, and
//!     persists it into the managed-runtime registry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::registry::{ManagedRuntime, ManualRuntime};
use crate::runtimes::{self, major_minor, runtime_by_id, LanguageView};
use crate::state::AppState;

const RUNTIME_INSTALL_CHANNEL: &str = "portbay://runtime-install";
const RUNTIME_MANIFEST_URL: &str =
    "https://github.com/portbay-app/portbay-runtimes/releases/latest/download/manifest.json";
const RUNTIME_MANIFEST_SIGNATURE_URL: &str =
    "https://github.com/portbay-app/portbay-runtimes/releases/latest/download/manifest.json.sig";
const UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDNBNEI4QjdFQzA4NkFBQjUKUldTMXFvYkFmb3RMT3J1MlZFdm51bDVlb3ZOU0cyNy94d0MvNjRKWGQ4eDRWUkxWR1poZ3VZMTgK";

#[tauri::command]
pub async fn list_runtimes(state: State<'_, AppState>) -> AppResult<Vec<LanguageView>> {
    let reg = load_registry(&state)?;
    Ok(runtimes::list_all(&reg.runtimes))
}

/// Register an existing binary as a manual install for `lang`. PortBay probes
/// its version and reuses the binary in place — it is never copied.
#[tauri::command]
pub async fn add_runtime_by_path(
    state: State<'_, AppState>,
    lang: String,
    path: String,
) -> AppResult<Vec<LanguageView>> {
    let runtime = runtime_by_id(&lang)
        .ok_or_else(|| AppError::BadInput(format!("unknown language `{lang}`")))?;

    let binary = PathBuf::from(&path);
    if !binary.is_file() {
        return Err(AppError::BadInput(format!("no binary found at {path}")));
    }

    // Probe the version; a binary that doesn't report one isn't the runtime
    // the user thinks it is.
    let version = runtime.probe_version(&binary).ok_or_else(|| {
        AppError::BadInput(format!(
            "{path} didn't report a {lang} version — is it the right binary?"
        ))
    })?;
    let version = major_minor(&version);

    let mut reg = load_registry(&state)?;
    let canon = binary.canonicalize().unwrap_or_else(|_| binary.clone());
    let exists = reg
        .runtimes
        .manual
        .iter()
        .any(|m| m.binary.canonicalize().unwrap_or_else(|_| m.binary.clone()) == canon);
    if !exists {
        reg.runtimes.manual.push(ManualRuntime {
            lang: lang.clone(),
            version,
            binary,
        });
        save_registry(&state, &reg)?;
    }

    Ok(runtimes::list_all(&reg.runtimes))
}

/// Remove a manually-added install. No-op if it wasn't manual / not present.
#[tauri::command]
pub async fn remove_runtime_path(
    state: State<'_, AppState>,
    lang: String,
    version: String,
) -> AppResult<Vec<LanguageView>> {
    let mut reg = load_registry(&state)?;
    let before = reg.runtimes.manual.len();
    reg.runtimes
        .manual
        .retain(|m| !(m.lang == lang && m.version == version));
    if reg.runtimes.manual.len() != before {
        save_registry(&state, &reg)?;
    }
    Ok(runtimes::list_all(&reg.runtimes))
}

/// Remove a PortBay-managed runtime and delete its owned install directory.
/// Manual and detected runtimes are intentionally untouched.
#[tauri::command]
pub async fn remove_managed_runtime(
    state: State<'_, AppState>,
    lang: String,
    version: String,
) -> AppResult<Vec<LanguageView>> {
    let mut reg = load_registry(&state)?;
    let Some(idx) = reg
        .runtimes
        .managed
        .iter()
        .position(|m| m.lang == lang && m.version == version)
    else {
        return Ok(runtimes::list_all(&reg.runtimes));
    };
    let removed = reg.runtimes.managed.remove(idx);
    if let Some(install_dir) = removed.binary.parent().and_then(Path::parent) {
        if install_dir.starts_with(runtime_dest_root()?) {
            std::fs::remove_dir_all(install_dir)?;
        }
    }
    save_registry(&state, &reg)?;
    state.reconciler.mark_dirty();
    Ok(runtimes::list_all(&reg.runtimes))
}

/// Set (or clear, when `version` is empty/None) the default version a new
/// project inherits for `lang`.
#[tauri::command]
pub async fn set_default_runtime(
    state: State<'_, AppState>,
    lang: String,
    version: Option<String>,
) -> AppResult<Vec<LanguageView>> {
    let mut reg = load_registry(&state)?;
    match version {
        Some(v) if !v.trim().is_empty() => {
            reg.runtimes.defaults.insert(lang, v);
        }
        _ => {
            reg.runtimes.defaults.remove(&lang);
        }
    }
    save_registry(&state, &reg)?;
    Ok(runtimes::list_all(&reg.runtimes))
}

/// Progress events for [`install_runtime`], streamed over the `Channel`.
/// `done` carries the final success flag so the UI can settle on a clean exit;
/// on failure the command also returns `Err`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InstallEvent {
    Log { line: String },
    Progress { downloaded: u64, total: Option<u64> },
    Done { success: bool },
}

/// Download and install a PortBay-managed runtime from the signed manifest.
/// `version = None` picks the newest manifest entry for this language and
/// architecture; callers that know the desired pin should pass it explicitly.
#[tauri::command]
pub async fn install_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
    lang: String,
    version: Option<String>,
    on_event: Channel<InstallEvent>,
) -> AppResult<()> {
    if !is_installable_runtime(&lang) {
        return Err(AppError::BadInput(format!(
            "no PortBay-managed download is wired for `{lang}`"
        )));
    }

    let _ = on_event.send(InstallEvent::Log {
        line: "Fetching signed PortBay runtime manifest…".into(),
    });
    let manifest = fetch_signed_manifest().await?;
    let arch = crate::runtimes::download::manifest::current_arch();
    let requested = version.as_deref().unwrap_or("");
    let entry = if requested.is_empty() {
        newest_entry(&manifest, &lang, arch)
    } else {
        manifest.select(&lang, requested, arch).cloned()
    }
    .ok_or_else(|| {
        AppError::BadInput(format!(
            "no PortBay runtime build found for {lang}{} on {arch}",
            if requested.is_empty() {
                "".to_string()
            } else {
                format!(" {requested}")
            }
        ))
    })?;

    let dest_root = runtime_dest_root()?;
    let expected_binary = expected_binary_rel(&entry.lang)?;
    let install_lang = entry.lang.clone();
    let install_version = entry.version.clone();
    let install_arch = entry.arch.clone();
    let app_for_progress = app.clone();
    let channel_for_progress = on_event.clone();
    let probe_lang = install_lang.clone();
    let probe_version = install_version.clone();
    let _ = on_event.send(InstallEvent::Log {
        line: format!(
            "Installing PortBay {} {} ({})…",
            install_lang, install_version, install_arch
        ),
    });
    let binary = crate::runtimes::download::install::fetch_and_install(
        &entry,
        &dest_root,
        expected_binary,
        move |downloaded, total| {
            let event = InstallEvent::Progress { downloaded, total };
            let _ = channel_for_progress.send(event.clone());
            let _ = app_for_progress.emit(RUNTIME_INSTALL_CHANNEL, event);
        },
        |bin| probe_runtime(&probe_lang, &probe_version, bin),
    )
    .await
    .map_err(|e| AppError::Internal(format!("runtime install failed: {e}")))?;

    let install_dir = binary
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AppError::Internal("installed runtime path is malformed".into()))?;
    strip_quarantine(install_dir)?;

    let mut reg = load_registry(&state)?;
    reg.runtimes.managed.retain(|m| {
        !(m.lang == install_lang
            && crate::runtimes::version_matches(&m.version, &install_version)
            && m.arch == install_arch)
    });
    reg.runtimes.managed.push(ManagedRuntime {
        lang: install_lang,
        version: install_version,
        binary,
        arch: install_arch,
    });
    save_registry(&state, &reg)?;
    state.reconciler.mark_dirty();

    let done = InstallEvent::Done { success: true };
    let _ = app.emit(RUNTIME_INSTALL_CHANNEL, done.clone());
    let _ = on_event.send(done);
    Ok(())
}

/// Fetch the signed PortBay runtimes manifest and verify it. Shared by the
/// runtime and database-engine installers — both pull builds from the same
/// signed manifest published by the portbay-runtimes repo.
pub(crate) async fn fetch_signed_manifest(
) -> AppResult<crate::runtimes::download::manifest::RuntimeManifest> {
    let manifest_url = std::env::var("PORTBAY_RUNTIME_MANIFEST_URL")
        .unwrap_or_else(|_| RUNTIME_MANIFEST_URL.to_string());
    let signature_url = std::env::var("PORTBAY_RUNTIME_MANIFEST_SIGNATURE_URL")
        .unwrap_or_else(|_| RUNTIME_MANIFEST_SIGNATURE_URL.to_string());
    let manifest_bytes = reqwest::get(&manifest_url)
        .await
        .map_err(|e| AppError::Internal(format!("runtime manifest fetch failed: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Internal(format!("runtime manifest fetch failed: {e}")))?
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("runtime manifest read failed: {e}")))?;
    let signature = reqwest::get(&signature_url)
        .await
        .map_err(|e| AppError::Internal(format!("runtime manifest signature fetch failed: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Internal(format!("runtime manifest signature fetch failed: {e}")))?
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("runtime manifest signature read failed: {e}")))?;
    crate::runtimes::download::manifest::verify_and_parse(
        &manifest_bytes,
        &signature,
        UPDATER_PUBKEY,
    )
    .map_err(|e| AppError::Internal(format!("runtime manifest verification failed: {e}")))
}

/// Newest manifest entry for a `lang`/`arch`, by descending version. Shared
/// with the database-engine installer (engine id is used as the `lang`).
pub(crate) fn newest_entry(
    manifest: &crate::runtimes::download::manifest::RuntimeManifest,
    lang: &str,
    arch: &str,
) -> Option<crate::runtimes::download::manifest::RuntimeEntry> {
    let mut entries = manifest
        .entries
        .iter()
        .filter(|e| e.lang == lang && e.arch == arch)
        .cloned()
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.version.cmp(&a.version));
    entries.into_iter().next()
}

fn runtime_dest_root() -> AppResult<PathBuf> {
    let mut dir =
        dirs::data_dir().ok_or_else(|| AppError::Internal("no data directory available".into()))?;
    dir.push("PortBay");
    dir.push("runtimes");
    Ok(dir)
}

/// Runtimes PortBay can install as its own managed builds: the detect-first
/// language runtimes (php, …) plus the bundled web servers (nginx, apache).
/// The web servers aren't in the `runtimes::registry()` language list, so they
/// are allow-listed here explicitly.
fn is_installable_runtime(lang: &str) -> bool {
    matches!(lang, "nginx" | "apache") || runtime_by_id(lang).is_some()
}

/// Relative path to the primary binary inside a managed archive, per lang.
/// This is the layout the runtimes-build CI must produce.
fn expected_binary_rel(lang: &str) -> AppResult<&'static Path> {
    match lang {
        "php" => Ok(Path::new("bin/php")),
        "node" => Ok(Path::new("bin/node")),
        "nginx" => Ok(Path::new("sbin/nginx")),
        "apache" => Ok(Path::new("bin/httpd")),
        _ => Err(AppError::BadInput(format!(
            "PortBay-managed downloads are not wired for `{lang}` yet"
        ))),
    }
}

fn probe_runtime(lang: &str, version: &str, bin: &Path) -> bool {
    match lang {
        "node" => Command::new(bin)
            .arg("--version")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| {
                let text = String::from_utf8_lossy(&out.stdout);
                // `node --version` prints `v22.14.0`; check the major.minor prefix.
                text.contains(&major_minor(version))
            })
            .unwrap_or(false),
        "php" => {
            let Some(install_dir) = bin.parent().and_then(Path::parent) else {
                return false;
            };
            if !install_dir.join("sbin/php-fpm").is_file() {
                return false;
            }
            Command::new(bin)
                .arg("--version")
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| {
                    let text = String::from_utf8_lossy(&out.stdout);
                    text.contains("PHP") && text.contains(&major_minor(version))
                })
                .unwrap_or(false)
        }
        // nginx prints `nginx version: nginx/1.x` to stderr on `-v`.
        "nginx" => Command::new(bin)
            .arg("-v")
            .output()
            .ok()
            .map(|out| {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                text.to_lowercase().contains("nginx")
            })
            .unwrap_or(false),
        // httpd prints `Server version: Apache/2.x` on `-v`.
        "apache" => Command::new(bin)
            .arg("-v")
            .output()
            .ok()
            .map(|out| {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                text.contains("Apache")
            })
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn strip_quarantine(path: &Path) -> AppResult<()> {
    let status = Command::new("xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(path)
        .status()
        .map_err(|e| AppError::Internal(format!("couldn't clear runtime quarantine: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::Internal(format!(
            "couldn't clear runtime quarantine from {}",
            path.display()
        )))
    }
}

#[cfg(not(target_os = "macos"))]
fn strip_quarantine(_path: &Path) -> AppResult<()> {
    Ok(())
}

/// Apply edits from an editable config tab (e.g. PHP's FPM / php.ini tabs).
/// `patches` maps each dirty row's `key` to its new string value. The change
/// is validated + persisted into the registry, then any services it affects
/// (the version's FPM pool) are restarted best-effort so it takes effect now.
/// Returns the refreshed language list so the panel re-renders saved values.
#[tauri::command]
pub async fn update_runtime_config(
    state: State<'_, AppState>,
    lang: String,
    version: String,
    tab_id: String,
    patches: BTreeMap<String, String>,
) -> AppResult<Vec<LanguageView>> {
    let runtime = runtime_by_id(&lang)
        .ok_or_else(|| AppError::BadInput(format!("unknown language `{lang}`")))?;

    let mut reg = load_registry(&state)?;
    let result = runtime
        .apply_config(&version, &tab_id, &patches, &mut reg.runtimes)
        .map_err(AppError::BadInput)?;
    save_registry(&state, &reg)?;

    // Restart affected services so the new config is live immediately. A
    // process that isn't currently running (no project uses this version yet)
    // simply errors out — ignored; the next reconcile tick will pick the
    // config up when the pool first starts.
    if let Ok(client) = state.pc_client() {
        for pid in &result.restart_processes {
            let _ = client.restart(pid).await;
        }
    }

    Ok(runtimes::list_all(&reg.runtimes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_binary_rel_node_is_bin_node() {
        let rel = expected_binary_rel("node").unwrap();
        assert_eq!(rel, Path::new("bin/node"));
    }

    #[test]
    fn expected_binary_rel_php_is_bin_php() {
        let rel = expected_binary_rel("php").unwrap();
        assert_eq!(rel, Path::new("bin/php"));
    }

    #[test]
    fn expected_binary_rel_unknown_is_error() {
        assert!(expected_binary_rel("ruby").is_err());
    }

    #[test]
    fn probe_runtime_node_accepts_valid_version_output() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("node");
        // Write a shell script that mimics `node --version` → `v22.14.0`.
        std::fs::write(&bin, b"#!/bin/sh\necho v22.14.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(
            probe_runtime("node", "22.14.0", &bin),
            "probe should accept a binary echoing the right version"
        );
    }

    #[test]
    fn probe_runtime_node_rejects_wrong_version() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("node");
        std::fs::write(&bin, b"#!/bin/sh\necho v18.20.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(
            !probe_runtime("node", "22.14.0", &bin),
            "probe must reject a binary reporting the wrong major.minor"
        );
    }
}
