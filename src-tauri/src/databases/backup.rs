//! Per-instance backup + restore.
//!
//! Snapshots a running SQL instance with `mysqldump` / `pg_dumpall` into a
//! timestamped directory under `<app-data>/backups/<instance-id>/<unix-ms>/`,
//! restores by replaying the dump through the engine's client, and prunes
//! snapshots past a retention window. Backup tools are resolved the same way as
//! the daemon/client — a PortBay-managed install wins over Homebrew/system.
//!
//! Scope: the SQL engines (MySQL/MariaDB/PostgreSQL). Redis/Mongo/Memcached use
//! different snapshot mechanics (RDB file swap, `mongodump`) and aren't covered
//! here yet — `supports_backup` gates the UI accordingly.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::databases::{client_binary_resolved, tool_binary};
use crate::registry::{DatabaseEngine, DatabaseInstance};

/// Default retention: snapshots older than this are pruned after each backup.
pub const DEFAULT_KEEP_DAYS: u64 = 7;

/// One backup snapshot on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSnapshot {
    /// Directory name — the unix-millis timestamp it was taken at.
    pub id: String,
    pub created_at: u64,
    pub size_bytes: u64,
}

/// Whether PortBay can back up + restore this engine.
pub fn supports_backup(engine: DatabaseEngine) -> bool {
    matches!(
        engine,
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb | DatabaseEngine::Postgres
    )
}

/// `<app-data>/backups/<instance-id>/`.
pub fn backups_root(app_data: &Path, instance_id: &str) -> PathBuf {
    app_data.join("backups").join(instance_id)
}

const DUMP_FILE: &str = "dump.sql";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A snapshot id is always the unix-millis timestamp — digits only. Validated
/// before it's joined into a path (defends the restore/delete path build).
fn valid_snapshot_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
}

/// Dump a running instance into a fresh timestamped snapshot dir.
pub fn create_backup(
    instance: &DatabaseInstance,
    managed_bin: Option<&Path>,
    app_data: &Path,
) -> Result<BackupSnapshot, String> {
    if !supports_backup(instance.engine) {
        return Err(format!(
            "backups aren't supported for {} yet.",
            instance.engine.label()
        ));
    }
    let ts = now_ms();
    let dir = backups_root(app_data, instance.id.as_str()).join(ts.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let out = dir.join(DUMP_FILE);
    let port = instance.port.to_string();

    let result = match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let tool = tool_binary(instance.engine, &["mysqldump"], managed_bin)
                .ok_or_else(|| "mysqldump not found alongside the engine.".to_string())?;
            run_to_file(
                &tool,
                &[
                    "-h",
                    "127.0.0.1",
                    "-P",
                    &port,
                    "-u",
                    "root",
                    "--all-databases",
                    "--no-tablespaces",
                ],
                &out,
            )
        }
        DatabaseEngine::Postgres => {
            let tool = tool_binary(instance.engine, &["pg_dumpall"], managed_bin)
                .ok_or_else(|| "pg_dumpall not found alongside the engine.".to_string())?;
            run_to_file(
                &tool,
                &["-h", "127.0.0.1", "-p", &port, "-U", "postgres"],
                &out,
            )
        }
        _ => unreachable!("guarded by supports_backup"),
    };

    if let Err(e) = result {
        // Don't leave a half-written snapshot behind.
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }

    let size_bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    Ok(BackupSnapshot {
        id: ts.to_string(),
        created_at: ts,
        size_bytes,
    })
}

/// List an instance's snapshots, newest first.
pub fn list_backups(app_data: &Path, instance_id: &str) -> Vec<BackupSnapshot> {
    let root = backups_root(app_data, instance_id);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_snapshot_id(&id) {
            continue;
        }
        let size_bytes = std::fs::metadata(entry.path().join(DUMP_FILE))
            .map(|m| m.len())
            .unwrap_or(0);
        out.push(BackupSnapshot {
            created_at: id.parse().unwrap_or(0),
            id,
            size_bytes,
        });
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    out
}

/// Restore a snapshot by replaying its dump through the engine client.
pub fn restore_backup(
    instance: &DatabaseInstance,
    managed_bin: Option<&Path>,
    app_data: &Path,
    snapshot_id: &str,
) -> Result<(), String> {
    if !supports_backup(instance.engine) {
        return Err(format!(
            "restore isn't supported for {} yet.",
            instance.engine.label()
        ));
    }
    if !valid_snapshot_id(snapshot_id) {
        return Err("invalid snapshot id.".to_string());
    }
    let dump = backups_root(app_data, instance.id.as_str())
        .join(snapshot_id)
        .join(DUMP_FILE);
    if !dump.is_file() {
        return Err("snapshot not found.".to_string());
    }
    let client = client_binary_resolved(instance.engine, managed_bin)
        .ok_or_else(|| format!("no CLI client for {} found.", instance.engine.label()))?;
    let port = instance.port.to_string();
    match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => run_from_file(
            &client,
            &["-h", "127.0.0.1", "-P", &port, "-u", "root"],
            &dump,
        ),
        DatabaseEngine::Postgres => run_from_file(
            &client,
            &[
                "-h",
                "127.0.0.1",
                "-p",
                &port,
                "-U",
                "postgres",
                "-d",
                "postgres",
            ],
            &dump,
        ),
        _ => unreachable!("guarded by supports_backup"),
    }
}

/// Delete a single snapshot. Validates the id and stays inside the backups root.
pub fn delete_backup(app_data: &Path, instance_id: &str, snapshot_id: &str) -> Result<(), String> {
    if !valid_snapshot_id(snapshot_id) {
        return Err("invalid snapshot id.".to_string());
    }
    let dir = backups_root(app_data, instance_id).join(snapshot_id);
    if dir.starts_with(app_data.join("backups")) && dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("delete {}: {e}", dir.display()))?;
    }
    Ok(())
}

/// Prune snapshots older than `keep_days`. Returns how many were removed.
pub fn prune(app_data: &Path, instance_id: &str, keep_days: u64) -> u64 {
    let cutoff = now_ms().saturating_sub(keep_days.saturating_mul(86_400_000));
    let mut removed = 0;
    for snap in list_backups(app_data, instance_id) {
        if snap.created_at < cutoff && delete_backup(app_data, instance_id, &snap.id).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Run `bin args…` with stdout redirected into `out`, with a hard timeout.
fn run_to_file(bin: &Path, args: &[&str], out: &Path) -> Result<(), String> {
    let file = std::fs::File::create(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
    wait_checked(child, Duration::from_secs(600))
}

/// Run `bin args…` with stdin fed from `input`, with a hard timeout.
fn run_from_file(bin: &Path, args: &[&str], input: &Path) -> Result<(), String> {
    let file = std::fs::File::open(input).map_err(|e| format!("open {}: {e}", input.display()))?;
    let child = Command::new(bin)
        .args(args)
        .stdin(Stdio::from(file))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
    wait_checked(child, Duration::from_secs(600))
}

/// Wait for a child with a timeout; on failure return its stderr tail.
fn wait_checked(child: std::process::Child, timeout: Duration) -> Result<(), String> {
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let output = child.wait_with_output();
        let _ = tx.send(output);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) if output.status.success() => Ok(()),
        Ok(Ok(output)) => {
            let err = String::from_utf8_lossy(&output.stderr);
            let err = err.trim();
            Err(if err.is_empty() {
                format!("exit {}", output.status)
            } else {
                err.chars().take(800).collect()
            })
        }
        Ok(Err(e)) => Err(format!("wait failed: {e}")),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            #[cfg(not(unix))]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .status();
            }
            Err("timed out.".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("wait thread disconnected.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_backup_is_sql_only() {
        assert!(supports_backup(DatabaseEngine::Mysql));
        assert!(supports_backup(DatabaseEngine::Postgres));
        assert!(!supports_backup(DatabaseEngine::Redis));
        assert!(!supports_backup(DatabaseEngine::Mongo));
    }

    #[test]
    fn backups_root_is_namespaced() {
        assert_eq!(
            backups_root(Path::new("/tmp/pb"), "myapp"),
            PathBuf::from("/tmp/pb/backups/myapp")
        );
    }

    #[test]
    fn valid_snapshot_id_is_digits_only() {
        assert!(valid_snapshot_id("1716864735000"));
        assert!(!valid_snapshot_id(""));
        assert!(!valid_snapshot_id("../etc"));
        assert!(!valid_snapshot_id("2024-01"));
    }

    #[test]
    fn list_backups_sorts_newest_first_and_skips_junk() {
        let tmp = tempfile::tempdir().unwrap();
        let root = backups_root(tmp.path(), "x");
        for ts in ["1000", "3000", "2000"] {
            std::fs::create_dir_all(root.join(ts)).unwrap();
            std::fs::write(root.join(ts).join(DUMP_FILE), b"-- dump\n").unwrap();
        }
        std::fs::create_dir_all(root.join("not-a-timestamp")).unwrap(); // skipped
        let snaps = list_backups(tmp.path(), "x");
        let ids: Vec<&str> = snaps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["3000", "2000", "1000"]);
        assert!(snaps.iter().all(|s| s.size_bytes > 0));
    }

    #[test]
    fn prune_removes_only_old_snapshots() {
        let tmp = tempfile::tempdir().unwrap();
        let root = backups_root(tmp.path(), "x");
        let recent = now_ms();
        let ancient = recent.saturating_sub(10 * 86_400_000); // 10 days old
        for ts in [recent.to_string(), ancient.to_string()] {
            std::fs::create_dir_all(root.join(&ts)).unwrap();
            std::fs::write(root.join(&ts).join(DUMP_FILE), b"x").unwrap();
        }
        let removed = prune(tmp.path(), "x", DEFAULT_KEEP_DAYS);
        assert_eq!(removed, 1);
        let left = list_backups(tmp.path(), "x");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].created_at, recent);
    }

    #[test]
    fn delete_backup_rejects_bad_ids() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(delete_backup(tmp.path(), "x", "../escape").is_err());
        assert!(delete_backup(tmp.path(), "x", "123").is_ok()); // no-op when absent
    }
}
