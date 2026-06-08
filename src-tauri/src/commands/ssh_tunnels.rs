//! SSH tunnel commands: saved connections + port-forwards plus runtime
//! start/stop/test.
//!
//! As of registry v3 a port-forward (`SshTunnelConnection`) references a saved
//! host (`SshConnection`) by id instead of restating host + auth. The GUI still
//! sends host + auth inline on save; we find-or-create the connection behind it
//! transparently, so the frontend needs no change. Everything the spawn/command
//! builders touch is the resolved [`EffectiveSshTunnel`] (connection ⨝ tunnel).

use std::path::PathBuf;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::registry::{
    DatabaseEngine, DatabaseInstance, DatabaseInstanceId, Registry, SshAuthKind, SshConnection,
    SshConnectionId, SshForwardKind, SshProxyConfig, SshTunnelConnection, SshTunnelId,
};
use crate::ssh::backend::{test_connection, EffectiveSshTunnel, SshError};
use crate::ssh::{SshTunnelRuntimeStatus, SSH_STATE_CHANNEL};
use crate::state::AppState;

const SSH_KEYCHAIN_SERVICE: &str = "PortBay SSH";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSshTunnelInput {
    pub id: Option<String>,
    pub name: String,
    pub ssh_host: String,
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    pub ssh_user: String,
    #[serde(default)]
    pub auth_kind: SshAuthKind,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_local_host")]
    pub local_host: String,
    #[serde(default)]
    pub local_port: Option<u16>,
    pub remote_host: String,
    pub remote_port: u16,
    #[serde(default)]
    pub forward_kind: SshForwardKind,
    #[serde(default)]
    pub proxy_jump: Option<String>,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default)]
    pub auto_reconnect: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSshTunnelDatabaseInput {
    pub id: String,
    pub engine: String,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_local_host() -> String {
    "127.0.0.1".into()
}

use crate::ssh::resolve_tunnels as resolve_all;

/// Resolve a single tunnel by id into its [`EffectiveSshTunnel`].
fn resolve_one(registry: &Registry, id: &str) -> AppResult<EffectiveSshTunnel> {
    let tunnel = registry
        .get_ssh_tunnel(&SshTunnelId::new(id))
        .ok_or_else(|| AppError::BadInput(format!("SSH tunnel `{id}` not found")))?;
    let connection = registry
        .get_ssh_connection(&tunnel.connection_id)
        .ok_or_else(|| {
            AppError::BadInput(format!(
                "SSH tunnel `{id}` references a missing connection `{}`",
                tunnel.connection_id
            ))
        })?;
    Ok(EffectiveSshTunnel::resolve(tunnel, connection))
}

#[tauri::command]
pub async fn ssh_tunnel_list(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<SshTunnelRuntimeStatus>> {
    let registry = load_registry(&state)?;
    // `list` locks the manager and probes each child's liveness. That work is
    // cheap now that status() never reconnects inline (the supervisor owns
    // reconnection), but it's still a blocking mutex + syscalls, so keep it off
    // the async worker pool.
    let app_for_task = app.clone();
    let statuses = tokio::task::spawn_blocking(move || {
        let state: State<AppState> = app_for_task.state();
        let effectives = resolve_all(&registry);
        let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
        mgr.list(&effectives)
    })
    .await
    .map_err(|e| AppError::Internal(format!("SSH list task failed: {e}")))?;
    state.mirror_ssh_tunnels(&statuses);
    let _ = app.emit(SSH_STATE_CHANNEL, statuses.clone());
    Ok(statuses)
}

/// Background SSH reconnect supervisor. Wakes every `period`, attempts a
/// backed-off reconnect of any dropped auto-reconnect tunnel **off the async
/// runtime**, and re-mirrors + emits state only when something actually changed
/// — so it's silent while everything is healthy. Runs for the life of the app,
/// independent of whether the SSH page is open. Spawned once from `lib::run`.
pub fn spawn_ssh_supervisor(app: AppHandle, period: std::time::Duration) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(period).await;

            let app_for_task = app.clone();
            let refreshed = tokio::task::spawn_blocking(move || {
                let state: State<AppState> = app_for_task.state();
                let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
                if !mgr.reconnect_due() {
                    return None;
                }
                // A tunnel reconnected, started reconnecting, or gave up. Build
                // the full status list (saved + running) under the same lock so
                // the emit reflects the post-reconnect truth.
                let registry = load_registry(&state).ok()?;
                Some(mgr.list(&resolve_all(&registry)))
            })
            .await
            .ok()
            .flatten();

            if let Some(statuses) = refreshed {
                let state: State<AppState> = app.state();
                state.mirror_ssh_tunnels(&statuses);
                let _ = app.emit(SSH_STATE_CHANNEL, statuses);
            }
        }
    });
}

#[tauri::command]
pub async fn ssh_tunnel_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SaveSshTunnelInput,
) -> AppResult<SshTunnelRuntimeStatus> {
    let password = input
        .password
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let mut registry = load_registry(&state)?;
    let (connection, connection_is_new, tunnel) = build_tunnel_and_connection(&registry, input)?;

    // Password belongs to the connection (the owner of auth), keyed by its id.
    store_password_if_present(connection.auth_kind, &connection.id, password.as_deref())?;

    if connection_is_new {
        registry.add_ssh_connection(connection.clone())?;
    }
    if registry.get_ssh_tunnel(&tunnel.id).is_some() {
        registry.update_ssh_tunnel(tunnel.clone())?;
    } else {
        registry.add_ssh_tunnel(tunnel.clone())?;
    }
    save_registry(&state, &registry)?;

    // `list` is a blocking mutex + a `try_wait` syscall per tunnel — keep it
    // off the async worker pool, same as `ssh_tunnel_list` above. Locking the
    // manager directly here is the exact stall pattern that comment documents.
    let app_for_task = app.clone();
    let statuses = tokio::task::spawn_blocking(move || {
        let state: State<AppState> = app_for_task.state();
        let effectives = resolve_all(&registry);
        let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
        mgr.list(&effectives)
    })
    .await
    .map_err(|e| AppError::Internal(format!("SSH list task failed: {e}")))?;
    state.mirror_ssh_tunnels(&statuses);
    let _ = app.emit(SSH_STATE_CHANNEL, statuses.clone());
    statuses
        .into_iter()
        .find(|s| s.id == tunnel.id.as_str())
        .ok_or_else(|| AppError::Internal("saved SSH tunnel did not reappear".into()))
}

#[tauri::command]
pub async fn ssh_tunnel_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let tid = SshTunnelId::new(id.clone());
    // `stop` drops the tunnel (SIGTERM→SIGKILL chain) — blocking work, keep it
    // off the async worker pool.
    let app_for_task = app.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let state: State<AppState> = app_for_task.state();
        let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
        mgr.stop(&id)
    })
    .await;
    let mut registry = load_registry(&state)?;
    let removed = registry.remove_ssh_tunnel(&tid)?;
    // Drop the connection too once no other tunnel uses it, so transparently
    // created connections don't accumulate as orphans. (A stale keychain entry
    // is harmless and gets overwritten if the id is ever reused.)
    if !registry.ssh_connection_in_use(&removed.connection_id) {
        let _ = registry.remove_ssh_connection(&removed.connection_id);
    }
    save_registry(&state, &registry)?;
    let statuses = snapshot_statuses(&app).await?;
    state.mirror_ssh_tunnels(&statuses);
    let _ = app.emit(SSH_STATE_CHANNEL, statuses);
    Ok(())
}

#[tauri::command]
pub async fn ssh_tunnel_start(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    // One-shot password from the credential prompt; used for this start only and
    // never persisted. Blank/absent falls back to a keychain-saved password.
    password: Option<String>,
) -> AppResult<SshTunnelRuntimeStatus> {
    let registry = load_registry(&state)?;
    let effective = resolve_one(&registry, &id)?;
    let password = match password.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => Some(p.to_string()),
        None => load_password_if_needed(effective.auth_kind, &effective.connection_id)?,
    };

    let app_for_task = app.clone();
    let status = tokio::task::spawn_blocking(move || {
        let state: State<AppState> = app_for_task.state();
        let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
        mgr.start(effective, password)
    })
    .await
    .map_err(|e| AppError::Internal(format!("SSH start task failed: {e}")))??;

    let statuses = snapshot_statuses(&app).await?;
    state.mirror_ssh_tunnels(&statuses);
    let _ = app.emit(SSH_STATE_CHANNEL, statuses);
    Ok(status)
}

#[tauri::command]
pub async fn ssh_tunnel_stop(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    // `stop` drops the tunnel (SIGTERM→SIGKILL chain) — blocking work, keep it
    // off the async worker pool.
    let app_for_task = app.clone();
    tokio::task::spawn_blocking(move || {
        let state: State<AppState> = app_for_task.state();
        let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
        mgr.stop(&id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("SSH stop task failed: {e}")))??;
    let statuses = snapshot_statuses(&app).await?;
    state.mirror_ssh_tunnels(&statuses);
    let _ = app.emit(SSH_STATE_CHANNEL, statuses);
    Ok(())
}

#[tauri::command]
pub async fn ssh_tunnel_test(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let registry = load_registry(&state)?;
    let effective = resolve_one(&registry, &id)?;
    let password = load_password_if_needed(effective.auth_kind, &effective.connection_id)?;
    tokio::task::spawn_blocking(move || test_connection(&effective, password.as_deref()))
        .await
        .map_err(|e| AppError::Internal(format!("SSH test task failed: {e}")))?
        .map_err(AppError::Ssh)
}

#[tauri::command]
pub async fn ssh_tunnel_open_database(
    app: AppHandle,
    state: State<'_, AppState>,
    input: OpenSshTunnelDatabaseInput,
) -> AppResult<String> {
    let engine = DatabaseEngine::from_id(&input.engine)
        .ok_or_else(|| AppError::BadInput(format!("unknown database engine `{}`", input.engine)))?;
    if engine.is_file_based() {
        return Err(AppError::BadInput(
            "SSH database tunnels need a network database engine".into(),
        ));
    }

    let mut registry = load_registry(&state)?;
    let profile = registry
        .get_ssh_tunnel(&SshTunnelId::new(input.id.clone()))
        .ok_or_else(|| AppError::BadInput(format!("SSH tunnel `{}` not found", input.id)))?
        .clone();

    // The DB client connects to the tunnel's local port. If the tunnel isn't up
    // that's a connection-refused with no explanation — surface the real cause
    // and the fix instead.
    let profile_id = profile.id.as_str().to_string();
    let app_for_task = app.clone();
    let tunnel_up = tokio::task::spawn_blocking(move || {
        let state: State<AppState> = app_for_task.state();
        let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
        mgr.is_running(&profile_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("SSH status task failed: {e}")))?;
    if !tunnel_up {
        return Err(AppError::BadInput(format!(
            "Start the SSH tunnel `{}` before opening its database — it isn't connected yet.",
            profile.name
        )));
    }

    let db_id = DatabaseInstanceId::new(format!("ssh-{}", profile.id.as_str()));
    let instance = DatabaseInstance {
        id: db_id.clone(),
        name: format!("{} (via SSH)", profile.name),
        engine,
        version: "remote".into(),
        port: profile.local_port,
        data_dir: ssh_database_data_dir(&state, profile.id.as_str()),
        config_path: None,
        socket_path: None,
        file_path: None,
        auto_start: false,
        linked_projects: vec![],
    };

    if registry.get_database(&db_id).is_some() {
        registry.update_database(instance)?;
    } else {
        registry.add_database(instance)?;
    }
    save_registry(&state, &registry)?;
    crate::commands::databases::open_database_client(app, state, db_id.as_str().to_string())
        .await?;
    Ok(db_id.as_str().to_string())
}

/// Validate the save input and produce the connection (found or freshly built)
/// plus the tunnel that references it. The bool is `true` when the connection is
/// new and must be added to the registry.
fn build_tunnel_and_connection(
    registry: &Registry,
    input: SaveSshTunnelInput,
) -> AppResult<(SshConnection, bool, SshTunnelConnection)> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadInput("a tunnel name is required".into()));
    }
    let ssh_host = input.ssh_host.trim();
    if ssh_host.is_empty() {
        return Err(AppError::BadInput("an SSH host is required".into()));
    }
    let ssh_user = input.ssh_user.trim();
    if matches!(input.auth_kind, SshAuthKind::Password) && ssh_user.is_empty() {
        return Err(AppError::BadInput(
            "password SSH auth needs an SSH user; leave user blank only for OpenSSH Host aliases"
                .into(),
        ));
    }
    // The in-process russh path used for password auth only implements local
    // (-L) forwards. Catch the unsupported combination at save time rather than
    // letting it fail ~instantly on start with a cryptic message.
    if matches!(input.auth_kind, SshAuthKind::Password)
        && !matches!(input.forward_kind, SshForwardKind::Local)
    {
        return Err(AppError::BadInput(
            "password authentication supports local (-L) forwards only; use an SSH key for reverse or SOCKS tunnels".into(),
        ));
    }
    let remote_host = input.remote_host.trim();
    if remote_host.is_empty() && !matches!(input.forward_kind, SshForwardKind::Socks) {
        return Err(AppError::BadInput("a remote host is required".into()));
    }
    if input.remote_port == 0 && !matches!(input.forward_kind, SshForwardKind::Socks) {
        return Err(AppError::BadInput("a remote port is required".into()));
    }

    let key_path = input
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let proxy_jump = input
        .proxy_jump
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    // Fail fast on a typo'd key path: a non-existent `-i` file otherwise surfaces
    // as a generic ~10s start-time timeout. Only check paths we can resolve — a
    // leftover `~` (HOME unset) is left for ssh itself to expand.
    if let Some(kp) = key_path.as_deref() {
        let expanded = expand_tilde(kp);
        if !expanded.starts_with('~') && !std::path::Path::new(&expanded).exists() {
            return Err(AppError::BadInput(format!(
                "SSH key file not found at `{kp}`. Check the path, or leave it blank to use your SSH agent / config."
            )));
        }
    }

    let ssh_port = input.ssh_port.max(1);

    // Find-or-create the connection by its host + auth fingerprint, so two
    // forwards to the same box share one connection (and one keychain entry).
    let existing = registry.list_ssh_connections().iter().find(|c| {
        c.ssh_host == ssh_host
            && c.ssh_port == ssh_port
            && c.ssh_user == ssh_user
            && c.auth_kind == input.auth_kind
            && c.key_path.as_deref() == key_path.as_deref()
            && c.proxy_jump.as_deref() == proxy_jump.as_deref()
    });
    let (connection, connection_is_new) = match existing {
        Some(c) => (c.clone(), false),
        None => {
            let display = if ssh_user.is_empty() {
                ssh_host.to_string()
            } else {
                format!("{ssh_user}@{ssh_host}")
            };
            let connection = SshConnection {
                id: SshConnectionId::new(unique_connection_id(registry, &display)),
                name: display,
                ssh_host: ssh_host.into(),
                ssh_port,
                ssh_user: ssh_user.into(),
                auth_kind: input.auth_kind,
                key_path,
                proxy_jump,
                identity_id: None,
                proxy: None,
                metadata: Default::default(),
            };
            (connection, true)
        }
    };

    let tunnel_id = input
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| unique_tunnel_id(registry, name));

    let tunnel = SshTunnelConnection {
        id: SshTunnelId::new(tunnel_id),
        name: name.into(),
        connection_id: connection.id.clone(),
        local_host: if input.local_host.trim().is_empty() {
            default_local_host()
        } else {
            input.local_host.trim().into()
        },
        local_port: input
            .local_port
            .filter(|p| *p > 0)
            .unwrap_or_else(|| allocate_local_port(registry, input.remote_port.max(1024))),
        remote_host: if remote_host.is_empty() {
            "localhost".into()
        } else {
            remote_host.into()
        },
        remote_port: input.remote_port,
        forward_kind: input.forward_kind,
        keep_alive: input.keep_alive,
        auto_reconnect: input.auto_reconnect,
    };

    enforce_pro_gates(registry, &connection, &tunnel)?;
    Ok((connection, connection_is_new, tunnel))
}

/// Expand a leading `~/` to `$HOME/` so an existence check matches what OpenSSH
/// will actually open. Returns the path unchanged when there's no `~/` prefix or
/// `HOME` is unset.
fn expand_tilde(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var("HOME") {
            Ok(home) => format!("{home}/{rest}"),
            Err(_) => path.to_string(),
        },
        None => path.to_string(),
    }
}

fn enforce_pro_gates(
    registry: &Registry,
    connection: &SshConnection,
    tunnel: &SshTunnelConnection,
) -> AppResult<()> {
    if crate::entitlements::is_pro() {
        return Ok(());
    }

    let existing_tunnel = registry.get_ssh_tunnel(&tunnel.id);
    let existing_connection = registry.get_ssh_connection(&connection.id);
    let introduces = |enabled: bool, was_enabled: bool| enabled && !was_enabled;

    if introduces(
        tunnel.keep_alive,
        existing_tunnel.map(|p| p.keep_alive).unwrap_or(false),
    ) {
        return Err(AppError::ProRequired {
            feature: "SSH tunnel keep-alive",
        });
    }
    if introduces(
        tunnel.auto_reconnect,
        existing_tunnel.map(|p| p.auto_reconnect).unwrap_or(false),
    ) {
        return Err(AppError::ProRequired {
            feature: "SSH tunnel auto-reconnect",
        });
    }
    if matches!(tunnel.forward_kind, SshForwardKind::Reverse)
        && !matches!(
            existing_tunnel.map(|p| p.forward_kind),
            Some(SshForwardKind::Reverse)
        )
    {
        return Err(AppError::ProRequired {
            feature: "SSH reverse tunnels",
        });
    }
    if matches!(tunnel.forward_kind, SshForwardKind::Socks)
        && !matches!(
            existing_tunnel.map(|p| p.forward_kind),
            Some(SshForwardKind::Socks)
        )
    {
        return Err(AppError::ProRequired {
            feature: "SSH SOCKS proxy",
        });
    }
    if connection
        .proxy_jump
        .as_deref()
        .map(|s| s.split(',').filter(|hop| !hop.trim().is_empty()).count() > 1)
        .unwrap_or(false)
        && existing_connection.and_then(|c| c.proxy_jump.as_deref())
            != connection.proxy_jump.as_deref()
    {
        return Err(AppError::ProRequired {
            feature: "SSH multi-hop profiles",
        });
    }

    Ok(())
}

fn unique_tunnel_id(registry: &Registry, name: &str) -> String {
    let base = {
        let slug = crate::util::slugify(name);
        if slug.is_empty() {
            "ssh-tunnel".to_string()
        } else {
            slug
        }
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while registry
        .get_ssh_tunnel(&SshTunnelId::new(candidate.clone()))
        .is_some()
    {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

pub(crate) fn unique_connection_id(registry: &Registry, name: &str) -> String {
    let base = {
        let slug = crate::util::slugify(name);
        if slug.is_empty() {
            "ssh-connection".to_string()
        } else {
            slug
        }
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while registry
        .get_ssh_connection(&SshConnectionId::new(candidate.clone()))
        .is_some()
    {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

fn allocate_local_port(registry: &Registry, start: u16) -> u16 {
    let mut avoid: Vec<u16> = registry
        .list_databases()
        .iter()
        .map(|d| d.port)
        .chain(registry.list_ssh_tunnels().iter().map(|t| t.local_port))
        .chain(
            registry
                .list_projects()
                .iter()
                .flat_map(|p| p.port.into_iter().chain(p.extra_ports.iter().copied())),
        )
        .collect();
    avoid.retain(|p| *p > 0);
    crate::process_compose::lifecycle::find_free_port(start, 500, &avoid).unwrap_or(start)
}

pub(crate) fn store_password_if_present(
    auth_kind: SshAuthKind,
    connection_id: &SshConnectionId,
    password: Option<&str>,
) -> AppResult<()> {
    if !matches!(auth_kind, SshAuthKind::Password) {
        return Ok(());
    }
    let Some(password) = password else {
        return Ok(());
    };
    let entry = keyring::Entry::new(SSH_KEYCHAIN_SERVICE, connection_id.as_str())
        .map_err(|e| AppError::Internal(format!("couldn't open SSH keychain entry: {e}")))?;
    entry
        .set_password(password)
        .map_err(|e| AppError::Internal(format!("couldn't store SSH password in keychain: {e}")))?;
    Ok(())
}

pub(crate) fn load_password_if_needed(
    auth_kind: SshAuthKind,
    connection_id: &SshConnectionId,
) -> AppResult<Option<String>> {
    if !matches!(auth_kind, SshAuthKind::Password) {
        return Ok(None);
    }
    let entry = keyring::Entry::new(SSH_KEYCHAIN_SERVICE, connection_id.as_str())
        .map_err(|e| AppError::Internal(format!("couldn't open SSH keychain entry: {e}")))?;
    match entry.get_password() {
        Ok(password) if !password.trim().is_empty() => Ok(Some(password)),
        // No usable password stored (empty, missing, or unreadable): surface a
        // typed signal so the UI prompts for it (VS Code-style) rather than a
        // dead-end error. The connection id stands in as the host label; the
        // frontend already knows which host it acted on.
        Ok(_) | Err(keyring::Error::NoEntry) => Err(AppError::Ssh(SshError::MissingPassword {
            host: connection_id.as_str().to_string(),
        })),
        Err(e) => {
            tracing::debug!(error = %e, "SSH password keychain read failed; prompting for a password");
            Err(AppError::Ssh(SshError::MissingPassword {
                host: connection_id.as_str().to_string(),
            }))
        }
    }
}

/// Best-effort keychain password lookup, regardless of the connection's
/// declared `auth_kind`. Returns the stored password if present, or `None` when
/// absent. Unlike [`load_password_if_needed`] this never errors on a miss: the
/// in-process connect pipeline ([`crate::ssh::connect_session`]) treats a stored
/// password as one fallback among key/agent, so a missing entry must not block
/// the other methods. The strict "password but none stored" error stays the
/// pipeline's job when password is the only viable method.
/// Best-effort removal of a connection's stored keychain password (on delete).
/// A miss or backend error is not fatal — a stale entry is harmless and gets
/// overwritten if the id is ever reused.
pub(crate) fn clear_stored_password(connection_id: &SshConnectionId) {
    if let Ok(entry) = keyring::Entry::new(SSH_KEYCHAIN_SERVICE, connection_id.as_str()) {
        let _ = entry.delete_credential();
    }
}

pub(crate) fn load_stored_password(connection_id: &SshConnectionId) -> AppResult<Option<String>> {
    let entry = keyring::Entry::new(SSH_KEYCHAIN_SERVICE, connection_id.as_str())
        .map_err(|e| AppError::Internal(format!("couldn't open SSH keychain entry: {e}")))?;
    match entry.get_password() {
        Ok(password) if !password.trim().is_empty() => Ok(Some(password)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            // A locked/erroring keychain shouldn't sink an otherwise-valid key or
            // agent login; note it and let the pipeline proceed without a password.
            tracing::debug!(error = %e, "SSH keychain lookup failed; continuing without a stored password");
            Ok(None)
        }
    }
}

/// Keychain account for a connection's proxy password — the connection id
/// prefixed `proxy:`, so it never collides with the host-password entry (which
/// is keyed by the bare connection id) under the same service.
fn proxy_keychain_account(connection_id: &SshConnectionId) -> String {
    format!("proxy:{}", connection_id.as_str())
}

/// Persist or clear a connection's proxy password to match the saved proxy
/// config. An authenticated proxy (one with a username) keeps a non-blank
/// password, leaving an existing one untouched when `password` is blank
/// (blank-on-edit, like the host password). An open proxy or no proxy at all
/// clears any stored entry.
pub(crate) fn store_proxy_password(
    connection_id: &SshConnectionId,
    proxy: Option<&SshProxyConfig>,
    password: Option<&str>,
) -> AppResult<()> {
    let needs_auth = proxy.is_some_and(|p| {
        p.username
            .as_deref()
            .map(str::trim)
            .is_some_and(|u| !u.is_empty())
    });
    if !needs_auth {
        clear_stored_proxy_password(connection_id);
        return Ok(());
    }
    let Some(password) = password else {
        // Blank on edit — keep whatever is already stored.
        return Ok(());
    };
    let entry =
        keyring::Entry::new(SSH_KEYCHAIN_SERVICE, &proxy_keychain_account(connection_id))
            .map_err(|e| AppError::Internal(format!("couldn't open proxy keychain entry: {e}")))?;
    entry.set_password(password).map_err(|e| {
        AppError::Internal(format!("couldn't store proxy password in keychain: {e}"))
    })?;
    Ok(())
}

/// Best-effort removal of a connection's stored proxy password. A miss or
/// backend error is not fatal.
pub(crate) fn clear_stored_proxy_password(connection_id: &SshConnectionId) {
    if let Ok(entry) =
        keyring::Entry::new(SSH_KEYCHAIN_SERVICE, &proxy_keychain_account(connection_id))
    {
        let _ = entry.delete_credential();
    }
}

/// Best-effort proxy-password lookup, mirroring [`load_stored_password`]: a
/// missing or erroring entry yields `None` so an open proxy (or a transient
/// keychain failure) never blocks the connect path.
pub(crate) fn load_stored_proxy_password(
    connection_id: &SshConnectionId,
) -> AppResult<Option<String>> {
    let entry =
        keyring::Entry::new(SSH_KEYCHAIN_SERVICE, &proxy_keychain_account(connection_id))
            .map_err(|e| AppError::Internal(format!("couldn't open proxy keychain entry: {e}")))?;
    match entry.get_password() {
        Ok(password) if !password.trim().is_empty() => Ok(Some(password)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            tracing::debug!(error = %e, "proxy keychain lookup failed; continuing without a proxy password");
            Ok(None)
        }
    }
}

/// Keychain account for a connection's key passphrase — the connection id
/// prefixed `passphrase:`, so it never collides with the host- or proxy-password
/// entries under the same service.
fn passphrase_keychain_account(connection_id: &SshConnectionId) -> String {
    format!("passphrase:{}", connection_id.as_str())
}

/// Store a connection key's passphrase in the keychain (the "Remember" path of
/// the credential prompt). A blank passphrase clears any stored one instead.
pub(crate) fn store_key_passphrase(
    connection_id: &SshConnectionId,
    passphrase: &str,
) -> AppResult<()> {
    if passphrase.is_empty() {
        clear_stored_key_passphrase(connection_id);
        return Ok(());
    }
    let entry = keyring::Entry::new(
        SSH_KEYCHAIN_SERVICE,
        &passphrase_keychain_account(connection_id),
    )
    .map_err(|e| AppError::Internal(format!("couldn't open passphrase keychain entry: {e}")))?;
    entry.set_password(passphrase).map_err(|e| {
        AppError::Internal(format!("couldn't store key passphrase in keychain: {e}"))
    })?;
    Ok(())
}

/// Best-effort removal of a connection's stored key passphrase.
pub(crate) fn clear_stored_key_passphrase(connection_id: &SshConnectionId) {
    if let Ok(entry) = keyring::Entry::new(
        SSH_KEYCHAIN_SERVICE,
        &passphrase_keychain_account(connection_id),
    ) {
        let _ = entry.delete_credential();
    }
}

/// Best-effort key-passphrase lookup, mirroring [`load_stored_password`]: a
/// missing or erroring entry yields `None` so an unencrypted key (or a transient
/// keychain failure) never blocks the connect path.
pub(crate) fn load_stored_key_passphrase(
    connection_id: &SshConnectionId,
) -> AppResult<Option<String>> {
    let entry = keyring::Entry::new(
        SSH_KEYCHAIN_SERVICE,
        &passphrase_keychain_account(connection_id),
    )
    .map_err(|e| AppError::Internal(format!("couldn't open passphrase keychain entry: {e}")))?;
    match entry.get_password() {
        Ok(secret) if !secret.is_empty() => Ok(Some(secret)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            tracing::debug!(error = %e, "passphrase keychain lookup failed; continuing without it");
            Ok(None)
        }
    }
}

/// Lock the manager and rebuild the full status list **off the async worker
/// pool** — `list` is a blocking mutex plus a `try_wait` syscall per tunnel.
async fn snapshot_statuses(app: &AppHandle) -> AppResult<Vec<SshTunnelRuntimeStatus>> {
    let app_for_task = app.clone();
    tokio::task::spawn_blocking(move || {
        let state: State<AppState> = app_for_task.state();
        let registry = load_registry(&state)?;
        let effectives = resolve_all(&registry);
        let mut mgr = state.ssh_tunnels.lock().unwrap_or_else(|e| e.into_inner());
        Ok(mgr.list(&effectives))
    })
    .await
    .map_err(|e| AppError::Internal(format!("SSH snapshot task failed: {e}")))?
}

fn ssh_database_data_dir(state: &State<'_, AppState>, id: &str) -> PathBuf {
    state
        .logs_dir
        .parent()
        .unwrap_or(&state.logs_dir)
        .join("ssh-databases")
        .join(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;

    fn input(name: &str, host: &str, user: &str, auth: SshAuthKind) -> SaveSshTunnelInput {
        SaveSshTunnelInput {
            id: None,
            name: name.into(),
            ssh_host: host.into(),
            ssh_port: 22,
            ssh_user: user.into(),
            auth_kind: auth,
            key_path: None,
            password: None,
            local_host: "127.0.0.1".into(),
            local_port: Some(15432),
            remote_host: "localhost".into(),
            remote_port: 5432,
            forward_kind: SshForwardKind::Local,
            proxy_jump: None,
            keep_alive: false,
            auto_reconnect: false,
        }
    }

    #[test]
    fn build_extracts_connection_and_does_not_store_password() {
        let reg = Registry::new("test");
        let mut i = input("Production DB", "host", "deploy", SshAuthKind::Password);
        i.password = Some("secret".into());
        let (connection, is_new, tunnel) = build_tunnel_and_connection(&reg, i).unwrap();

        assert!(is_new);
        assert_eq!(tunnel.id, SshTunnelId::new("production-db"));
        assert_eq!(tunnel.connection_id, connection.id);
        assert_eq!(connection.ssh_host, "host");
        assert_eq!(connection.ssh_user, "deploy");
        // No secret material lands in either registry object.
        let json = format!(
            "{}{}",
            serde_json::to_string(&connection).unwrap(),
            serde_json::to_string(&tunnel).unwrap()
        );
        assert!(!json.contains("secret"));
    }

    #[test]
    fn build_accepts_openssh_host_alias_without_user() {
        let reg = Registry::new("test");
        let (connection, _, _) = build_tunnel_and_connection(
            &reg,
            input("Teleport Prod", "teleport-prod", " ", SshAuthKind::Key),
        )
        .unwrap();
        assert_eq!(connection.ssh_user, "");
        assert_eq!(connection.ssh_host, "teleport-prod");
    }

    #[test]
    fn password_auth_requires_user_because_it_bypasses_ssh_config() {
        let reg = Registry::new("test");
        let err = build_tunnel_and_connection(
            &reg,
            input("Shared", "shared-host", "", SshAuthKind::Password),
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("password SSH auth needs an SSH user"));
    }

    #[test]
    fn two_forwards_to_same_host_share_one_connection() {
        let mut reg = Registry::new("test");
        let (c1, new1, t1) =
            build_tunnel_and_connection(&reg, input("DB", "host", "me", SshAuthKind::Key)).unwrap();
        assert!(new1);
        reg.add_ssh_connection(c1.clone()).unwrap();
        reg.add_ssh_tunnel(t1).unwrap();

        let (c2, new2, _) =
            build_tunnel_and_connection(&reg, input("Cache", "host", "me", SshAuthKind::Key))
                .unwrap();
        assert!(!new2, "identical host+auth should reuse the connection");
        assert_eq!(c1.id, c2.id);
    }

    #[test]
    fn allocated_port_avoids_saved_profiles() {
        let mut reg = Registry::new("test");
        let mut i = input("A", "host", "me", SshAuthKind::Key);
        i.id = Some("a".into());
        let (connection, _, tunnel) = build_tunnel_and_connection(&reg, i).unwrap();
        reg.add_ssh_connection(connection).unwrap();
        reg.add_ssh_tunnel(tunnel).unwrap();
        let next = allocate_local_port(&reg, 15432);
        assert_ne!(next, 15432);
    }

    // ── build_tunnel_and_connection validation ────────────────────────────────

    #[test]
    fn empty_name_is_rejected() {
        let reg = Registry::new("test");
        let err = build_tunnel_and_connection(&reg, input("", "host", "me", SshAuthKind::Key))
            .unwrap_err();
        assert!(err.to_string().contains("tunnel name is required"), "{err}");
    }

    #[test]
    fn blank_whitespace_name_is_rejected() {
        let reg = Registry::new("test");
        let err = build_tunnel_and_connection(&reg, input("   ", "host", "me", SshAuthKind::Key))
            .unwrap_err();
        assert!(err.to_string().contains("tunnel name is required"), "{err}");
    }

    #[test]
    fn empty_host_is_rejected() {
        let reg = Registry::new("test");
        let err = build_tunnel_and_connection(&reg, input("Tunnel", "", "me", SshAuthKind::Key))
            .unwrap_err();
        assert!(err.to_string().contains("SSH host is required"), "{err}");
    }

    #[test]
    fn password_auth_with_non_local_forward_is_rejected() {
        let reg = Registry::new("test");
        let mut i = input("Reverse Tunnel", "host", "me", SshAuthKind::Password);
        i.forward_kind = SshForwardKind::Reverse;
        let err = build_tunnel_and_connection(&reg, i).unwrap_err();
        assert!(
            err.to_string()
                .contains("password authentication supports local"),
            "{err}"
        );
    }

    #[test]
    fn password_auth_with_socks_forward_is_rejected() {
        let reg = Registry::new("test");
        let mut i = input("SOCKS Tunnel", "host", "me", SshAuthKind::Password);
        i.forward_kind = SshForwardKind::Socks;
        let err = build_tunnel_and_connection(&reg, i).unwrap_err();
        assert!(
            err.to_string()
                .contains("password authentication supports local"),
            "{err}"
        );
    }

    #[test]
    fn remote_host_required_for_non_socks() {
        let reg = Registry::new("test");
        let mut i = input("Fwd", "host", "me", SshAuthKind::Key);
        i.remote_host = "".into();
        i.forward_kind = SshForwardKind::Local;
        let err = build_tunnel_and_connection(&reg, i).unwrap_err();
        assert!(err.to_string().contains("remote host is required"), "{err}");
    }

    #[test]
    fn socks_does_not_require_remote_host() {
        let reg = Registry::new("test");
        let mut i = input("SOCKS", "host", "me", SshAuthKind::Key);
        i.remote_host = "".into();
        i.forward_kind = SshForwardKind::Socks;
        // SOCKS forwards don't forward to a single remote; empty host is fine.
        assert!(build_tunnel_and_connection(&reg, i).is_ok());
    }

    #[test]
    fn remote_port_zero_is_rejected_for_local_forward() {
        let reg = Registry::new("test");
        let mut i = input("Fwd", "host", "me", SshAuthKind::Key);
        i.remote_port = 0;
        i.forward_kind = SshForwardKind::Local;
        let err = build_tunnel_and_connection(&reg, i).unwrap_err();
        assert!(err.to_string().contains("remote port is required"), "{err}");
    }

    #[test]
    fn tunnel_id_collision_appends_counter() {
        let mut reg = Registry::new("test");
        let (c1, _, t1) =
            build_tunnel_and_connection(&reg, input("My Tunnel", "h", "u", SshAuthKind::Key))
                .unwrap();
        let t1_id = t1.id.clone();
        reg.add_ssh_connection(c1).unwrap();
        reg.add_ssh_tunnel(t1).unwrap();

        // Second tunnel with the same name gets a suffixed id.
        let (c2, _, t2) =
            build_tunnel_and_connection(&reg, input("My Tunnel", "h2", "u", SshAuthKind::Key))
                .unwrap();
        reg.add_ssh_connection(c2).unwrap();
        // t1.id = "my-tunnel", t2.id must be different (e.g. "my-tunnel-2").
        assert_ne!(t1_id, t2.id);
        assert!(
            t2.id.as_str().contains("my-tunnel"),
            "id should still be based on slug: {}",
            t2.id
        );
    }

    #[test]
    fn connection_id_collision_appends_counter() {
        let mut reg = Registry::new("test");
        // Build the same connection twice (different tunnel name, same host+user).
        let (c1, is_new1, t1) =
            build_tunnel_and_connection(&reg, input("Alpha", "myhost", "user", SshAuthKind::Key))
                .unwrap();
        assert!(is_new1);
        reg.add_ssh_connection(c1.clone()).unwrap();
        reg.add_ssh_tunnel(t1).unwrap();

        // Changing auth_kind forces a *new* connection (different fingerprint).
        // We use a different host to also force a different slug base so we can
        // test the counter independently.
        let slug = unique_connection_id(&reg, "user@myhost");
        // The base "user-myhost" or similar isn't taken; adding a second with the same
        // display name forces a collision suffix.
        let slug2 = unique_connection_id(&reg, "user@myhost");
        // They shouldn't collide — unique_connection_id is idempotent if the
        // first slug was never inserted.
        assert_eq!(slug, slug2);

        // Now insert the first slug and verify the second call generates a different one.
        let id1 = SshConnectionId::new(slug.clone());
        // Synthesise a minimal connection at that id to pollute the registry.
        let dummy = crate::registry::SshConnection {
            id: id1,
            name: "dummy".into(),
            ssh_host: "dummy".into(),
            ssh_port: 22,
            ssh_user: "dummy".into(),
            auth_kind: SshAuthKind::Key,
            key_path: None,
            proxy_jump: None,
            identity_id: None,
            proxy: None,
            metadata: Default::default(),
        };
        reg.add_ssh_connection(dummy).unwrap();
        let slug3 = unique_connection_id(&reg, "user@myhost");
        assert_ne!(slug, slug3, "collision suffix must be appended");
        assert!(
            slug3.contains(&slug) || slug3.contains("user"),
            "slug3 should be related to original slug: {slug3}"
        );
    }

    // ── expand_tilde ──────────────────────────────────────────────────────────

    #[test]
    fn expand_tilde_replaces_home_prefix_when_home_is_set() {
        // Read the actual HOME rather than calling set_var (set_var is not
        // safe in multi-threaded test runs and leaks global state).
        if let Ok(home) = std::env::var("HOME") {
            let expanded = expand_tilde("~/.ssh/id_rsa");
            let expected = format!("{home}/.ssh/id_rsa");
            assert_eq!(expanded, expected, "tilde prefix must expand to $HOME");
        }
        // If HOME is unset the function leaves the path unchanged; we can't
        // assert the positive case but also can't fail here.
    }

    #[test]
    fn expand_tilde_result_contains_rest_of_path() {
        // Platform-neutral: whatever HOME is, the rest of the path is preserved.
        let expanded = expand_tilde("~/.ssh/id_rsa");
        assert!(
            expanded.ends_with("/.ssh/id_rsa"),
            "suffix must be preserved: {expanded}"
        );
    }

    #[test]
    fn expand_tilde_leaves_absolute_path_unchanged() {
        let expanded = expand_tilde("/etc/ssh/id_rsa");
        assert_eq!(expanded, "/etc/ssh/id_rsa");
    }

    #[test]
    fn expand_tilde_leaves_relative_path_unchanged() {
        let expanded = expand_tilde("relative/path");
        assert_eq!(expanded, "relative/path");
    }

    #[test]
    fn expand_tilde_only_expands_leading_tilde_slash() {
        // A tilde not at the start must be left alone.
        let expanded = expand_tilde("/path/with~/file");
        assert_eq!(expanded, "/path/with~/file");
    }

    #[test]
    fn expand_tilde_bare_tilde_without_slash_is_unchanged() {
        // Just "~" alone (no slash) is not a home-relative path.
        let expanded = expand_tilde("~");
        assert_eq!(expanded, "~");
    }

    // ── proxy_keychain_account / passphrase_keychain_account isolation ────────

    #[test]
    fn proxy_and_host_keychain_accounts_are_distinct() {
        let id = SshConnectionId::new("my-server");
        let proxy_account = proxy_keychain_account(&id);
        // The proxy account must carry a distinguishing prefix so it doesn't
        // shadow the bare host-password entry.
        assert!(
            proxy_account.starts_with("proxy:"),
            "proxy account: {proxy_account}"
        );
        assert!(proxy_account.contains("my-server"));
        // And it must differ from the bare connection id string.
        assert_ne!(proxy_account, id.as_str());
    }

    #[test]
    fn passphrase_and_host_keychain_accounts_are_distinct() {
        let id = SshConnectionId::new("my-server");
        let pp_account = passphrase_keychain_account(&id);
        assert!(
            pp_account.starts_with("passphrase:"),
            "passphrase account: {pp_account}"
        );
        assert!(pp_account.contains("my-server"));
        assert_ne!(pp_account, id.as_str());
    }

    #[test]
    fn all_three_keychain_accounts_are_distinct_from_each_other() {
        let id = SshConnectionId::new("box");
        let host = id.as_str().to_string();
        let proxy = proxy_keychain_account(&id);
        let pp = passphrase_keychain_account(&id);
        assert_ne!(host, proxy);
        assert_ne!(host, pp);
        assert_ne!(proxy, pp);
    }
}
