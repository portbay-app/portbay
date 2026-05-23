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

pub use error::{RegistryError, Result};
pub use types::{Group, Project, ProjectId, ProjectType, Readiness};

/// The registry-file schema version this build can read and write.
///
/// On load, a version higher than this is rejected with
/// [`RegistryError::UnsupportedVersion`]. Lower versions go through
/// `migrate()` (currently a no-op — we'll fill it in when v2 ships).
pub const SUPPORTED_VERSION: u32 = 1;

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
}

impl Registry {
    /// A fresh empty registry pinned to the current schema version.
    pub fn new(domain_suffix: impl Into<String>) -> Self {
        Self {
            version: SUPPORTED_VERSION,
            domain_suffix: domain_suffix.into(),
            projects: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// Reject registries whose `version` is newer than this build supports.
    pub(crate) fn validate_version(&self) -> Result<()> {
        if self.version > SUPPORTED_VERSION {
            return Err(RegistryError::UnsupportedVersion {
                found: self.version,
                supported: SUPPORTED_VERSION,
            });
        }
        Ok(())
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
    fn version_above_supported_is_rejected() {
        let mut r = Registry::new("test");
        r.version = SUPPORTED_VERSION + 1;
        match r.validate_version() {
            Err(RegistryError::UnsupportedVersion { found, supported }) => {
                assert_eq!(found, SUPPORTED_VERSION + 1);
                assert_eq!(supported, SUPPORTED_VERSION);
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }
}
