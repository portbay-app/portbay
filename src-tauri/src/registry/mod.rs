//! Project registry — the single source of truth for what PortBay manages.
//!
//! The registry is a plain JSON file on disk (one user, one process, no
//! concurrent writers). It survives daemon restarts because it *is* the
//! state. Runtime state — which projects are actually running right now —
//! lives in Process Compose and is queried fresh on every status request.
//! See `claudedocs/ASSESSMENT_AND_PLAN.md` §7 for the shape, and
//! `claudedocs/spike-process-compose.md` for why we don't trust cached state.

use serde::{Deserialize, Serialize};

pub mod error;
pub mod store;
pub mod types;
pub mod workspace;

pub use error::{RegistryError, Result};
pub use types::{
    DatabaseEngine, DatabaseInstance, DatabaseInstanceId, DnsmasqSettings, FpmTuning, Group,
    ManualRuntime, PhpVersionConfig, Project, ProjectId, ProjectType, Readiness, Runtime,
    RuntimeSettings, Workspace, WorkspaceTool,
};

/// The registry-file schema version this build reads and writes.
///
/// On load, a higher version is rejected with
/// [`RegistryError::UnsupportedVersion`]; a lower version is upgraded by
/// [`migrate`] and the file is rewritten in the new shape (see
/// `store::load_from`).
///
/// ## Version history
/// - **v1** — original shape (`ASSESSMENT_AND_PLAN.md` §7.1).
/// - **v2** — adds [`Project::runtime`]; migrated from the legacy
///   `php_version` field.
pub const SUPPORTED_VERSION: u32 = 2;

/// Upgrade a raw registry JSON document from `from_version` up to
/// [`SUPPORTED_VERSION`], applying each version step in order. Every step
/// fills new fields from the old shape (or sensible defaults) and never
/// drops data, so the result deserializes cleanly into [`Registry`].
///
/// Callers reject a `from_version` greater than [`SUPPORTED_VERSION`] before
/// calling this. A version with no registered step is a bug and surfaces as
/// [`RegistryError::Migration`].
pub fn migrate(value: serde_json::Value, from_version: u32) -> Result<serde_json::Value> {
    let mut value = value;
    let mut version = from_version;
    while version < SUPPORTED_VERSION {
        value = match version {
            1 => migrate_v1_to_v2(value)?,
            other => {
                return Err(RegistryError::Migration {
                    from: other,
                    reason: format!("no migration step from v{other}"),
                });
            }
        };
        version += 1;
    }
    Ok(value)
}

/// v1 → v2: introduce [`Project::runtime`]. v1 only ever pinned a PHP
/// version (`php_version`), so projects carrying one gain the structured
/// equivalent `{ "lang": "php", "version": … }`. Every other field is
/// carried over verbatim, and `php_version` is intentionally preserved —
/// current consumers still read it.
fn migrate_v1_to_v2(value: serde_json::Value) -> Result<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(RegistryError::Migration {
                from: 1,
                reason: "registry root is not a JSON object".into(),
            });
        }
    };

    obj.insert("version".into(), serde_json::json!(2));

    if let Some(serde_json::Value::Array(projects)) = obj.get_mut("projects") {
        for project in projects.iter_mut() {
            let serde_json::Value::Object(p) = project else {
                continue;
            };
            let has_runtime = p.get("runtime").is_some_and(|r| !r.is_null());
            if has_runtime {
                continue;
            }
            if let Some(ver) = p.get("php_version").and_then(|v| v.as_str()) {
                let ver = ver.to_owned();
                p.insert(
                    "runtime".into(),
                    serde_json::json!({ "lang": "php", "version": ver }),
                );
            }
        }
    }

    Ok(serde_json::Value::Object(obj))
}

/// The top-level registry document.
///
/// JSON shape matches `ASSESSMENT_AND_PLAN.md` §7.1.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub domain_suffix: String,

    #[serde(default)]
    pub projects: Vec<Project>,

    #[serde(default)]
    pub groups: Vec<Group>,

    /// Database instances PortBay provisions and supervises. `#[serde(default)]`
    /// keeps pre-databases registry files loading cleanly (no version bump).
    #[serde(default)]
    pub databases: Vec<DatabaseInstance>,

    /// Tunable dnsmasq daemon settings. `#[serde(default)]` keeps
    /// pre-DNS-settings registry files loading with sane defaults.
    #[serde(default)]
    pub dnsmasq: DnsmasqSettings,

    /// Manually-added runtime installs + default version per language.
    /// `#[serde(default)]` keeps pre-runtimes registry files loading.
    #[serde(default)]
    pub runtimes: RuntimeSettings,
}

impl Registry {
    /// A fresh empty registry pinned to the current schema version.
    pub fn new(domain_suffix: impl Into<String>) -> Self {
        Self {
            version: SUPPORTED_VERSION,
            domain_suffix: domain_suffix.into(),
            projects: Vec::new(),
            groups: Vec::new(),
            databases: Vec::new(),
            dnsmasq: DnsmasqSettings::default(),
            runtimes: RuntimeSettings::default(),
        }
    }

    // ---- Project CRUD ------------------------------------------------------

    pub fn list_projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn get_project(&self, id: &ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| &p.id == id)
    }

    pub fn get_project_mut(&mut self, id: &ProjectId) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| &p.id == id)
    }

    /// Insert a new project. Errors if the id is already taken.
    pub fn add_project(&mut self, project: Project) -> Result<()> {
        if self.get_project(&project.id).is_some() {
            return Err(RegistryError::DuplicateProjectId(project.id));
        }
        self.projects.push(project);
        Ok(())
    }

    /// Remove a project by id, returning the removed entry. Errors if missing.
    pub fn remove_project(&mut self, id: &ProjectId) -> Result<Project> {
        let idx = self
            .projects
            .iter()
            .position(|p| &p.id == id)
            .ok_or_else(|| RegistryError::ProjectNotFound(id.clone()))?;
        let removed = self.projects.remove(idx);
        // Also strip the id from any group that referenced it.
        for g in &mut self.groups {
            g.projects.retain(|pid| pid != id);
        }
        Ok(removed)
    }

    /// Replace an existing project (matched by id) with the given value.
    /// Errors if no project with that id exists.
    pub fn update_project(&mut self, project: Project) -> Result<()> {
        let slot = self
            .projects
            .iter_mut()
            .find(|p| p.id == project.id)
            .ok_or_else(|| RegistryError::ProjectNotFound(project.id.clone()))?;
        *slot = project;
        Ok(())
    }

    // ---- Group CRUD --------------------------------------------------------

    pub fn list_groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn add_group(&mut self, group: Group) -> Result<()> {
        if self.groups.iter().any(|g| g.id == group.id) {
            return Err(RegistryError::DuplicateGroupId(group.id));
        }
        self.groups.push(group);
        Ok(())
    }

    pub fn remove_group(&mut self, id: &str) -> Result<Group> {
        let idx = self
            .groups
            .iter()
            .position(|g| g.id == id)
            .ok_or_else(|| RegistryError::GroupNotFound(id.to_owned()))?;
        Ok(self.groups.remove(idx))
    }

    /// Replace an existing group (matched by id) with the given value.
    /// Errors if no group with that id exists.
    pub fn update_group(&mut self, group: Group) -> Result<()> {
        let slot = self
            .groups
            .iter_mut()
            .find(|g| g.id == group.id)
            .ok_or_else(|| RegistryError::GroupNotFound(group.id.clone()))?;
        *slot = group;
        Ok(())
    }

    pub fn get_group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    // ---- Database instance CRUD -------------------------------------------

    pub fn list_databases(&self) -> &[DatabaseInstance] {
        &self.databases
    }

    pub fn get_database(&self, id: &DatabaseInstanceId) -> Option<&DatabaseInstance> {
        self.databases.iter().find(|d| &d.id == id)
    }

    pub fn get_database_mut(&mut self, id: &DatabaseInstanceId) -> Option<&mut DatabaseInstance> {
        self.databases.iter_mut().find(|d| &d.id == id)
    }

    /// Insert a new database instance. Errors if the id is already taken.
    pub fn add_database(&mut self, instance: DatabaseInstance) -> Result<()> {
        if self.get_database(&instance.id).is_some() {
            return Err(RegistryError::DuplicateDatabaseId(instance.id));
        }
        self.databases.push(instance);
        Ok(())
    }

    /// Remove a database instance by id, returning the removed entry.
    pub fn remove_database(&mut self, id: &DatabaseInstanceId) -> Result<DatabaseInstance> {
        let idx = self
            .databases
            .iter()
            .position(|d| &d.id == id)
            .ok_or_else(|| RegistryError::DatabaseNotFound(id.clone()))?;
        Ok(self.databases.remove(idx))
    }

    /// Replace an existing database instance (matched by id).
    pub fn update_database(&mut self, instance: DatabaseInstance) -> Result<()> {
        let slot = self
            .databases
            .iter_mut()
            .find(|d| d.id == instance.id)
            .ok_or_else(|| RegistryError::DatabaseNotFound(instance.id.clone()))?;
        *slot = instance;
        Ok(())
    }

    /// Whether a port is already claimed by another database instance.
    /// Used by the create flow's port allocator.
    pub fn database_port_in_use(&self, port: u16, except: Option<&DatabaseInstanceId>) -> bool {
        self.databases
            .iter()
            .any(|d| d.port == port && except != Some(&d.id))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn sample_project(id: &str) -> Project {
        Project {
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
            tags: vec![],
            document_root: None,
            php_version: None,
            runtime: None,
            workspace: None,
        }
    }

    #[test]
    fn new_registry_is_pinned_to_current_version() {
        let r = Registry::new("test");
        assert_eq!(r.version, SUPPORTED_VERSION);
        assert!(r.projects.is_empty());
        assert!(r.groups.is_empty());
    }

    #[test]
    fn add_project_succeeds_then_listing_shows_it() {
        let mut r = Registry::new("test");
        r.add_project(sample_project("marketing-site")).unwrap();
        assert_eq!(r.list_projects().len(), 1);
        assert_eq!(r.list_projects()[0].name, "marketing-site");
    }

    #[test]
    fn add_duplicate_project_errors() {
        let mut r = Registry::new("test");
        r.add_project(sample_project("marketing-site")).unwrap();
        match r.add_project(sample_project("marketing-site")) {
            Err(RegistryError::DuplicateProjectId(id)) => {
                assert_eq!(id.as_str(), "marketing-site");
            }
            other => panic!("expected DuplicateProjectId, got {other:?}"),
        }
    }

    #[test]
    fn remove_returns_the_removed_project() {
        let mut r = Registry::new("test");
        r.add_project(sample_project("a")).unwrap();
        r.add_project(sample_project("b")).unwrap();
        let removed = r.remove_project(&ProjectId::new("a")).unwrap();
        assert_eq!(removed.id.as_str(), "a");
        assert_eq!(r.list_projects().len(), 1);
        assert_eq!(r.list_projects()[0].id.as_str(), "b");
    }

    #[test]
    fn remove_missing_project_errors() {
        let mut r = Registry::new("test");
        match r.remove_project(&ProjectId::new("nope")) {
            Err(RegistryError::ProjectNotFound(_)) => {}
            other => panic!("expected ProjectNotFound, got {other:?}"),
        }
    }

    #[test]
    fn remove_also_strips_project_from_groups() {
        let mut r = Registry::new("test");
        r.add_project(sample_project("a")).unwrap();
        r.add_project(sample_project("b")).unwrap();
        r.add_group(Group {
            id: "suite".into(),
            name: "Suite".into(),
            projects: vec![ProjectId::new("a"), ProjectId::new("b")],
        })
        .unwrap();
        r.remove_project(&ProjectId::new("a")).unwrap();
        let g = r.list_groups().first().unwrap();
        assert_eq!(g.projects.len(), 1);
        assert_eq!(g.projects[0].as_str(), "b");
    }

    #[test]
    fn update_existing_project_replaces_it() {
        let mut r = Registry::new("test");
        r.add_project(sample_project("a")).unwrap();
        let mut updated = sample_project("a");
        updated.name = "Renamed".into();
        r.update_project(updated).unwrap();
        assert_eq!(r.get_project(&ProjectId::new("a")).unwrap().name, "Renamed");
    }

    #[test]
    fn update_missing_project_errors() {
        let mut r = Registry::new("test");
        match r.update_project(sample_project("absent")) {
            Err(RegistryError::ProjectNotFound(_)) => {}
            other => panic!("expected ProjectNotFound, got {other:?}"),
        }
    }

    #[test]
    fn add_duplicate_group_errors() {
        let mut r = Registry::new("test");
        let g = Group {
            id: "x".into(),
            name: "X".into(),
            projects: vec![],
        };
        r.add_group(g.clone()).unwrap();
        match r.add_group(g) {
            Err(RegistryError::DuplicateGroupId(_)) => {}
            other => panic!("expected DuplicateGroupId, got {other:?}"),
        }
    }

    #[test]
    fn registry_without_dnsmasq_field_loads_with_defaults() {
        // A pre-DNS-settings registry blob must still deserialise, with the
        // dnsmasq settings falling back to their defaults.
        let json = r#"{ "version": 1, "domain_suffix": "test", "projects": [] }"#;
        let reg: Registry = serde_json::from_str(json).unwrap();
        assert_eq!(reg.dnsmasq, DnsmasqSettings::default());
        assert_eq!(reg.dnsmasq.cache_size, 150);
    }

    // ---- Migration --------------------------------------------------------

    /// A representative v1 registry blob: one PHP project (with `php_version`)
    /// and one Node project (without), plus a group and the dnsmasq block —
    /// the shape a real user's file would have before the v2 bump.
    fn v1_registry_json() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "domain_suffix": "test",
            "projects": [
                {
                    "id": "legacy-shop",
                    "name": "Legacy Shop",
                    "path": "/tmp/legacy-shop",
                    "type": "php",
                    "hostname": "legacy-shop.test",
                    "https": true,
                    "document_root": "public",
                    "php_version": "8.3"
                },
                {
                    "id": "marketing-site",
                    "name": "Marketing Site",
                    "path": "/tmp/marketing-site",
                    "type": "next",
                    "start_command": "pnpm dev",
                    "port": 3010,
                    "hostname": "marketing-site.test",
                    "https": true
                }
            ],
            "groups": [
                { "id": "suite", "name": "Suite", "projects": ["legacy-shop", "marketing-site"] }
            ]
        })
    }

    #[test]
    fn migrate_v1_to_v2_derives_runtime_from_php_version() {
        let migrated = migrate(v1_registry_json(), 1).unwrap();
        let reg: Registry = serde_json::from_value(migrated).unwrap();

        assert_eq!(reg.version, 2);

        // The PHP project gains a structured runtime derived from php_version,
        // and the legacy field is preserved (no loss).
        let php = reg.get_project(&ProjectId::new("legacy-shop")).unwrap();
        assert_eq!(
            php.runtime,
            Some(Runtime {
                lang: "php".into(),
                version: "8.3".into()
            })
        );
        assert_eq!(php.php_version.as_deref(), Some("8.3"));

        // The non-PHP project has no runtime to derive — left as None.
        let next = reg.get_project(&ProjectId::new("marketing-site")).unwrap();
        assert!(next.runtime.is_none());

        // Nothing else was dropped.
        assert_eq!(reg.domain_suffix, "test");
        assert_eq!(reg.list_projects().len(), 2);
        assert_eq!(reg.list_groups().len(), 1);
    }

    #[test]
    fn migrate_is_idempotent_when_runtime_already_present() {
        // Running the v1→v2 step over a doc that already has a runtime must
        // not clobber it.
        let mut v1 = v1_registry_json();
        v1["projects"][0]["runtime"] = serde_json::json!({ "lang": "php", "version": "7.4" });
        let migrated = migrate(v1, 1).unwrap();
        let reg: Registry = serde_json::from_value(migrated).unwrap();
        let php = reg.get_project(&ProjectId::new("legacy-shop")).unwrap();
        assert_eq!(php.runtime.as_ref().unwrap().version, "7.4");
    }

    #[test]
    fn migrate_from_current_version_is_a_noop() {
        // Already at SUPPORTED_VERSION: migrate returns the value untouched.
        let mut doc = v1_registry_json();
        doc["version"] = serde_json::json!(SUPPORTED_VERSION);
        let out = migrate(doc.clone(), SUPPORTED_VERSION).unwrap();
        assert_eq!(out, doc);
    }
}
