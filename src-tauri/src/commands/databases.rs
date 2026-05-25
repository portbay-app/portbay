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

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_shell::ShellExt;

use crate::commands::projects::{load_registry, save_registry, slugify};
use crate::databases as engine;
use crate::error::{AppError, AppResult};
use crate::registry::{DatabaseEngine, DatabaseInstance, DatabaseInstanceId, ProjectId, Registry};
use crate::state::AppState;

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

fn install_formula(e: DatabaseEngine) -> &'static str {
    match e {
        DatabaseEngine::Mysql => "mysql",
        DatabaseEngine::Postgres => "postgresql@16",
        DatabaseEngine::Mariadb => "mariadb",
        DatabaseEngine::Redis => "redis",
        DatabaseEngine::Mongo => "mongodb-community",
        DatabaseEngine::Memcached => "memcached",
    }
}

/// `list_database_engines()` — every supported engine, with install state.
#[tauri::command]
pub async fn list_database_engines() -> AppResult<Vec<DatabaseEngineView>> {
    Ok(ALL_ENGINES
        .iter()
        .map(|&e| {
            let det = engine::detect(e);
            DatabaseEngineView {
                id: e.id().into(),
                label: e.label().into(),
                installed: det.installed,
                version: det.version,
                default_port: e.default_port(),
                client_available: det.client.is_some(),
                install_hint: install_hint(e).into(),
            }
        })
        .collect())
}

/// `install_database_engine(engine)` — install the engine binary via brew.
#[tauri::command]
pub async fn install_database_engine(engine: String) -> AppResult<()> {
    let eng = parse_engine(&engine)?;
    let brew = require_brew()?;
    let formula = install_formula(eng);
    // `brew install` can run for minutes — never on the async runtime.
    tokio::task::spawn_blocking(move || {
        engine::run_capture(&brew, &["install", formula], Duration::from_secs(8 * 60))
    })
    .await
    .map_err(|e| AppError::Internal(format!("install join: {e}")))?
    .map(|_| ())
    .map_err(|e| {
        AppError::Internal(format!(
            "brew install {formula} failed: {}",
            truncate(&e, 600)
        ))
    })
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
            instance_view(inst, status, &app_data)
        })
        .collect();
    Ok(views)
}

fn instance_view(
    inst: &DatabaseInstance,
    status: InstanceStatus,
    app_data: &std::path::Path,
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
        binary_available: engine::daemon_binary(inst.engine).is_some(),
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

    // Daemon must be installed before we can provision.
    let daemon = engine::daemon_binary(eng).ok_or_else(|| {
        AppError::BadInput(format!(
            "{} isn't installed. Install it first ({}).",
            eng.label(),
            install_hint(eng)
        ))
    })?;

    let mut registry = load_registry(&state)?;
    let id = unique_instance_id(&registry, name);
    let app_data = app_data_dir(&state)?;

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
    let detection = engine::detect(eng);
    let provision_data = app_data.clone();
    let provision_id = id.clone();
    tokio::task::spawn_blocking(move || {
        engine::provision(eng, &daemon, &provision_data, &provision_id, port)
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

    Ok(instance_view(&instance, InstanceStatus::Stopped, &app_data))
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
    let client = engine::client_binary(inst.engine).ok_or_else(|| {
        AppError::BadInput(format!("no CLI client for {} found.", inst.engine.label()))
    })?;
    let command = engine::client_invocation(inst, &client);
    open_in_terminal(&app, &command).await
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

fn require_brew() -> AppResult<PathBuf> {
    which::which("brew").map_err(|_| {
        AppError::BadInput(
            "Homebrew isn't installed. Install from https://brew.sh, then restart PortBay.".into(),
        )
    })
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

fn truncate(s: &str, limit: usize) -> &str {
    if s.len() <= limit {
        s
    } else {
        &s[..limit]
    }
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
            assert!(!install_formula(*engine).is_empty());
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
