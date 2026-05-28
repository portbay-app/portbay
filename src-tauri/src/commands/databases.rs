//! Database commands — engine catalogue + owned-instance lifecycle.
//!
//! PortBay owns database *instances*: it provisions an isolated data dir,
//! renders a config, and supervises the daemon through Process Compose (the
//! same surface projects use). Engine binaries are installed via Homebrew;
//! everything after that is PortBay's own lifecycle.
//!
//! Command surface:
//!   - `list_database_engines`        — available engines for the Add wizard
//!   - `install_database_engine`      — `brew install <formula>`
//!   - `list_database_instances`      — the user's instances + live status
//!   - `create_database_instance`     — provision + register a new instance
//!   - `remove_database_instance`     — stop + deregister (+ optional data wipe)
//!   - `start/stop/restart_database_instance`
//!   - `link_database_to_project` / `unlink_database_from_project`
//!   - `open_database_client`         — launch the CLI in Terminal

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_shell::ShellExt;

use crate::commands::projects::{load_registry, save_registry, slugify};
use crate::commands::runtimes::{fetch_signed_manifest, newest_entry, InstallEvent};
use crate::databases as engine;
use crate::error::{AppError, AppResult};
use crate::registry::{
    DatabaseEngine, DatabaseInstance, DatabaseInstanceId, ManagedDatabaseEngine, ProjectId,
    Registry,
};
use crate::state::AppState;

/// App-event channel mirroring the per-call `Channel<InstallEvent>` for the
/// managed database-engine install (parallels `portbay://runtime-install`).
const DB_ENGINE_INSTALL_CHANNEL: &str = "portbay://db-engine-install";

// ===========================================================================
// Wire types
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseEngineView {
    pub id: String,
    pub label: String,
    pub installed: bool,
    pub version: String,
    pub default_port: u16,
    pub client_available: bool,
    pub install_hint: String,
    /// True when a PortBay-managed build of this engine is installed.
    pub managed: bool,
    /// Version of the managed build, when `managed` is true.
    pub managed_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Running,
    Stopped,
    Starting,
    Errored,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInstanceView {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub engine_label: String,
    pub version: String,
    pub port: u16,
    pub status: InstanceStatus,
    pub auto_start: bool,
    pub data_dir: String,
    pub config_path: Option<String>,
    pub socket_path: Option<String>,
    pub connection_url: String,
    pub account: String,
    pub linked_projects: Vec<String>,
    /// True when the daemon binary still resolves on this machine.
    pub binary_available: bool,
    /// True when the data dir has been provisioned.
    pub provisioned: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDatabaseInput {
    /// Engine id ("mysql", "postgres", "redis", "mongo", "mariadb").
    pub engine: String,
    /// User-facing name. Slugified into the instance id.
    pub name: String,
    /// Optional explicit port; auto-allocated when absent.
    pub port: Option<u16>,
    #[serde(default)]
    pub auto_start: bool,
}

// ===========================================================================
// Engine catalogue
// ===========================================================================

const ALL_ENGINES: &[DatabaseEngine] = &[
    DatabaseEngine::Mysql,
    DatabaseEngine::Postgres,
    DatabaseEngine::Mariadb,
    DatabaseEngine::Redis,
    DatabaseEngine::Mongo,
    DatabaseEngine::Memcached,
];

/// Fallback hint shown when an engine isn't installed and no managed build is
/// published yet — the managed install button is the primary path.
fn install_hint(e: DatabaseEngine) -> &'static str {
    match e {
        DatabaseEngine::Mysql => "brew install mysql",
        DatabaseEngine::Postgres => "brew install postgresql@16",
        DatabaseEngine::Mariadb => "brew install mariadb",
        DatabaseEngine::Redis => "brew install redis",
        DatabaseEngine::Mongo => "brew install mongodb-community",
        DatabaseEngine::Memcached => "brew install memcached",
    }
}

/// The `bin` dir of a PortBay-managed engine install, when one exists.
fn managed_bin(registry: &Registry, engine: DatabaseEngine) -> Option<PathBuf> {
    registry
        .managed_engine(engine)
        .map(|m| engine::managed_bin_dir(&m.dir))
}

/// `list_database_engines()` — every supported engine, with install state.
/// Reports a PortBay-managed install ahead of any Homebrew/system copy.
#[tauri::command]
pub async fn list_database_engines(
    state: State<'_, AppState>,
) -> AppResult<Vec<DatabaseEngineView>> {
    let registry = load_registry(&state)?;
    Ok(ALL_ENGINES
        .iter()
        .map(|&e| {
            let managed = registry.managed_engine(e);
            let mb = managed.map(|m| engine::managed_bin_dir(&m.dir));
            let det = engine::detect_resolved(e, mb.as_deref());
            DatabaseEngineView {
                id: e.id().into(),
                label: e.label().into(),
                installed: det.installed,
                version: det.version,
                default_port: e.default_port(),
                client_available: det.client.is_some(),
                install_hint: install_hint(e).into(),
                managed: managed.is_some(),
                managed_version: managed.map(|m| m.version.clone()).unwrap_or_default(),
            }
        })
        .collect())
}

/// `install_database_engine(engine, onEvent)` — download a signed, PortBay-managed
/// build of the engine into `<app-data>/database-engines/<engine>/<version>/` and
/// register it. Reuses the runtime download/verify/install pipeline (same signed
/// manifest); the engine id is the manifest `lang`. Progress streams over the
/// channel, mirroring `install_runtime`.
#[tauri::command]
pub async fn install_database_engine(
    app: AppHandle,
    state: State<'_, AppState>,
    engine: String,
    on_event: Channel<InstallEvent>,
) -> AppResult<()> {
    let eng = parse_engine(&engine)?;

    let _ = on_event.send(InstallEvent::Log {
        line: "Fetching signed PortBay manifest…".into(),
    });
    let manifest = fetch_signed_manifest().await?;
    let arch = crate::runtimes::download::manifest::current_arch();
    let entry = newest_entry(&manifest, eng.id(), arch).ok_or_else(|| {
        AppError::BadInput(format!(
            "no PortBay-managed {} build is published for {arch} yet",
            eng.label()
        ))
    })?;

    let app_data = app_data_dir(&state)?;
    let dest_root = engine::engines_root(&app_data);
    let expected = engine::expected_daemon_rel(eng);
    let version = entry.version.clone();
    let install_arch = entry.arch.clone();
    let app_for_progress = app.clone();
    let channel = on_event.clone();
    let _ = on_event.send(InstallEvent::Log {
        line: format!("Installing {} {} ({install_arch})…", eng.label(), version),
    });
    let binary = crate::runtimes::download::install::fetch_and_install(
        &entry,
        &dest_root,
        &expected,
        move |downloaded, total| {
            let ev = InstallEvent::Progress { downloaded, total };
            let _ = channel.send(ev.clone());
            let _ = app_for_progress.emit(DB_ENGINE_INSTALL_CHANNEL, ev);
        },
        |bin| probe_engine(bin),
    )
    .await
    .map_err(|e| AppError::Internal(format!("engine install failed: {e}")))?;

    // binary = <dest_root>/<engine>/<version>/bin/<daemon>; the install root
    // (what we record + resolve `bin/` under) is two parents up.
    let install_dir = binary
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AppError::Internal("installed engine path is malformed".into()))?
        .to_path_buf();

    let mut reg = load_registry(&state)?;
    reg.upsert_managed_engine(ManagedDatabaseEngine {
        engine: eng,
        version,
        dir: install_dir,
        arch: install_arch,
    });
    save_registry(&state, &reg)?;

    let done = InstallEvent::Done { success: true };
    let _ = app.emit(DB_ENGINE_INSTALL_CHANNEL, done.clone());
    let _ = on_event.send(done);
    Ok(())
}

/// `remove_managed_engine(engine)` — drop a PortBay-managed engine install and
/// delete its binaries. Instances fall back to any Homebrew/system copy.
#[tauri::command]
pub async fn remove_managed_engine(state: State<'_, AppState>, engine: String) -> AppResult<()> {
    let eng = parse_engine(&engine)?;
    let app_data = app_data_dir(&state)?;
    let mut reg = load_registry(&state)?;
    let removed = reg.remove_managed_engine(eng);
    save_registry(&state, &reg)?;
    if let Some(m) = removed {
        // Only ever delete inside our own engines root.
        if m.dir.starts_with(engine::engines_root(&app_data)) && m.dir.exists() {
            std::fs::remove_dir_all(&m.dir).map_err(|e| {
                AppError::Internal(format!("delete engine dir {}: {e}", m.dir.display()))
            })?;
        }
    }
    state.reconciler.mark_dirty();
    Ok(())
}

/// Validate a freshly-extracted engine daemon: it runs and reports a version.
fn probe_engine(bin: &Path) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false)
}

// ===========================================================================
// Instance listing
// ===========================================================================

/// `list_database_instances()` — registry instances merged with live PC state.
#[tauri::command]
pub async fn list_database_instances(
    state: State<'_, AppState>,
) -> AppResult<Vec<DatabaseInstanceView>> {
    let registry = load_registry(&state)?;
    let app_data = app_data_dir(&state)?;

    // Snapshot PC processes once; graceful-degrade to Stopped if unreachable.
    let pc = crate::commands::projects::fetch_pc_state(&state).await;

    let views = registry
        .list_databases()
        .iter()
        .map(|inst| {
            let proc = pc.as_ref().and_then(|m| m.get(&inst.process_id()));
            let status = match proc {
                None => InstanceStatus::Stopped,
                Some(p) => {
                    let s = p.status.to_lowercase();
                    if p.is_running && (s.contains("running") || s.contains("ready")) {
                        InstanceStatus::Running
                    } else if s.contains("launching") || s.contains("starting") {
                        InstanceStatus::Starting
                    } else if s.contains("error") || s.contains("failed") {
                        InstanceStatus::Errored
                    } else {
                        InstanceStatus::Stopped
                    }
                }
            };
            let mb = managed_bin(&registry, inst.engine);
            instance_view(inst, status, &app_data, mb.as_deref())
        })
        .collect();
    Ok(views)
}

fn instance_view(
    inst: &DatabaseInstance,
    status: InstanceStatus,
    app_data: &Path,
    managed_bin: Option<&Path>,
) -> DatabaseInstanceView {
    let data = engine::data_dir(app_data, inst.id.as_str());
    DatabaseInstanceView {
        id: inst.id.to_string(),
        name: inst.name.clone(),
        engine: inst.engine.id().into(),
        engine_label: inst.engine.label().into(),
        version: inst.version.clone(),
        port: inst.port,
        status,
        auto_start: inst.auto_start,
        data_dir: data.to_string_lossy().into_owned(),
        config_path: engine::config_path(inst.engine, app_data, inst.id.as_str())
            .map(|p| p.to_string_lossy().into_owned()),
        socket_path: inst
            .socket_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        connection_url: inst.connection_url(),
        account: inst.default_account().into(),
        linked_projects: inst.linked_projects.iter().map(|p| p.to_string()).collect(),
        binary_available: engine::daemon_binary_resolved(inst.engine, managed_bin).is_some(),
        provisioned: engine::is_initialized(inst.engine, &data),
    }
}

// ===========================================================================
// Create / remove
// ===========================================================================

/// `create_database_instance(input)` — provision + register a new instance,
/// then force a reconcile so the daemon process exists in Process Compose.
#[tauri::command]
pub async fn create_database_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CreateDatabaseInput,
) -> AppResult<DatabaseInstanceView> {
    let eng = parse_engine(&input.engine)?;

    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadInput("a name is required".into()));
    }

    let mut registry = load_registry(&state)?;
    let id = unique_instance_id(&registry, name);
    let app_data = app_data_dir(&state)?;

    // Daemon must be installed before we can provision — prefer a PortBay-managed
    // build, falling back to a Homebrew/system copy.
    let mb = managed_bin(&registry, eng);
    let daemon = engine::daemon_binary_resolved(eng, mb.as_deref()).ok_or_else(|| {
        AppError::BadInput(format!(
            "{} isn't installed. Install it from the Databases page first.",
            eng.label()
        ))
    })?;

    let port = match input.port {
        Some(p) => {
            if registry.database_port_in_use(p, None) {
                return Err(AppError::BadInput(format!(
                    "port {p} is already used by another database instance"
                )));
            }
            p
        }
        None => allocate_port(&registry, eng),
    };

    // Provision: init data dir + write config. This shells out to
    // `mysqld --initialize-insecure` / `initdb`, which can take 30–120s.
    // Run it off the async runtime so status, metrics, and log-stream IPC
    // stay responsive while the GUI shows its spinner.
    let detection = engine::detect_resolved(eng, mb.as_deref());
    let provision_data = app_data.clone();
    let provision_id = id.clone();
    let provision_managed = mb.clone();
    tokio::task::spawn_blocking(move || {
        engine::provision(
            eng,
            &daemon,
            &provision_data,
            &provision_id,
            port,
            provision_managed.as_deref(),
        )
    })
    .await
    .map_err(|e| AppError::Internal(format!("provision join: {e}")))?
    .map_err(AppError::Internal)?;

    let instance = DatabaseInstance {
        id: DatabaseInstanceId::new(id.clone()),
        name: name.to_string(),
        engine: eng,
        version: detection.version,
        port,
        data_dir: engine::data_dir(&app_data, &id),
        config_path: engine::config_path(eng, &app_data, &id),
        socket_path: engine::socket_path(eng, &app_data, &id),
        auto_start: input.auto_start,
        linked_projects: vec![],
    };

    registry.add_database(instance.clone())?;
    save_registry(&state, &registry)?;

    // Force the reconciler to regenerate the PC YAML so the new daemon
    // process exists (and auto-starts if requested) before we return.
    let _ = state.reconciler.tick(&app).await;

    Ok(instance_view(
        &instance,
        InstanceStatus::Stopped,
        &app_data,
        mb.as_deref(),
    ))
}

/// `remove_database_instance(id, deleteData)` — stop the daemon, drop it
/// from the registry, unlink it from any projects, and optionally delete
/// the data directory.
#[tauri::command]
pub async fn remove_database_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    delete_data: bool,
) -> AppResult<()> {
    let did = DatabaseInstanceId::new(id.clone());
    let app_data = app_data_dir(&state)?;

    // Best-effort stop first so we don't orphan a running daemon.
    let process_id = format!("db-{id}");
    if let Ok(client) = state.pc_client() {
        let _ = client.stop(&process_id).await;
    }

    let mut registry = load_registry(&state)?;
    let removed = registry.remove_database(&did)?;
    save_registry(&state, &registry)?;

    // Regenerate YAML (drops the daemon process) before touching disk.
    let _ = state.reconciler.tick(&app).await;

    if delete_data {
        let dir = engine::instance_dir(&app_data, removed.id.as_str());
        if dir.starts_with(engine::instances_root(&app_data)) && dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| {
                AppError::Internal(format!("delete data dir {}: {e}", dir.display()))
            })?;
        }
    }
    Ok(())
}

// ===========================================================================
// Lifecycle
// ===========================================================================

/// `start_database_instance(id)` — start the daemon via Process Compose.
#[tauri::command]
pub async fn start_database_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    require_instance(&state, &id)?;
    // Make sure the daemon process exists in PC's loaded YAML (a stale
    // reconcile could otherwise 404 the start).
    let _ = state.reconciler.tick(&app).await;
    let client = state.pc_client()?;
    client
        .start(&format!("db-{id}"))
        .await
        .map_err(|e| AppError::Internal(format!("start failed: {e}")))
}

/// `stop_database_instance(id)`.
#[tauri::command]
pub async fn stop_database_instance(state: State<'_, AppState>, id: String) -> AppResult<()> {
    require_instance(&state, &id)?;
    let client = state.pc_client()?;
    client
        .stop(&format!("db-{id}"))
        .await
        .map_err(|e| AppError::Internal(format!("stop failed: {e}")))
}

/// `restart_database_instance(id)`.
#[tauri::command]
pub async fn restart_database_instance(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    require_instance(&state, &id)?;
    let _ = state.reconciler.tick(&app).await;
    let client = state.pc_client()?;
    client
        .restart(&format!("db-{id}"))
        .await
        .map_err(|e| AppError::Internal(format!("restart failed: {e}")))
}

// ===========================================================================
// Project binding
// ===========================================================================

/// `link_database_to_project(id, projectId)` — bind an instance to a
/// project. The reconciler injects this instance's connection env vars into
/// the project's process on the next tick.
#[tauri::command]
pub async fn link_database_to_project(
    state: State<'_, AppState>,
    id: String,
    project_id: String,
) -> AppResult<()> {
    let did = DatabaseInstanceId::new(id);
    let pid = ProjectId::new(project_id.clone());

    let mut registry = load_registry(&state)?;
    if registry.get_project(&pid).is_none() {
        return Err(AppError::NotFound(project_id));
    }
    let inst = registry
        .get_database_mut(&did)
        .ok_or_else(|| AppError::BadInput(format!("database `{did}` not found")))?;
    if !inst.linked_projects.contains(&pid) {
        inst.linked_projects.push(pid);
    }
    save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();
    Ok(())
}

/// `unlink_database_from_project(id, projectId)`.
#[tauri::command]
pub async fn unlink_database_from_project(
    state: State<'_, AppState>,
    id: String,
    project_id: String,
) -> AppResult<()> {
    let did = DatabaseInstanceId::new(id);
    let pid = ProjectId::new(project_id);

    let mut registry = load_registry(&state)?;
    let inst = registry
        .get_database_mut(&did)
        .ok_or_else(|| AppError::BadInput(format!("database `{did}` not found")))?;
    inst.linked_projects.retain(|p| p != &pid);
    save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();
    Ok(())
}

/// `set_database_auto_start(id, autoStart)`.
#[tauri::command]
pub async fn set_database_auto_start(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    auto_start: bool,
) -> AppResult<()> {
    let did = DatabaseInstanceId::new(id);
    let mut registry = load_registry(&state)?;
    let inst = registry
        .get_database_mut(&did)
        .ok_or_else(|| AppError::BadInput(format!("database `{did}` not found")))?;
    inst.auto_start = auto_start;
    save_registry(&state, &registry)?;
    let _ = state.reconciler.tick(&app).await;
    Ok(())
}

// ===========================================================================
// Client launcher
// ===========================================================================

/// `open_database_client(id)` — launch the engine CLI in Terminal.app,
/// pointed at the instance's port.
#[tauri::command]
pub async fn open_database_client(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let registry = load_registry(&state)?;
    let inst = registry
        .get_database(&DatabaseInstanceId::new(id.clone()))
        .ok_or_else(|| AppError::BadInput(format!("database `{id}` not found")))?;
    let mb = managed_bin(&registry, inst.engine);
    let client = engine::client_binary_resolved(inst.engine, mb.as_deref()).ok_or_else(|| {
        AppError::BadInput(format!("no CLI client for {} found.", inst.engine.label()))
    })?;
    let command = engine::client_invocation(inst, &client);
    open_in_terminal(&app, &command).await
}

// ===========================================================================
// Per-database (schema) management
// ===========================================================================

/// Resolve a running-instance + its CLI client (managed-aware) for schema ops.
fn instance_and_client(
    state: &State<'_, AppState>,
    id: &str,
) -> AppResult<(DatabaseInstance, PathBuf)> {
    let registry = load_registry(state)?;
    let inst = registry
        .get_database(&DatabaseInstanceId::new(id))
        .ok_or_else(|| AppError::BadInput(format!("database `{id}` not found")))?;
    let mb = managed_bin(&registry, inst.engine);
    let client = engine::client_binary_resolved(inst.engine, mb.as_deref()).ok_or_else(|| {
        AppError::BadInput(format!("no CLI client for {} found.", inst.engine.label()))
    })?;
    Ok((inst.clone(), client))
}

/// `list_instance_databases(id)` — the databases/schemas inside the instance.
/// Empty for engines without a schema namespace (Redis/Mongo/Memcached).
#[tauri::command]
pub async fn list_instance_databases(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Vec<String>> {
    let (inst, client) = instance_and_client(&state, &id)?;
    if !engine::supports_schema_management(inst.engine) {
        return Ok(vec![]);
    }
    tokio::task::spawn_blocking(move || engine::list_schemas(&inst, &client))
        .await
        .map_err(|e| AppError::Internal(format!("schema list join: {e}")))?
        .map_err(AppError::Internal)
}

/// `create_instance_database(id, name)` — create a schema in a running instance.
#[tauri::command]
pub async fn create_instance_database(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<()> {
    let (inst, client) = instance_and_client(&state, &id)?;
    tokio::task::spawn_blocking(move || engine::create_schema(&inst, &client, &name))
        .await
        .map_err(|e| AppError::Internal(format!("schema create join: {e}")))?
        .map_err(AppError::Internal)
}

/// `drop_instance_database(id, name)` — drop a schema from a running instance.
#[tauri::command]
pub async fn drop_instance_database(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<()> {
    let (inst, client) = instance_and_client(&state, &id)?;
    tokio::task::spawn_blocking(move || engine::drop_schema(&inst, &client, &name))
        .await
        .map_err(|e| AppError::Internal(format!("schema drop join: {e}")))?
        .map_err(AppError::Internal)
}

// ===========================================================================
// Per-project provisioning
// ===========================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDbProvision {
    pub database: String,
    pub username: String,
    pub connection_url: String,
}

/// `provision_project_database(projectId, instanceId, password)` — create a
/// dedicated database + login user (named after the project) on a running SQL
/// instance and inject DB_* / DATABASE_URL into the project's env. The password
/// is generated client-side (Web Crypto) and must be alphanumeric.
#[tauri::command]
pub async fn provision_project_database(
    state: State<'_, AppState>,
    project_id: String,
    instance_id: String,
    password: String,
) -> AppResult<ProjectDbProvision> {
    let (inst, client) = instance_and_client(&state, &instance_id)?;
    if !engine::supports_schema_management(inst.engine) {
        return Err(AppError::BadInput(format!(
            "{} can't host a per-project database.",
            inst.engine.label()
        )));
    }

    let pid = ProjectId::new(project_id.clone());
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&pid)
        .ok_or_else(|| AppError::NotFound(project_id.clone()))?;
    let base = engine::sanitize_identifier(project.id.as_str());
    let database = format!("{base}_dev");
    let username = base;
    let env_path = project.path.join(".env");

    // Provision off the async runtime (it shells out to the engine client).
    let inst_for_blocking = inst.clone();
    let client_for_blocking = client.clone();
    let (db, user, pw) = (database.clone(), username.clone(), password.clone());
    tokio::task::spawn_blocking(move || {
        engine::provision_app_database(&inst_for_blocking, &client_for_blocking, &db, &user, &pw)
    })
    .await
    .map_err(|e| AppError::Internal(format!("provision join: {e}")))?
    .map_err(AppError::Internal)?;

    let connection_url = engine::app_connection_url(&inst, &username, &password, &database);

    // Write the connection into the project's on-disk .env so the framework
    // picks it up however it's launched, and it surfaces in the Database panel.
    // Only the DB_* / DATABASE_URL keys are touched; everything else is left as-is.
    let pairs: Vec<(&str, String)> = vec![
        ("DB_CONNECTION", inst.engine.id().to_string()),
        ("DB_HOST", "127.0.0.1".to_string()),
        ("DB_PORT", inst.port.to_string()),
        ("DB_DATABASE", database.clone()),
        ("DB_USERNAME", username.clone()),
        ("DB_PASSWORD", password.clone()),
        ("DATABASE_URL", connection_url.clone()),
    ];
    upsert_dotenv(&env_path, &pairs)
        .map_err(|e| AppError::Internal(format!("write {}: {e}", env_path.display())))?;

    Ok(ProjectDbProvision {
        database,
        username,
        connection_url,
    })
}

/// Upsert `KEY=value` pairs into a `.env` file: existing keys are rewritten in
/// place (comments and unrelated lines preserved), missing keys are appended
/// under a PortBay header. Creates the file (and parent dir) if absent. Values
/// here are alphanumeric / URL-safe, so no quoting is needed.
fn upsert_dotenv(path: &Path, pairs: &[(&str, String)]) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    let mut applied = vec![false; pairs.len()];

    for line in lines.iter_mut() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        if let Some(i) = pairs.iter().position(|(k, _)| *k == key) {
            *line = format!("{}={}", pairs[i].0, pairs[i].1);
            applied[i] = true;
        }
    }

    let missing: Vec<&(&str, String)> = pairs
        .iter()
        .zip(applied.iter())
        .filter(|(_, done)| !**done)
        .map(|(p, _)| p)
        .collect();
    if !missing.is_empty() {
        if lines.last().map(|l| !l.is_empty()).unwrap_or(false) {
            lines.push(String::new());
        }
        lines.push("# PortBay-provisioned database".to_string());
        for (k, v) in missing {
            lines.push(format!("{k}={v}"));
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(path, out)
}

// ===========================================================================
// Backups & restore
// ===========================================================================

/// Resolve an instance + its managed bin dir (if any) for backup tooling.
fn instance_and_managed_bin(
    state: &State<'_, AppState>,
    id: &str,
) -> AppResult<(DatabaseInstance, Option<PathBuf>)> {
    let registry = load_registry(state)?;
    let inst = registry
        .get_database(&DatabaseInstanceId::new(id))
        .ok_or_else(|| AppError::BadInput(format!("database `{id}` not found")))?;
    let mb = managed_bin(&registry, inst.engine);
    Ok((inst.clone(), mb))
}

/// `list_database_backups(id)` — snapshots on disk, newest first.
#[tauri::command]
pub async fn list_database_backups(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Vec<engine::backup::BackupSnapshot>> {
    let app_data = app_data_dir(&state)?;
    Ok(engine::backup::list_backups(&app_data, &id))
}

/// `backup_database_instance(id)` — dump the instance, then prune old snapshots.
#[tauri::command]
pub async fn backup_database_instance(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<engine::backup::BackupSnapshot> {
    let (inst, mb) = instance_and_managed_bin(&state, &id)?;
    let app_data = app_data_dir(&state)?;

    let ad = app_data.clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        engine::backup::create_backup(&inst, mb.as_deref(), &ad)
    })
    .await
    .map_err(|e| AppError::Internal(format!("backup join: {e}")))?
    .map_err(AppError::Internal)?;

    // Retention: prune past the default window (best-effort).
    let ad = app_data.clone();
    let pid = id.clone();
    let _ = tokio::task::spawn_blocking(move || {
        engine::backup::prune(&ad, &pid, engine::backup::DEFAULT_KEEP_DAYS)
    })
    .await;

    Ok(snapshot)
}

/// `restore_database_backup(id, snapshotId)` — replay a snapshot's dump.
#[tauri::command]
pub async fn restore_database_backup(
    state: State<'_, AppState>,
    id: String,
    snapshot_id: String,
) -> AppResult<()> {
    let (inst, mb) = instance_and_managed_bin(&state, &id)?;
    let app_data = app_data_dir(&state)?;
    tokio::task::spawn_blocking(move || {
        engine::backup::restore_backup(&inst, mb.as_deref(), &app_data, &snapshot_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("restore join: {e}")))?
    .map_err(AppError::Internal)
}

/// `delete_database_backup(id, snapshotId)` — remove one snapshot.
#[tauri::command]
pub async fn delete_database_backup(
    state: State<'_, AppState>,
    id: String,
    snapshot_id: String,
) -> AppResult<()> {
    let app_data = app_data_dir(&state)?;
    engine::backup::delete_backup(&app_data, &id, &snapshot_id).map_err(AppError::Internal)
}

async fn open_in_terminal(app: &AppHandle, command: &str) -> AppResult<()> {
    let safe = command.replace('"', "\\\"");
    let script =
        format!("tell application \"Terminal\"\n  activate\n  do script \"{safe}\"\nend tell");
    app.shell()
        .command("osascript")
        .args(["-e", &script])
        .spawn()
        .map_err(|e| AppError::Internal(format!("failed to open Terminal.app: {e}")))?;
    Ok(())
}

// ===========================================================================
// Helpers
// ===========================================================================

fn parse_engine(s: &str) -> AppResult<DatabaseEngine> {
    DatabaseEngine::from_id(s).ok_or_else(|| AppError::BadInput(format!("unknown engine: {s}")))
}

fn require_instance(state: &State<'_, AppState>, id: &str) -> AppResult<()> {
    let registry = load_registry(state)?;
    if registry
        .get_database(&DatabaseInstanceId::new(id))
        .is_none()
    {
        return Err(AppError::BadInput(format!("database `{id}` not found")));
    }
    Ok(())
}

/// PortBay app-data dir — the parent of `logs_dir` (e.g.
/// `~/Library/Application Support/PortBay`).
fn app_data_dir(state: &State<'_, AppState>) -> AppResult<PathBuf> {
    state
        .logs_dir
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| AppError::Internal("could not resolve app-data dir".into()))
}

/// Slugify the name, then ensure uniqueness against existing instances by
/// appending `-2`, `-3`, … on collision.
fn unique_instance_id(registry: &Registry, name: &str) -> String {
    let base = {
        let s = slugify(name);
        if s.is_empty() {
            "db".to_string()
        } else {
            s
        }
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while registry
        .get_database(&DatabaseInstanceId::new(candidate.clone()))
        .is_some()
    {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

/// Find a free port for the engine: start at its default, walk up until one
/// is free both in the registry and on the host.
fn allocate_port(registry: &Registry, eng: DatabaseEngine) -> u16 {
    let mut port = eng.default_port();
    for _ in 0..500 {
        if !registry.database_port_in_use(port, None) && crate::port_holder::find(port).is_none() {
            return port;
        }
        port = port.saturating_add(1);
        if port == u16::MAX {
            break;
        }
    }
    eng.default_port()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_engine_accepts_known_ids() {
        assert!(parse_engine("mysql").is_ok());
        assert!(parse_engine("mariadb").is_ok());
        assert!(parse_engine("postgres").is_ok());
        assert!(parse_engine("mongo").is_ok());
        assert!(parse_engine("redis").is_ok());
        assert!(parse_engine("memcached").is_ok());
        assert!(parse_engine("clickhouse").is_err());
    }

    #[test]
    fn every_supported_engine_has_ui_metadata() {
        let expected = [
            DatabaseEngine::Mysql,
            DatabaseEngine::Postgres,
            DatabaseEngine::Mariadb,
            DatabaseEngine::Redis,
            DatabaseEngine::Mongo,
            DatabaseEngine::Memcached,
        ];

        assert_eq!(ALL_ENGINES, expected);

        for engine in ALL_ENGINES {
            assert_eq!(DatabaseEngine::from_id(engine.id()), Some(*engine));
            assert!(!engine.label().is_empty());
            assert!(engine.default_port() > 0);
            assert!(install_hint(*engine).starts_with("brew install "));
        }
    }

    #[test]
    fn unique_id_appends_suffix_on_collision() {
        let mut reg = Registry::new("test");
        let inst = DatabaseInstance {
            id: DatabaseInstanceId::new("myapp"),
            name: "myapp".into(),
            engine: DatabaseEngine::Redis,
            version: "7".into(),
            port: 6379,
            data_dir: PathBuf::from("/x"),
            config_path: None,
            socket_path: None,
            auto_start: false,
            linked_projects: vec![],
        };
        reg.add_database(inst).unwrap();
        assert_eq!(unique_instance_id(&reg, "MyApp"), "myapp-2");
        assert_eq!(unique_instance_id(&reg, "Other DB"), "other-db");
    }

    #[test]
    fn upsert_dotenv_updates_existing_and_appends_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let env = tmp.path().join(".env");
        std::fs::write(
            &env,
            "APP_ENV=local\n# comment\nDB_HOST=oldhost\nDB_PASSWORD=stale\n",
        )
        .unwrap();

        let pairs: Vec<(&str, String)> = vec![
            ("DB_HOST", "127.0.0.1".to_string()),
            ("DB_PASSWORD", "newpw".to_string()),
            ("DB_DATABASE", "myapp_dev".to_string()), // missing → appended
        ];
        upsert_dotenv(&env, &pairs).unwrap();
        let out = std::fs::read_to_string(&env).unwrap();

        // Unrelated lines preserved; existing keys rewritten in place; new appended.
        assert!(out.contains("APP_ENV=local"));
        assert!(out.contains("# comment"));
        assert!(out.contains("DB_HOST=127.0.0.1"));
        assert!(!out.contains("oldhost"));
        assert!(out.contains("DB_PASSWORD=newpw"));
        assert!(!out.contains("stale"));
        assert!(out.contains("DB_DATABASE=myapp_dev"));
        // Idempotent: a second run doesn't duplicate keys.
        upsert_dotenv(&env, &pairs).unwrap();
        let out2 = std::fs::read_to_string(&env).unwrap();
        assert_eq!(out2.matches("DB_DATABASE=").count(), 1);
    }

    #[test]
    fn allocate_port_avoids_registry_collisions() {
        let mut reg = Registry::new("test");
        // Claim the default redis port; allocator should move up.
        let inst = DatabaseInstance {
            id: DatabaseInstanceId::new("a"),
            name: "a".into(),
            engine: DatabaseEngine::Redis,
            version: "7".into(),
            port: 6379,
            data_dir: PathBuf::from("/x"),
            config_path: None,
            socket_path: None,
            auto_start: false,
            linked_projects: vec![],
        };
        reg.add_database(inst).unwrap();
        let p = allocate_port(&reg, DatabaseEngine::Redis);
        assert_ne!(p, 6379);
        assert!(p >= 6380);
    }
}
