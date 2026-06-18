//! Auto-update commands — thin wrappers over `tauri-plugin-updater`'s Rust API,
//! plus a rollback safety net.
//!
//! The frontend stays declarative: `check_for_update` reports whether a newer
//! release is published (reading the GitHub-hosted `latest.json` configured in
//! `tauri.conf.json::plugins.updater`), and `install_update` downloads +
//! verifies the minisign signature + installs it, then relaunches into the new
//! version. Both flow through the standard `AppError` envelope so the existing
//! toast / ErrorEnvelope path renders failures with no special-casing.
//!
//! ## Rollback
//! `tauri-plugin-updater` verifies the signature *before* it applies a package,
//! so a tampered/corrupt download is never installed. What it can't guard is a
//! correctly-signed build that won't *launch*. So `install_update` first snapshots
//! the current (known-good) `.app` bundle and writes a `pending` marker; the new
//! version's first boot must confirm health ([`confirm_update_health`], called by
//! the UI once it has mounted), which prunes the snapshot. If the new version
//! crash-loops instead, [`rollback_on_startup`] — run from the Tauri setup hook —
//! restores the snapshot after a short grace window. A failed install restores
//! immediately. Bundle snapshot/restore is macOS-only (the `.app` model); other
//! targets keep the prior behavior with no safety net.

use serde::Serialize;
use tauri_plugin_updater::UpdaterExt;

use crate::error::{AppError, AppResult};

/// What the frontend needs to render the "update available" toast and the
/// Settings → Updates row. A `None` return from [`check_for_update`] means the
/// running build is already current.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Version offered by the manifest (e.g. `"0.2.0"`).
    pub version: String,
    /// Version currently running.
    pub current_version: String,
    /// Release notes from the manifest, if the producer set them.
    pub notes: Option<String>,
    /// Publish date from the manifest, if present.
    pub pub_date: Option<String>,
}

/// Check the configured endpoint for a newer signed release. Returns `None`
/// when up to date. Network / parse / signature errors surface as an envelope.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> AppResult<Option<UpdateInfo>> {
    let updater = app
        .updater()
        .map_err(|e| AppError::Internal(format!("updater unavailable: {e}")))?;

    match updater.check().await {
        Ok(Some(update)) => Ok(Some(UpdateInfo {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
            notes: update.body.clone(),
            pub_date: update.date.map(|d| d.to_string()),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(AppError::Internal(format!("update check failed: {e}"))),
    }
}

/// Download, verify, and install the latest update, then relaunch. The plugin
/// rejects any package whose signature doesn't match the configured pubkey, so
/// a tampered binary fails here rather than running. Before applying, the current
/// bundle is snapshotted so a failed install — or a new version that won't launch
/// — can be rolled back. Never returns on success — `app.restart()` replaces the
/// process.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> AppResult<()> {
    let updater = app
        .updater()
        .map_err(|e| AppError::Internal(format!("updater unavailable: {e}")))?;

    let update = updater
        .check()
        .await
        .map_err(|e| AppError::Internal(format!("update check failed: {e}")))?
        .ok_or_else(|| AppError::Internal("no update available to install".into()))?;

    // Keep the current known-good bundle so a failed install, or a new version
    // that won't launch, can be restored. Best-effort: if staging fails we
    // proceed without the safety net rather than block the update entirely.
    let staged = rollback::stage_backup(&app, &update.current_version, &update.version);

    match update
        .download_and_install(|_chunk_len, _content_len| {}, || {})
        .await
    {
        Ok(()) => {
            // Verified + installed. The pending marker rides into the new
            // version's first boot: it confirms health (pruning the snapshot)
            // or, if it won't launch, a later boot rolls back.
            app.restart()
        }
        Err(e) => {
            // The signature gate means a bad package was never applied, but
            // restore anyway to guarantee the prior bundle is intact, then drop
            // the marker so the next boot doesn't think an update is pending.
            if staged {
                rollback::restore_and_clear(&app);
            }
            Err(AppError::Internal(format!("update install failed: {e}")))
        }
    }
}

/// The UI calls this once it has mounted and reached a steady state — the
/// "launched successfully" signal. It clears the pending-update marker and
/// prunes the kept-back prior version. Best-effort and idempotent; it never
/// errors the caller (a no-op when no update is pending).
#[tauri::command]
pub fn confirm_update_health(app: tauri::AppHandle) -> AppResult<()> {
    rollback::confirm_health(&app);
    Ok(())
}

/// Run once from the Tauri setup hook, before the window is shown. If the
/// previous boot was a fresh update that never confirmed health within the grace
/// window, restore the prior version and relaunch into it.
pub fn rollback_on_startup(app: &tauri::AppHandle) {
    rollback::on_startup(app);
}

/// Update-rollback bookkeeping. Pure decision logic + a small on-disk marker and
/// (macOS) bundle snapshot/restore. Lives under `<app_data_dir>/updates/`:
/// `pending.json` (the marker) and `rollback/<Name>.app` (the snapshot).
mod rollback {
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Serialize};
    use tauri::Manager;

    /// Boots we let a freshly-installed update proceed *without* a health confirm
    /// before rolling back. 2 → roll back on the 3rd unconfirmed boot, which
    /// tolerates a couple of force-quits-before-confirm without a false rollback,
    /// while still self-healing a genuine crash-loop quickly.
    const MAX_BOOT_ATTEMPTS: u32 = 2;

    /// The marker written between installing an update and the new version
    /// confirming it launched. Its presence at boot means "an update is on
    /// probation"; its absence means a normal boot.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub(super) struct PendingUpdate {
        pub prior_version: String,
        pub new_version: String,
        #[serde(default)]
        pub boot_attempts: u32,
    }

    /// What a boot should do given the stored marker.
    #[derive(Debug, PartialEq)]
    pub(super) enum BootDecision {
        /// No update pending — normal boot.
        Idle,
        /// Still within the grace window; persist the incremented attempt count
        /// and keep booting.
        Continue(PendingUpdate),
        /// Out of grace without a health confirm — restore the prior version.
        Rollback(PendingUpdate),
    }

    /// Pure: given the stored marker and the attempt budget, decide what this
    /// boot does (and what the marker should become). Separated from all I/O so
    /// the state machine is unit-testable.
    pub(super) fn decide_on_boot(
        pending: Option<PendingUpdate>,
        max_attempts: u32,
    ) -> BootDecision {
        match pending {
            None => BootDecision::Idle,
            Some(mut p) => {
                p.boot_attempts = p.boot_attempts.saturating_add(1);
                if p.boot_attempts > max_attempts {
                    BootDecision::Rollback(p)
                } else {
                    BootDecision::Continue(p)
                }
            }
        }
    }

    fn updates_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path().app_data_dir().ok().map(|d| d.join("updates"))
    }
    fn pending_path(dir: &Path) -> PathBuf {
        dir.join("pending.json")
    }
    fn backup_dir(dir: &Path) -> PathBuf {
        dir.join("rollback")
    }

    pub(super) fn read_pending(dir: &Path) -> Option<PendingUpdate> {
        let raw = fs::read_to_string(pending_path(dir)).ok()?;
        serde_json::from_str(&raw).ok()
    }
    pub(super) fn write_pending(dir: &Path, p: &PendingUpdate) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let raw = serde_json::to_string_pretty(p).unwrap_or_default();
        fs::write(pending_path(dir), raw)
    }
    pub(super) fn clear_pending(dir: &Path) {
        let _ = fs::remove_file(pending_path(dir));
    }
    pub(super) fn prune_backup(dir: &Path) {
        let _ = fs::remove_dir_all(backup_dir(dir));
    }

    /// Recursively copy a directory tree, replacing `dst`. Used to snapshot and
    /// restore a macOS `.app` bundle (symlinks within frameworks are preserved).
    #[cfg(target_os = "macos")]
    fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
        if dst.exists() {
            fs::remove_dir_all(dst)?;
        }
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if ty.is_symlink() {
                let target = fs::read_link(&from)?;
                std::os::unix::fs::symlink(target, &to)?;
            } else if ty.is_dir() {
                copy_dir(&from, &to)?;
            } else {
                fs::copy(&from, &to)?;
            }
        }
        Ok(())
    }

    /// The running app's `.app` bundle, or `None` when not launched from one (a
    /// dev binary, tests). `…/PortBay.app/Contents/MacOS/<bin>` → 3 ancestors up.
    #[cfg(target_os = "macos")]
    fn current_bundle() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let app = exe.ancestors().nth(3)?;
        if app.extension().and_then(|e| e.to_str()) == Some("app") && app.is_dir() {
            Some(app.to_path_buf())
        } else {
            None
        }
    }

    #[cfg(target_os = "macos")]
    fn restore_bundle(dir: &Path) -> std::io::Result<()> {
        use std::io::{Error, ErrorKind};
        let bundle = current_bundle()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "not running from an .app bundle"))?;
        let name = bundle
            .file_name()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "bundle has no name"))?;
        let snapshot = backup_dir(dir).join(name);
        if !snapshot.is_dir() {
            return Err(Error::new(ErrorKind::NotFound, "no rollback snapshot"));
        }
        copy_dir(&snapshot, &bundle)
    }

    /// Snapshot the current bundle and write the pending marker. Returns whether
    /// a usable safety net was staged. macOS-only; a no-op elsewhere.
    #[cfg(target_os = "macos")]
    pub(super) fn stage_backup(
        app: &tauri::AppHandle,
        prior_version: &str,
        new_version: &str,
    ) -> bool {
        let Some(dir) = updates_dir(app) else {
            return false;
        };
        let Some(bundle) = current_bundle() else {
            return false;
        };
        let Some(name) = bundle.file_name() else {
            return false;
        };
        if copy_dir(&bundle, &backup_dir(&dir).join(name)).is_err() {
            return false;
        }
        let p = PendingUpdate {
            prior_version: prior_version.to_string(),
            new_version: new_version.to_string(),
            boot_attempts: 0,
        };
        write_pending(&dir, &p).is_ok()
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn stage_backup(
        _app: &tauri::AppHandle,
        _prior_version: &str,
        _new_version: &str,
    ) -> bool {
        false
    }

    /// Restore the prior bundle (macOS) and drop the marker + snapshot. Used when
    /// an install fails after staging.
    pub(super) fn restore_and_clear(app: &tauri::AppHandle) {
        let Some(dir) = updates_dir(app) else {
            return;
        };
        #[cfg(target_os = "macos")]
        let _ = restore_bundle(&dir);
        clear_pending(&dir);
        prune_backup(&dir);
    }

    /// The new version reports it launched cleanly: drop the marker and prune the
    /// snapshot. No-op when nothing is pending.
    pub(super) fn confirm_health(app: &tauri::AppHandle) {
        let Some(dir) = updates_dir(app) else {
            return;
        };
        if read_pending(&dir).is_some() {
            clear_pending(&dir);
            prune_backup(&dir);
        }
    }

    /// Setup-time watchdog: continue (and count the boot) while within grace, or
    /// restore the prior version and relaunch once a probationary update has
    /// failed to confirm health.
    pub(super) fn on_startup(app: &tauri::AppHandle) {
        let Some(dir) = updates_dir(app) else {
            return;
        };
        match decide_on_boot(read_pending(&dir), MAX_BOOT_ATTEMPTS) {
            BootDecision::Idle => {}
            BootDecision::Continue(p) => {
                let _ = write_pending(&dir, &p);
            }
            BootDecision::Rollback(_) => {
                #[cfg(target_os = "macos")]
                let restored = restore_bundle(&dir).is_ok();
                #[cfg(not(target_os = "macos"))]
                let restored = false;
                // Clear unconditionally: a successful restore must not re-trigger,
                // and a failed restore must not loop forever on a broken build.
                clear_pending(&dir);
                prune_backup(&dir);
                if restored {
                    app.restart();
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn pending(attempts: u32) -> PendingUpdate {
            PendingUpdate {
                prior_version: "0.1.0".into(),
                new_version: "0.2.0".into(),
                boot_attempts: attempts,
            }
        }

        #[test]
        fn no_marker_is_an_idle_boot() {
            assert_eq!(decide_on_boot(None, MAX_BOOT_ATTEMPTS), BootDecision::Idle);
        }

        #[test]
        fn within_grace_continues_and_counts_the_boot() {
            // First boot after install (0 → 1) and the next (1 → 2) keep going.
            assert_eq!(
                decide_on_boot(Some(pending(0)), 2),
                BootDecision::Continue(pending(1))
            );
            assert_eq!(
                decide_on_boot(Some(pending(1)), 2),
                BootDecision::Continue(pending(2))
            );
        }

        #[test]
        fn exceeding_grace_rolls_back() {
            // 2 → 3 is past the budget of 2: restore the prior version.
            assert_eq!(
                decide_on_boot(Some(pending(2)), 2),
                BootDecision::Rollback(pending(3))
            );
        }

        #[test]
        fn marker_round_trips_and_clears() {
            let dir = tempfile::tempdir().unwrap();
            assert!(read_pending(dir.path()).is_none());
            let p = pending(0);
            write_pending(dir.path(), &p).unwrap();
            assert_eq!(read_pending(dir.path()), Some(p));
            clear_pending(dir.path());
            assert!(read_pending(dir.path()).is_none());
        }

        #[test]
        fn prune_removes_the_snapshot_dir() {
            let dir = tempfile::tempdir().unwrap();
            let snap = backup_dir(dir.path()).join("PortBay.app");
            fs::create_dir_all(&snap).unwrap();
            fs::write(snap.join("marker"), b"x").unwrap();
            assert!(backup_dir(dir.path()).exists());
            prune_backup(dir.path());
            assert!(!backup_dir(dir.path()).exists());
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn copy_dir_snapshots_and_restores_a_tree() {
            let root = tempfile::tempdir().unwrap();
            let src = root.path().join("PortBay.app");
            fs::create_dir_all(src.join("Contents/MacOS")).unwrap();
            fs::write(src.join("Contents/Info.plist"), b"plist").unwrap();
            fs::write(src.join("Contents/MacOS/PortBay"), b"binary").unwrap();

            let snap = root.path().join("snapshot.app");
            copy_dir(&src, &snap).unwrap();
            assert_eq!(
                fs::read(snap.join("Contents/MacOS/PortBay")).unwrap(),
                b"binary"
            );

            // A broken "new version" overwrites the binary; restore brings it back.
            fs::write(src.join("Contents/MacOS/PortBay"), b"broken").unwrap();
            copy_dir(&snap, &src).unwrap();
            assert_eq!(
                fs::read(src.join("Contents/MacOS/PortBay")).unwrap(),
                b"binary"
            );
        }
    }
}
