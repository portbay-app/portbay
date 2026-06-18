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
        let reg = registry_from_value_lenient(migrated)?;
        Ok((reg, Some(found)))
    } else {
        let reg = registry_from_value_lenient(value)?;
        Ok((reg, None))
    }
}

/// Deserialize a [`Registry`] from a JSON value, tolerating individual project
/// entries this build can't parse.
///
/// A project written by a newer version (an unknown `type`, a new nested enum
/// value, …) would otherwise fail the *entire* registry load — blanking every
/// project and, in `setup()`, taking the whole app down on boot (the panic
/// surfaces inside `did_finish_launching`, which cannot unwind, so it aborts).
/// Instead we peel each element out of the `projects` array, keep the ones that
/// deserialize, and stash the rest verbatim in [`Registry::unparsed_projects`]
/// so [`save_to`] re-emits them untouched. Anything *other* than a bad project
/// entry (corrupt top-level shape, a malformed `dnsmasq` block, …) still errors:
/// leniency is scoped to the one array where forward-incompatible drift across
/// builds is both expected and individually recoverable.
fn registry_from_value_lenient(mut value: serde_json::Value) -> Result<Registry> {
    use crate::registry::types::Project;

    let mut quarantined: Vec<String> = Vec::new();
    if let Some(serde_json::Value::Array(projects)) = value.get_mut("projects") {
        let original = std::mem::take(projects);
        let mut kept = Vec::with_capacity(original.len());
        for entry in original {
            match serde_json::from_value::<Project>(entry.clone()) {
                Ok(_) => kept.push(entry),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "registry project entry unreadable by this build; preserving it \
                         out-of-band so the next save can't drop it"
                    );
                    // Compact JSON keeps the field `Eq` (`serde_json::Value` is
                    // not) and round-trips losslessly when re-spliced on save.
                    if let Ok(raw) = serde_json::to_string(&entry) {
                        quarantined.push(raw);
                    }
                }
            }
        }
        *projects = kept;
    }

    let mut reg: Registry = serde_json::from_value(value)?;
    reg.unparsed_projects = quarantined;
    Ok(reg)
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
    // `fs::copy` carries over the source's mode, which may predate the 0600
    // hardening — backups hold the same key paths/hosts as the live registry.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o600))
            .map_err(|e| RegistryError::io(&backup, e))?;
    }
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

/// Serialize a registry for disk, re-splicing any [`Registry::unparsed_projects`]
/// (entries this build couldn't parse on load) back into the `projects` array so
/// they survive the save instead of being silently dropped. The common path —
/// nothing quarantined — is a plain pretty-print with no extra work.
fn serialize_registry(reg: &Registry) -> Result<Vec<u8>> {
    if reg.unparsed_projects.is_empty() {
        return Ok(serde_json::to_vec_pretty(reg)?);
    }
    let mut value = serde_json::to_value(reg)?;
    let preserved = reg
        .unparsed_projects
        .iter()
        .filter_map(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    match value.get_mut("projects") {
        Some(serde_json::Value::Array(projects)) => projects.extend(preserved),
        _ => value["projects"] = serde_json::Value::Array(preserved.collect()),
    }
    Ok(serde_json::to_vec_pretty(&value)?)
}

/// Atomically write the registry to `path`. Creates parent directories as
/// needed. Survives crash-during-write.
pub fn save_to(reg: &Registry, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| RegistryError::io(parent, e))?;
        }
    }

    let bytes = serialize_registry(reg)?;

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
        // Owner-only: the registry carries key paths, hosts/users, and proxy
        // config — not for other local users.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| RegistryError::io(&tmp_path, e))?;
        }
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
            framework: None,
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

    /// Pins the P2-2 fix from the 2026-06-10 SSH security assessment: the
    /// registry (key paths, hosts/users, proxy config) is owner-only on disk.
    #[cfg(unix)]
    #[test]
    fn save_writes_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        save_to(&Registry::new("test"), &path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
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

    /// A project whose `type` this build doesn't know (the real-world case: a
    /// release reading a registry a newer dev build wrote, e.g. `astro` before
    /// it shipped) must NOT fail the whole load. The unknown entry is quarantined
    /// and every other project still loads — no boot abort, no blanked registry.
    #[test]
    fn unknown_project_type_is_quarantined_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");

        let mut reg = Registry::new("test");
        reg.add_project(sample_project("good-one")).unwrap();
        reg.add_project(sample_project("from-the-future")).unwrap();

        // Forge a `type` no current build knows. Serialise the real registry,
        // then tamper one entry so the rest of the document stays valid.
        let mut value = serde_json::to_value(&reg).unwrap();
        value["projects"][1]["type"] = serde_json::json!("quasar_9000");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let loaded = load_from(&path).expect("unknown type must not fail the load");
        assert_eq!(
            loaded.list_projects().len(),
            1,
            "the readable project survives"
        );
        assert_eq!(loaded.list_projects()[0].id.as_str(), "good-one");
        assert_eq!(
            loaded.unparsed_projects.len(),
            1,
            "the unknown one is quarantined"
        );
        assert!(loaded.unparsed_projects[0].contains("quasar_9000"));
    }

    /// The quarantined entry round-trips: saving a registry that holds an
    /// unparseable project re-emits it verbatim, so an older build can't silently
    /// drop a project a newer build created.
    #[test]
    fn quarantined_project_survives_a_save() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");

        let mut reg = Registry::new("test");
        reg.add_project(sample_project("good-one")).unwrap();
        reg.add_project(sample_project("from-the-future")).unwrap();
        let mut value = serde_json::to_value(&reg).unwrap();
        value["projects"][1]["type"] = serde_json::json!("quasar_9000");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let loaded = load_from(&path).unwrap();
        let out = tmp.path().join("registry-out.json");
        save_to(&loaded, &out).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
        let types: Vec<&str> = written["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types.len(),
            2,
            "both the kept and quarantined entries are written"
        );
        assert!(
            types.contains(&"next"),
            "the readable project is re-emitted"
        );
        assert!(
            types.contains(&"quasar_9000"),
            "the unknown project is preserved, not dropped: {types:?}"
        );
    }

    /// The common path is untouched: a registry with nothing quarantined writes
    /// no `unparsed_projects` artefact and reloads byte-for-byte equal.
    #[test]
    fn clean_registry_roundtrips_without_quarantine_noise() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("registry.json");
        let mut reg = Registry::new("test");
        reg.add_project(sample_project("only-one")).unwrap();
        save_to(&reg, &path).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("unparsed_projects"),
            "skip field never hits disk"
        );
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, reg);
        assert!(loaded.unparsed_projects.is_empty());
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
