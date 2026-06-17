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
    AcmeConfig, AcmeDnsProvider, AcmeEnvironment, AcmeIssuer, AcmeKeyType, CorsConfig,
    CustomTunnelConfig, DatabaseEngine, DatabaseInstance, DatabaseInstanceId, DnsmasqSettings,
    DomainConfig, FpmTuning, Framework, Group, ManagedDatabaseEngine, ManagedRuntime,
    ManualRuntime, MobileRunConfig, PhpVersionConfig, Project, ProjectDeploy, ProjectId,
    ProjectType, Readiness, ResolverMode, Runtime, RuntimeSettings, SandboxConfig,
    SandboxNetworkPolicy, SshAuthKind, SshConnection, SshConnectionId, SshConnectionMeta,
    SshForwardKind, SshIdentity, SshIdentityId, SshProxyConfig, SshProxyKind, SshTunnelConnection,
    SshTunnelId, SslMode, WebServer, Workspace, WorkspaceTool,
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
/// - **v3** — extracts [`SshConnection`] from `ssh_tunnels`: each tunnel's host
///   and auth fields move to a deduplicated connection, and the tunnel keeps a
///   `connection_id` reference (the spine for SFTP / deploy / shell).
pub const SUPPORTED_VERSION: u32 = 3;

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
            2 => migrate_v2_to_v3(value)?,
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

/// v2 → v3: extract [`SshConnection`]s from the self-contained `ssh_tunnels`.
///
/// Each tunnel carried its own host + auth (`ssh_host`, `ssh_port`, `ssh_user`,
/// `auth_kind`, `key_path`, `proxy_jump`). We pull those into a deduplicated
/// connection list (one connection per distinct host+auth tuple) and rewrite
/// each tunnel to keep just its forward coordinates plus a `connectionId`.
///
/// The connection id reuses the **first** referencing tunnel's id on purpose:
/// password auth is stored in the OS keychain keyed by id, so reusing the tunnel
/// id means migrated passwords keep resolving with zero keychain migration.
fn migrate_v2_to_v3(value: serde_json::Value) -> Result<serde_json::Value> {
    let mut obj = match value {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(RegistryError::Migration {
                from: 2,
                reason: "registry root is not a JSON object".into(),
            });
        }
    };

    obj.insert("version".into(), serde_json::json!(3));

    // Preserve any connections already present so re-running the step never
    // clobbers them (idempotency), and seed the dedup map from them so extracted
    // tunnels can attach to an existing matching connection.
    let mut connections: Vec<serde_json::Value> = match obj.remove("ssh_connections") {
        Some(serde_json::Value::Array(existing)) => existing,
        _ => Vec::new(),
    };
    // Dedup key (host, port, user, auth, key, proxy) → connection id.
    let mut by_key: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let conn_str = |c: &serde_json::Value, k: &str| {
        c.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    for conn in &connections {
        if let Some(id) = conn.get("id").and_then(|v| v.as_str()) {
            let key = format!(
                "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
                conn_str(conn, "sshHost"),
                conn.get("sshPort").and_then(|v| v.as_u64()).unwrap_or(22),
                conn_str(conn, "sshUser"),
                conn.get("authKind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("key"),
                conn_str(conn, "keyPath"),
                conn_str(conn, "proxyJump"),
            );
            by_key.insert(key, id.to_string());
        }
    }

    if let Some(serde_json::Value::Array(tunnels)) = obj.get_mut("ssh_tunnels") {
        for tunnel in tunnels.iter_mut() {
            let serde_json::Value::Object(t) = tunnel else {
                continue;
            };

            // A tunnel already in v3 shape (has connectionId) is left alone — keeps
            // the step idempotent if it ever runs twice.
            if t.contains_key("connectionId") {
                continue;
            }

            let take_str = |t: &mut serde_json::Map<String, serde_json::Value>, k: &str| {
                t.remove(k).and_then(|v| v.as_str().map(ToOwned::to_owned))
            };
            let ssh_host = take_str(t, "sshHost").unwrap_or_default();
            let ssh_port = t.remove("sshPort").and_then(|v| v.as_u64()).unwrap_or(22);
            let ssh_user = take_str(t, "sshUser").unwrap_or_default();
            let auth_kind = take_str(t, "authKind").unwrap_or_else(|| "key".into());
            let key_path = take_str(t, "keyPath");
            let proxy_jump = take_str(t, "proxyJump");

            let dedup_key = format!(
                "{ssh_host}\u{1}{ssh_port}\u{1}{ssh_user}\u{1}{auth_kind}\u{1}{}\u{1}{}",
                key_path.as_deref().unwrap_or(""),
                proxy_jump.as_deref().unwrap_or(""),
            );

            let connection_id = if let Some(existing) = by_key.get(&dedup_key) {
                existing.clone()
            } else {
                // Reuse this tunnel's id as the new connection id (keychain compat).
                let id = t
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("ssh-conn-{}", connections.len() + 1));
                let display = if ssh_user.is_empty() {
                    ssh_host.clone()
                } else {
                    format!("{ssh_user}@{ssh_host}")
                };
                let mut conn = serde_json::Map::new();
                conn.insert("id".into(), serde_json::json!(id));
                conn.insert("name".into(), serde_json::json!(display));
                conn.insert("sshHost".into(), serde_json::json!(ssh_host));
                conn.insert("sshPort".into(), serde_json::json!(ssh_port));
                conn.insert("sshUser".into(), serde_json::json!(ssh_user));
                conn.insert("authKind".into(), serde_json::json!(auth_kind));
                if let Some(kp) = &key_path {
                    conn.insert("keyPath".into(), serde_json::json!(kp));
                }
                if let Some(pj) = &proxy_jump {
                    conn.insert("proxyJump".into(), serde_json::json!(pj));
                }
                connections.push(serde_json::Value::Object(conn));
                by_key.insert(dedup_key, id.clone());
                id
            };

            t.insert("connectionId".into(), serde_json::json!(connection_id));
        }
    }

    obj.insert(
        "ssh_connections".into(),
        serde_json::Value::Array(connections),
    );

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

    /// PortBay-managed database engines fetched on demand (the on-demand
    /// counterpart to bundling). Preferred over Homebrew/system installs when
    /// resolving an engine's binaries. `#[serde(default)]` keeps older registry
    /// files loading.
    #[serde(default)]
    pub managed_database_engines: Vec<ManagedDatabaseEngine>,

    /// Tunable dnsmasq daemon settings. `#[serde(default)]` keeps
    /// pre-DNS-settings registry files loading with sane defaults.
    #[serde(default)]
    pub dnsmasq: DnsmasqSettings,

    /// Manually-added runtime installs + default version per language.
    /// `#[serde(default)]` keeps pre-runtimes registry files loading.
    #[serde(default)]
    pub runtimes: RuntimeSettings,

    /// Saved SSH connections (host + auth). The anchor every SSH capability hangs
    /// on; secret-free (passwords live in the OS keychain). `#[serde(default)]`
    /// keeps pre-v3 registry files loading — the v2→v3 migration populates it.
    #[serde(default)]
    pub ssh_connections: Vec<SshConnection>,

    /// Saved SSH port-forwards. Each references an [`SshConnection`] by id and
    /// stores only its forward coordinates.
    #[serde(default)]
    pub ssh_tunnels: Vec<SshTunnelConnection>,

    /// Reusable SSH identities (shared username + key/agent/password method).
    /// Additive and default-empty, so registries written before identities load
    /// unchanged — no schema bump.
    #[serde(default)]
    pub ssh_identities: Vec<SshIdentity>,

    /// Project entries from disk that THIS build could not deserialize — almost
    /// always one written by a newer version that introduced a `type`/enum value
    /// this binary doesn't know yet (e.g. a release reading a registry a dev
    /// build wrote). They are kept out of the typed [`projects`] list so a single
    /// forward-incompatible entry can't fail the whole load, and preserved here
    /// verbatim (as their raw JSON text) so [`store::save_to`] re-emits them
    /// untouched — a downgrade never silently drops a project.
    ///
    /// `#[serde(skip)]`: never read or written through the derived codec. It is
    /// populated only by `store::parse_and_migrate` on load and re-spliced into
    /// the `projects` array by `store::save_to` on save. Stored as `String`
    /// rather than `serde_json::Value` because the latter is not `Eq` (it admits
    /// `f64`), which would break this struct's `Eq` derive.
    #[serde(skip)]
    pub unparsed_projects: Vec<String>,
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
            managed_database_engines: Vec::new(),
            dnsmasq: DnsmasqSettings::default(),
            runtimes: RuntimeSettings::default(),
            ssh_connections: Vec::new(),
            ssh_tunnels: Vec::new(),
            ssh_identities: Vec::new(),
            unparsed_projects: Vec::new(),
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
    ///
    /// Hostname/port uniqueness is enforced one layer up, at the user-input
    /// boundary (the add/update IPC commands and the MCP ops), via
    /// [`Registry::hostname_conflict`] / [`Registry::port_conflict`] — so a
    /// clear error reaches the user, while migrations, importers, and tests can
    /// still construct registries freely.
    pub fn add_project(&mut self, project: Project) -> Result<()> {
        if self.get_project(&project.id).is_some() {
            return Err(RegistryError::DuplicateProjectId(project.id));
        }
        self.projects.push(project);
        Ok(())
    }

    /// True if another project already claims `hostname` (case-insensitive),
    /// ignoring the project whose id equals `exclude` (pass the project's own id
    /// when validating an update so it doesn't conflict with itself). Two
    /// projects sharing a hostname make the second's Caddy route unreachable and
    /// its traffic silently vanish, so the collision is rejected up front.
    pub fn hostname_conflict(&self, hostname: &str, exclude: Option<&ProjectId>) -> bool {
        self.projects
            .iter()
            .any(|p| Some(&p.id) != exclude && p.hostname.eq_ignore_ascii_case(hostname))
    }

    /// True if another project already binds `port`, ignoring `exclude`. A
    /// duplicate port otherwise surfaces only as an opaque runtime bind failure.
    pub fn port_conflict(&self, port: u16, exclude: Option<&ProjectId>) -> bool {
        self.projects
            .iter()
            .any(|p| Some(&p.id) != exclude && p.port == Some(port))
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

    // ---- Managed database engines -----------------------------------------

    /// The PortBay-managed install for `engine`, if one has been downloaded.
    pub fn managed_engine(&self, engine: DatabaseEngine) -> Option<&ManagedDatabaseEngine> {
        self.managed_database_engines
            .iter()
            .find(|m| m.engine == engine)
    }

    /// Record (or replace) a managed engine install. One managed install per
    /// engine — installing a new version supersedes the prior one.
    pub fn upsert_managed_engine(&mut self, managed: ManagedDatabaseEngine) {
        self.managed_database_engines
            .retain(|m| m.engine != managed.engine);
        self.managed_database_engines.push(managed);
    }

    /// Drop the managed install for `engine`, returning it if present.
    pub fn remove_managed_engine(
        &mut self,
        engine: DatabaseEngine,
    ) -> Option<ManagedDatabaseEngine> {
        let idx = self
            .managed_database_engines
            .iter()
            .position(|m| m.engine == engine)?;
        Some(self.managed_database_engines.remove(idx))
    }

    // ---- SSH tunnel CRUD --------------------------------------------------

    pub fn list_ssh_tunnels(&self) -> &[SshTunnelConnection] {
        &self.ssh_tunnels
    }

    pub fn get_ssh_tunnel(&self, id: &SshTunnelId) -> Option<&SshTunnelConnection> {
        self.ssh_tunnels.iter().find(|t| &t.id == id)
    }

    pub fn add_ssh_tunnel(&mut self, tunnel: SshTunnelConnection) -> Result<()> {
        if self.get_ssh_tunnel(&tunnel.id).is_some() {
            return Err(RegistryError::DuplicateSshTunnelId(tunnel.id));
        }
        self.ssh_tunnels.push(tunnel);
        Ok(())
    }

    pub fn update_ssh_tunnel(&mut self, tunnel: SshTunnelConnection) -> Result<()> {
        let slot = self
            .ssh_tunnels
            .iter_mut()
            .find(|t| t.id == tunnel.id)
            .ok_or_else(|| RegistryError::SshTunnelNotFound(tunnel.id.clone()))?;
        *slot = tunnel;
        Ok(())
    }

    pub fn remove_ssh_tunnel(&mut self, id: &SshTunnelId) -> Result<SshTunnelConnection> {
        let idx = self
            .ssh_tunnels
            .iter()
            .position(|t| &t.id == id)
            .ok_or_else(|| RegistryError::SshTunnelNotFound(id.clone()))?;
        Ok(self.ssh_tunnels.remove(idx))
    }

    // ---- SSH connection CRUD ----------------------------------------------

    pub fn list_ssh_connections(&self) -> &[SshConnection] {
        &self.ssh_connections
    }

    pub fn get_ssh_connection(&self, id: &SshConnectionId) -> Option<&SshConnection> {
        self.ssh_connections.iter().find(|c| &c.id == id)
    }

    pub fn add_ssh_connection(&mut self, connection: SshConnection) -> Result<()> {
        if self.get_ssh_connection(&connection.id).is_some() {
            return Err(RegistryError::DuplicateSshConnectionId(connection.id));
        }
        self.ssh_connections.push(connection);
        Ok(())
    }

    pub fn update_ssh_connection(&mut self, connection: SshConnection) -> Result<()> {
        let slot = self
            .ssh_connections
            .iter_mut()
            .find(|c| c.id == connection.id)
            .ok_or_else(|| RegistryError::SshConnectionNotFound(connection.id.clone()))?;
        *slot = connection;
        Ok(())
    }

    pub fn remove_ssh_connection(&mut self, id: &SshConnectionId) -> Result<SshConnection> {
        let idx = self
            .ssh_connections
            .iter()
            .position(|c| &c.id == id)
            .ok_or_else(|| RegistryError::SshConnectionNotFound(id.clone()))?;
        Ok(self.ssh_connections.remove(idx))
    }

    /// Is any tunnel still referencing this connection? Used to avoid deleting a
    /// connection out from under a live forward.
    pub fn ssh_connection_in_use(&self, id: &SshConnectionId) -> bool {
        self.ssh_tunnels.iter().any(|t| &t.connection_id == id)
    }

    // ---- SSH identity CRUD -------------------------------------------------

    pub fn list_ssh_identities(&self) -> &[SshIdentity] {
        &self.ssh_identities
    }

    pub fn get_ssh_identity(&self, id: &SshIdentityId) -> Option<&SshIdentity> {
        self.ssh_identities.iter().find(|i| &i.id == id)
    }

    pub fn add_ssh_identity(&mut self, identity: SshIdentity) -> Result<()> {
        if self.get_ssh_identity(&identity.id).is_some() {
            return Err(RegistryError::DuplicateSshIdentityId(identity.id));
        }
        self.ssh_identities.push(identity);
        Ok(())
    }

    pub fn update_ssh_identity(&mut self, identity: SshIdentity) -> Result<()> {
        let slot = self
            .ssh_identities
            .iter_mut()
            .find(|i| i.id == identity.id)
            .ok_or_else(|| RegistryError::SshIdentityNotFound(identity.id.clone()))?;
        *slot = identity;
        Ok(())
    }

    pub fn remove_ssh_identity(&mut self, id: &SshIdentityId) -> Result<SshIdentity> {
        let idx = self
            .ssh_identities
            .iter()
            .position(|i| &i.id == id)
            .ok_or_else(|| RegistryError::SshIdentityNotFound(id.clone()))?;
        Ok(self.ssh_identities.remove(idx))
    }

    /// Is any connection borrowing this identity? Blocks deleting it out from
    /// under a host that depends on it.
    pub fn ssh_identity_in_use(&self, id: &SshIdentityId) -> bool {
        self.ssh_connections
            .iter()
            .any(|c| c.identity_id.as_ref() == Some(id))
    }

    /// Resolve a connection's effective auth: if it borrows an identity, that
    /// identity supplies `auth_kind`, and the user / key_path the connection
    /// leaves blank. The connection's own non-empty user / key_path win. With no
    /// (or a missing) identity the connection is returned unchanged. The id and
    /// host are never altered, so keychain lookups (keyed by connection id) and
    /// host-key TOFU are unaffected.
    pub fn effective_ssh_connection(&self, conn: &SshConnection) -> SshConnection {
        let Some(identity) = conn
            .identity_id
            .as_ref()
            .and_then(|id| self.get_ssh_identity(id))
        else {
            return conn.clone();
        };
        let mut effective = conn.clone();
        effective.auth_kind = identity.auth_kind;
        if effective.ssh_user.trim().is_empty() {
            effective.ssh_user = identity.ssh_user.clone();
        }
        if effective
            .key_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
        {
            effective.key_path = identity.key_path.clone();
        }
        effective
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

        // migrate() runs every step up to SUPPORTED_VERSION, so a v1 doc lands at v3.
        assert_eq!(reg.version, SUPPORTED_VERSION);

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

    fn v2_registry_with_ssh_tunnels() -> serde_json::Value {
        serde_json::json!({
            "version": 2,
            "domain_suffix": "test",
            "projects": [],
            "ssh_tunnels": [
                // Two forwards to the same host+auth → must collapse to one connection.
                { "id": "db", "name": "DB", "sshHost": "bastion", "sshPort": 22,
                  "sshUser": "deploy", "authKind": "key", "keyPath": "/k",
                  "localHost": "127.0.0.1", "localPort": 15432, "remoteHost": "pg",
                  "remotePort": 5432, "forwardKind": "local", "keepAlive": true,
                  "autoReconnect": false },
                { "id": "cache", "name": "Cache", "sshHost": "bastion", "sshPort": 22,
                  "sshUser": "deploy", "authKind": "key", "keyPath": "/k",
                  "localHost": "127.0.0.1", "localPort": 16379, "remoteHost": "redis",
                  "remotePort": 6379, "forwardKind": "local", "keepAlive": false,
                  "autoReconnect": false },
                // A different host (password auth) → its own connection.
                { "id": "web", "name": "Web", "sshHost": "web1", "sshPort": 2222,
                  "sshUser": "root", "authKind": "password", "localHost": "127.0.0.1",
                  "localPort": 18080, "remoteHost": "localhost", "remotePort": 8080,
                  "forwardKind": "local", "keepAlive": false, "autoReconnect": true }
            ]
        })
    }

    #[test]
    fn migrate_v2_to_v3_extracts_and_dedupes_connections() {
        let migrated = migrate(v2_registry_with_ssh_tunnels(), 2).unwrap();
        let reg: Registry = serde_json::from_value(migrated).unwrap();

        assert_eq!(reg.version, 3);

        // Two distinct hosts → two connections (the two bastion forwards merged).
        assert_eq!(reg.list_ssh_connections().len(), 2);

        // The connection id reuses the FIRST referencing tunnel's id, so a
        // keychain password stored under that id keeps resolving.
        let db = reg.get_ssh_tunnel(&SshTunnelId::new("db")).unwrap();
        let cache = reg.get_ssh_tunnel(&SshTunnelId::new("cache")).unwrap();
        assert_eq!(db.connection_id, SshConnectionId::new("db"));
        assert_eq!(
            cache.connection_id,
            SshConnectionId::new("db"),
            "the second bastion forward reuses the first's connection"
        );

        // The resolved connection carries the host + auth the tunnels dropped.
        let conn = reg.get_ssh_connection(&SshConnectionId::new("db")).unwrap();
        assert_eq!(conn.ssh_host, "bastion");
        assert_eq!(conn.ssh_user, "deploy");
        assert_eq!(conn.auth_kind, SshAuthKind::Key);
        assert_eq!(conn.key_path.as_deref(), Some("/k"));

        // Forward coordinates stayed on the tunnel.
        assert_eq!(db.local_port, 15432);
        assert_eq!(cache.local_port, 16379);
        assert!(db.keep_alive);

        // The web tunnel got its own (password) connection.
        let web = reg.get_ssh_tunnel(&SshTunnelId::new("web")).unwrap();
        assert_eq!(web.connection_id, SshConnectionId::new("web"));
        let web_conn = reg
            .get_ssh_connection(&SshConnectionId::new("web"))
            .unwrap();
        assert_eq!(web_conn.auth_kind, SshAuthKind::Password);
        assert_eq!(web_conn.ssh_port, 2222);
        assert!(web.auto_reconnect);
    }

    #[test]
    fn migrate_v2_to_v3_handles_empty_and_is_idempotent() {
        // No ssh_tunnels at all → empty connections, clean load.
        let doc = serde_json::json!({
            "version": 2, "domain_suffix": "test", "projects": []
        });
        let reg: Registry = serde_json::from_value(migrate(doc, 2).unwrap()).unwrap();
        assert!(reg.list_ssh_connections().is_empty());
        assert!(reg.list_ssh_tunnels().is_empty());

        // Re-running the step over an already-migrated tunnel (has connectionId)
        // leaves it untouched — no second connection, no stripped fields.
        let once = migrate(v2_registry_with_ssh_tunnels(), 2).unwrap();
        let twice = migrate_v2_to_v3(once.clone()).unwrap();
        let reg_once: Registry = serde_json::from_value(once).unwrap();
        let reg_twice: Registry = serde_json::from_value(twice).unwrap();
        assert_eq!(
            reg_once.list_ssh_connections().len(),
            reg_twice.list_ssh_connections().len()
        );
    }

    #[test]
    fn managed_engine_upsert_replaces_and_remove_works() {
        let mut reg = Registry::new("test");
        assert!(reg.managed_engine(DatabaseEngine::Redis).is_none());

        reg.upsert_managed_engine(ManagedDatabaseEngine {
            engine: DatabaseEngine::Redis,
            version: "7.2.0".into(),
            dir: PathBuf::from("/x/redis/7.2.0"),
            arch: "aarch64".into(),
        });
        assert_eq!(
            reg.managed_engine(DatabaseEngine::Redis).unwrap().version,
            "7.2.0"
        );

        // Upsert supersedes the prior version — one managed install per engine.
        reg.upsert_managed_engine(ManagedDatabaseEngine {
            engine: DatabaseEngine::Redis,
            version: "7.4.0".into(),
            dir: PathBuf::from("/x/redis/7.4.0"),
            arch: "aarch64".into(),
        });
        assert_eq!(reg.managed_database_engines.len(), 1);
        assert_eq!(
            reg.managed_engine(DatabaseEngine::Redis).unwrap().version,
            "7.4.0"
        );

        let removed = reg.remove_managed_engine(DatabaseEngine::Redis);
        assert_eq!(removed.unwrap().version, "7.4.0");
        assert!(reg.managed_engine(DatabaseEngine::Redis).is_none());
    }

    #[test]
    fn effective_connection_overlays_identity_but_connection_overrides_win() {
        let mut reg = Registry::new("test");
        reg.add_ssh_identity(SshIdentity {
            id: SshIdentityId::new("deploy"),
            name: "Deploy".into(),
            ssh_user: "deploy".into(),
            auth_kind: SshAuthKind::Key,
            key_path: Some("/keys/id_ed25519".into()),
        })
        .unwrap();

        // A connection that borrows the identity and leaves user/key blank.
        let borrow = SshConnection {
            id: SshConnectionId::new("h1"),
            name: "Host 1".into(),
            ssh_host: "h1.example.com".into(),
            ssh_port: 22,
            ssh_user: String::new(),
            auth_kind: SshAuthKind::Password, // overridden by the identity
            key_path: None,
            proxy_jump: None,
            identity_id: Some(SshIdentityId::new("deploy")),
            proxy: None,
            metadata: SshConnectionMeta::default(),
        };
        let eff = reg.effective_ssh_connection(&borrow);
        assert_eq!(eff.ssh_user, "deploy", "blank user inherits from identity");
        assert_eq!(eff.key_path.as_deref(), Some("/keys/id_ed25519"));
        assert_eq!(eff.auth_kind, SshAuthKind::Key, "identity defines auth");
        assert_eq!(eff.ssh_host, "h1.example.com", "host is never altered");

        // A connection whose own user/key are set keeps them (override wins).
        let override_conn = SshConnection {
            ssh_user: "root".into(),
            key_path: Some("/keys/other".into()),
            ..borrow.clone()
        };
        let eff2 = reg.effective_ssh_connection(&override_conn);
        assert_eq!(eff2.ssh_user, "root");
        assert_eq!(eff2.key_path.as_deref(), Some("/keys/other"));

        // No identity → returned unchanged.
        let plain = SshConnection {
            identity_id: None,
            ..borrow.clone()
        };
        let eff3 = reg.effective_ssh_connection(&plain);
        assert_eq!(eff3.ssh_user, "");
        assert_eq!(eff3.auth_kind, SshAuthKind::Password);
    }

    #[test]
    fn identity_in_use_blocks_while_a_connection_borrows_it() {
        let mut reg = Registry::new("test");
        reg.add_ssh_identity(SshIdentity {
            id: SshIdentityId::new("id1"),
            name: "One".into(),
            ssh_user: "u".into(),
            auth_kind: SshAuthKind::Key,
            key_path: None,
        })
        .unwrap();
        assert!(!reg.ssh_identity_in_use(&SshIdentityId::new("id1")));

        reg.add_ssh_connection(SshConnection {
            id: SshConnectionId::new("c1"),
            name: "C1".into(),
            ssh_host: "h".into(),
            ssh_port: 22,
            ssh_user: String::new(),
            auth_kind: SshAuthKind::Key,
            key_path: None,
            proxy_jump: None,
            identity_id: Some(SshIdentityId::new("id1")),
            proxy: None,
            metadata: SshConnectionMeta::default(),
        })
        .unwrap();
        assert!(reg.ssh_identity_in_use(&SshIdentityId::new("id1")));
    }
}
