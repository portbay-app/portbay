//! Disk persistence for the registry.
//!
//! Two guarantees this module provides on top of `serde_json`:
//!
//! 1. **Atomic writes.** A crash during `save()` never leaves the registry
//!    half-written. We write to `registry.json.tmp`, fsync it, then rename.
//!    Rename within the same directory is atomic on macOS/Linux APFS/HFS+/ext4.
//!
//! 2. **First-run friendliness.** Loading from the default path when no file
//!    exists yet returns a fresh empty registry instead of an error. That's
//!    the dominant first-run case for PortBay.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::registry::error::{RegistryError, Result};
use crate::registry::{migrate, Registry, SUPPORTED_VERSION};

/// The registry's well-known location inside the platform's data dir.
///
/// macOS: `~/Library/Application Support/PortBay/registry.json`
pub fn default_path() -> Result<PathBuf> {
    let mut p = dirs::data_dir().ok_or(RegistryError::NoDataDir)?;
    p.push("PortBay");
    p.push("registry.json");
    Ok(p)
}

/// Load a registry from the given path. Returns `NotFound` if the file is
/// missing. For first-run convenience, prefer [`load_or_default`].
///
/// A registry written by an older build is migrated up to
/// [`SUPPORTED_VERSION`] (see [`migrate`]) and then rewritten to disk in its
/// new shape, so the upgrade happens exactly once. The pre-migration file is
/// backed up first (`registry.json` → `registry.json.v1.bak`) so a downgrade
/// or recovery is always possible.
pub fn load_from(path: &Path) -> Result<Registry> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(RegistryError::NotFound { path: path.into() });
        }
        Err(e) => return Err(RegistryError::io(path, e)),
    };

    let (reg, migrated_from) = parse_and_migrate(&bytes)?;

    if let Some(from) = migrated_from {
        back_up(path, from)?;
        save_to(&reg, path)?;
    }

    Ok(reg)
}

/// Parse registry bytes and, when the document predates [`SUPPORTED_VERSION`],
/// migrate it in memory. Returns the registry plus `Some(from_version)` when a
/// migration was applied (so the caller can persist it), or `None` when the
/// file was already current. Pure — does no I/O — so the version gating and
/// migration are unit-testable without touching disk.
fn parse_and_migrate(bytes: &[u8]) -> Result<(Registry, Option<u32>)> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    // A missing `version` is treated as v1 (the field predates being written
    // unconditionally); anything present is read as the declared version.
    let found = value.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;

    if found > SUPPORTED_VERSION {
        return Err(RegistryError::UnsupportedVersion {
            found,
            supported: SUPPORTED_VERSION,
        });
    }

    if found < SUPPORTED_VERSION {
        let migrated = migrate(value, found)?;
        let reg: Registry = serde_json::from_value(migrated)?;
        Ok((reg, Some(found)))
    } else {
        let reg: Registry = serde_json::from_value(value)?;
        Ok((reg, None))
    }
}

/// Copy the on-disk registry to a sibling backup before a migration rewrites
/// it, e.g. `registry.json` → `registry.json.v1.bak`. The copy must succeed —
/// we never overwrite a pre-migration file without first preserving it.
fn back_up(path: &Path, from_version: u32) -> Result<()> {
    let mut backup = path.to_path_buf();
    let file_name = backup
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("registry.json"));
    let mut name = file_name;
    name.push(format!(".v{from_version}.bak"));
    backup.set_file_name(name);
    fs::copy(path, &backup).map_err(|e| RegistryError::io(&backup, e))?;
    Ok(())
}

/// Load the registry from `path`, or return a fresh empty registry with the
/// given domain suffix if the file doesn't exist yet.
pub fn load_or_default(path: &Path, domain_suffix: impl Into<String>) -> Result<Registry> {
    match load_from(path) {
        Ok(r) => Ok(r),
        Err(RegistryError::NotFound { .. }) => Ok(Registry::new(domain_suffix)),
        Err(e) => Err(e),
    }
}

/// Atomically write the registry to `path`. Creates parent directories as
/// needed. Survives crash-during-write.
pub fn save_to(reg: &Registry, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| RegistryError::io(parent, e))?;
        }
    }

    let bytes = serde_json::to_vec_pretty(reg)?;

    // Write to a sibling tempfile; the rename is the atomic step.
    let mut tmp_path = path.to_path_buf();
    let file_name = tmp_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("registry.json"));
    let mut tmp_name = file_name;
    tmp_name.push(".tmp");
    tmp_path.set_file_name(tmp_name);

    {
        let mut f: File = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)
            .map_err(|e| RegistryError::io(&tmp_path, e))?;
        f.write_all(&bytes)
            .map_err(|e| RegistryError::io(&tmp_path, e))?;
        f.sync_all().map_err(|e| RegistryError::io(&tmp_path, e))?;
    } // file closed here

    fs::rename(&tmp_path, path).map_err(|e| RegistryError::io(path, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::{Project, ProjectId, ProjectType};
    use std::collections::BTreeMap;

    fn sample_project(id: &str) -> Project {
        Project {
            cors: None,
            sandbox: None,
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
            extra_ports: vec![],
            hostname: format!("{id}.test"),
            https: true,
            services: vec!["caddy".into()],
            env: BTreeMap::new(),
            readiness: None,
            auto_start: false,
            pre_start: vec![],
            post_start: vec![],
            tags: vec![],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
            domain: None,
            tunnel: None,
            deploy: None,
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");

        let mut reg = Registry::new("test");
        reg.add_project(sample_project("marketing-site")).unwrap();
        reg.add_project(sample_project("api-gateway")).unwrap();

        save_to(&reg, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, reg);
    }

    #[test]
    fn load_missing_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.json");
        match load_from(&path) {
            Err(RegistryError::NotFound { .. }) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn load_or_default_creates_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let reg = load_or_default(&path, "test").unwrap();
        assert_eq!(reg.domain_suffix, "test");
        assert!(reg.list_projects().is_empty());
    }

    #[test]
    fn malformed_json_errors_clearly() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        fs::write(&path, b"{ this is not json").unwrap();
        match load_from(&path) {
            Err(RegistryError::Malformed(_)) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        // Forge a registry with a future version.
        let body = serde_json::json!({
            "version": 99,
            "domain_suffix": "test",
            "projects": [],
            "groups": []
        });
        fs::write(&path, body.to_string()).unwrap();
        match load_from(&path) {
            Err(RegistryError::UnsupportedVersion { found: 99, .. }) => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn loading_a_v1_file_migrates_backs_up_and_rewrites_at_current_version() {
        use crate::registry::types::ProjectId;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let v1 = serde_json::json!({
            "version": 1,
            "domain_suffix": "test",
            "projects": [{
                "id": "legacy-shop",
                "name": "Legacy Shop",
                "path": "/tmp/legacy-shop",
                "type": "php",
                "hostname": "legacy-shop.test",
                "https": true,
                "document_root": "public",
                "php_version": "8.3"
            }]
        });
        fs::write(&path, v1.to_string()).unwrap();

        let reg = load_from(&path).unwrap();
        assert_eq!(reg.version, SUPPORTED_VERSION);
        let p = reg.get_project(&ProjectId::new("legacy-shop")).unwrap();
        assert_eq!(p.runtime.as_ref().unwrap().lang, "php");
        assert_eq!(p.runtime.as_ref().unwrap().version, "8.3");

        // The original v1 document is preserved as a backup.
        let backup = tmp.path().join("registry.json.v1.bak");
        assert!(backup.exists(), "v1 backup must be written");
        let backed: serde_json::Value =
            serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
        assert_eq!(backed["version"], 1);

        // The live file is rewritten at the current schema version, so a second
        // load is a no-op (no migration, identical result).
        let on_disk: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk["version"], SUPPORTED_VERSION);
        let reg2 = load_from(&path).unwrap();
        assert_eq!(reg2, reg);
    }

    #[test]
    fn loading_a_current_v2_file_does_not_create_a_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let mut reg = Registry::new("test");
        reg.add_project(sample_project("a")).unwrap();
        save_to(&reg, &path).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, reg);
        assert!(!tmp.path().join("registry.json.v1.bak").exists());
    }

    #[test]
    fn save_is_atomic_no_tmpfile_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let reg = Registry::new("test");
        save_to(&reg, &path).unwrap();

        let tmpfile = tmp.path().join("registry.json.tmp");
        assert!(path.exists(), "registry.json must exist after save");
        assert!(!tmpfile.exists(), "registry.json.tmp must be renamed away");
    }

    #[test]
    fn save_creates_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("registry.json");
        let reg = Registry::new("test");
        save_to(&reg, &path).unwrap();
        assert!(path.exists());
    }
}
