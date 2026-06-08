//! Connection-management commands behind the SSH **connections dashboard**.
//!
//! The dashboard is the front door to the Phase 1 connection model: list saved
//! hosts (ordered by last use), add/edit/delete them directly (not only as a
//! side effect of saving a tunnel), detect the remote OS, and stamp last-used.
//! Auth stays exactly as elsewhere — passwords live in the OS keychain keyed by
//! connection id; key/agent details live on the connection.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::projects::{load_registry, save_registry};
use crate::commands::ssh_tunnels::{
    clear_stored_key_passphrase, clear_stored_password, clear_stored_proxy_password,
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
    store_key_passphrase, store_password_if_present, store_proxy_password, unique_connection_id,
};
use crate::error::{AppError, AppResult};
use crate::registry::{
    Registry, SshAuthKind, SshConnection, SshConnectionId, SshConnectionMeta, SshIdentityId,
    SshProxyConfig,
};
use crate::ssh::config_import::{parse_ssh_config, SshConfigCandidate};
use crate::ssh::exec::run_command;
use crate::ssh::interaction::{EventInteractor, SshInteractor};
use crate::ssh::probe::{probe_connection, ProbeResult};
use crate::state::AppState;

/// A saved connection plus the two derived facts the dashboard renders per host.
/// `#[serde(flatten)]` keeps the connection fields at the top level so the
/// frontend reads `view.sshHost` etc. directly.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectionView {
    #[serde(flatten)]
    pub connection: SshConnection,
    /// How many saved tunnels reference this host.
    pub tunnel_count: usize,
    /// Whether any of those tunnels exist (delete is blocked while true).
    pub in_use: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSshConnectionInput {
    /// Existing connection id to update; absent/blank creates a new one.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub ssh_host: String,
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    #[serde(default)]
    pub ssh_user: String,
    #[serde(default)]
    pub auth_kind: SshAuthKind,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub proxy_jump: Option<String>,
    /// Reusable identity to borrow user / key / auth from (blank = none).
    #[serde(default)]
    pub identity_id: Option<String>,
    /// Optional forward proxy (SOCKS5 / HTTP CONNECT). Absent / blank host = none.
    #[serde(default)]
    pub proxy: Option<SshProxyConfig>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Manual environment override (e.g. `cpanel`, `ubuntu`, `aws`). Blank /
    /// `auto` leaves the detected/existing value in place.
    #[serde(default)]
    pub environment: Option<String>,
    /// Deployment tier for the dashboard's Environment column
    /// (`production` / `staging` / `research` / `sandbox`). Blank = none.
    #[serde(default)]
    pub stage: Option<String>,
    /// Provider region label (`us-east-1`, `nyc3`, …). Blank = none.
    #[serde(default)]
    pub region: Option<String>,
    /// Password to store in the keychain (password auth only). Blank leaves any
    /// existing stored password untouched.
    #[serde(default)]
    pub password: Option<String>,
    /// Proxy password to store in the keychain (authenticated proxy only).
    /// Blank leaves any existing one untouched, mirroring `password`.
    #[serde(default)]
    pub proxy_password: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn trimmed_opt(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

/// Normalise a proxy from the form: trim the host, drop the whole proxy when
/// the host is blank, and blank an empty username down to `None` (an open
/// proxy). Port is left as submitted (the form constrains it to a `u16`).
fn clean_proxy(proxy: Option<SshProxyConfig>) -> Option<SshProxyConfig> {
    let mut proxy = proxy?;
    proxy.host = proxy.host.trim().to_string();
    if proxy.host.is_empty() {
        return None;
    }
    proxy.username = proxy
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    Some(proxy)
}

fn clean_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !out.iter().any(|t| t == tag) {
            out.push(tag.to_string());
        }
    }
    out
}

fn view_of(registry: &Registry, connection: SshConnection) -> SshConnectionView {
    let tunnel_count = registry
        .list_ssh_tunnels()
        .iter()
        .filter(|t| t.connection_id == connection.id)
        .count();
    SshConnectionView {
        in_use: tunnel_count > 0,
        tunnel_count,
        connection,
    }
}

/// List saved connections, newest-used first, each with its tunnel count.
#[tauri::command]
pub async fn ssh_connections_list(state: State<'_, AppState>) -> AppResult<Vec<SshConnectionView>> {
    let registry = load_registry(&state)?;
    let mut views: Vec<SshConnectionView> = registry
        .list_ssh_connections()
        .iter()
        .cloned()
        .map(|c| view_of(&registry, c))
        .collect();
    sort_views(&mut views);
    Ok(views)
}

/// Most-recently-used first; never-used (`None`) sink to the bottom, ties broken
/// by case-insensitive name. Extracted so the ordering is unit-testable.
fn sort_views(views: &mut [SshConnectionView]) {
    views.sort_by(|a, b| {
        b.connection
            .metadata
            .last_used
            .cmp(&a.connection.metadata.last_used)
            .then_with(|| {
                a.connection
                    .name
                    .to_lowercase()
                    .cmp(&b.connection.name.to_lowercase())
            })
    });
}

/// Create or update a saved connection (host + auth + display metadata).
#[tauri::command]
pub async fn ssh_connection_save(
    state: State<'_, AppState>,
    input: SaveSshConnectionInput,
) -> AppResult<SshConnectionView> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadInput("a connection name is required".into()));
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

    let mut registry = load_registry(&state)?;
    let id = match input.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(existing) => SshConnectionId::new(existing),
        None => SshConnectionId::new(unique_connection_id(&registry, name)),
    };
    let existing = registry.get_ssh_connection(&id).cloned();

    let connection = SshConnection {
        id: id.clone(),
        name: name.to_string(),
        ssh_host: ssh_host.to_string(),
        ssh_port: input.ssh_port.max(1),
        ssh_user: ssh_user.to_string(),
        auth_kind: input.auth_kind,
        key_path: trimmed_opt(input.key_path),
        proxy_jump: trimmed_opt(input.proxy_jump),
        identity_id: trimmed_opt(input.identity_id).map(SshIdentityId::new),
        proxy: clean_proxy(input.proxy),
        metadata: SshConnectionMeta {
            tags: clean_tags(input.tags),
            color: trimmed_opt(input.color),
            notes: trimmed_opt(input.notes),
            // Caches survive an edit — they're not user-entered on the form.
            detected_os: existing
                .as_ref()
                .and_then(|c| c.metadata.detected_os.clone()),
            // A manual pick wins; a blank/"auto" submission preserves the
            // detected (or previously-set) environment rather than clearing it.
            environment: trimmed_opt(input.environment)
                .filter(|e| e != "auto")
                .or_else(|| {
                    existing
                        .as_ref()
                        .and_then(|c| c.metadata.environment.clone())
                }),
            stage: trimmed_opt(input.stage),
            region: trimmed_opt(input.region),
            // Detection-only (DMI vendor), not a form field — preserve across edits.
            provider: existing.as_ref().and_then(|c| c.metadata.provider.clone()),
            // Stamped once, on first save; preserved verbatim across edits.
            created_at: existing
                .as_ref()
                .and_then(|c| c.metadata.created_at)
                .or_else(|| Some(now_epoch_secs())),
            last_used: existing.as_ref().and_then(|c| c.metadata.last_used),
        },
    };

    // A blank password on edit must not wipe a stored one.
    store_password_if_present(
        connection.auth_kind,
        &connection.id,
        trimmed_opt(input.password).as_deref(),
    )?;
    // Store/clear the proxy password to match the saved proxy config (same
    // blank-on-edit semantics; an open proxy or no proxy clears the entry).
    store_proxy_password(
        &connection.id,
        connection.proxy.as_ref(),
        trimmed_opt(input.proxy_password).as_deref(),
    )?;

    if existing.is_some() {
        registry
            .update_ssh_connection(connection.clone())
            .map_err(AppError::Registry)?;
    } else {
        registry
            .add_ssh_connection(connection.clone())
            .map_err(AppError::Registry)?;
    }
    save_registry(&state, &registry)?;
    Ok(view_of(&registry, connection))
}

/// Delete a saved connection. Refuses while any tunnel still references it, so a
/// live forward never loses its host out from under it.
///
/// By contract this only removes PortBay's own state — the registry row and the
/// connection's keychain secrets (host + proxy passwords). It never edits
/// `~/.ssh/config`, so a host imported from there (VS Code / Cursor / OpenSSH)
/// keeps its source entry; "remove from PortBay" is not "delete everywhere".
#[tauri::command]
pub async fn ssh_connection_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let id = SshConnectionId::new(id);
    let mut registry = load_registry(&state)?;
    if registry.ssh_connection_in_use(&id) {
        return Err(AppError::BadInput(
            "this SSH host still has saved tunnels — delete those first".into(),
        ));
    }
    registry
        .remove_ssh_connection(&id)
        .map_err(AppError::Registry)?;
    save_registry(&state, &registry)?;
    clear_stored_password(&id);
    clear_stored_proxy_password(&id);
    clear_stored_key_passphrase(&id);
    Ok(())
}

/// Detect the remote OS (`uname -srm`, falling back to `/etc/os-release`),
/// cache it on the connection, and stamp last-used. Returns the OS string.
#[tauri::command]
pub async fn ssh_connection_detect_os(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    // One-shot secrets from the credential prompt; used for this connect only
    // and never persisted. Blank/absent falls back to the keychain.
    password: Option<String>,
    passphrase: Option<String>,
) -> AppResult<String> {
    let conn = {
        let registry = load_registry(&state)?;
        let raw = registry
            .get_ssh_connection(&SshConnectionId::new(&id))
            .ok_or_else(|| AppError::BadInput(format!("SSH connection `{id}` not found")))?;
        // Fold in a borrowed identity (user / key / auth) before connecting.
        registry.effective_ssh_connection(raw)
    };
    let nonblank = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let password = match nonblank(password) {
        Some(p) => Some(p),
        None => load_stored_password(&conn.id)?,
    };
    let passphrase = match nonblank(passphrase) {
        Some(p) => Some(p),
        None => load_stored_key_passphrase(&conn.id)?,
    };
    let proxy_password = load_stored_proxy_password(&conn.id)?;

    let interactor: Option<Arc<dyn SshInteractor>> = Some(EventInteractor::shared(app));
    let os = detect_os_string(
        &conn,
        password.as_deref(),
        proxy_password.as_deref(),
        passphrase.as_deref(),
        interactor,
    )
    .await?;
    // Best-effort: classify the environment for the host's brand mark. A probe
    // failure or an unknown ("generic") result must not fail OS detection, and
    // must not wipe a value the user set manually for an undetectable host.
    let environment = detect_environment(
        &conn,
        password.as_deref(),
        proxy_password.as_deref(),
        passphrase.as_deref(),
    )
    .await;
    // Best-effort: the real cloud provider + region (DMI vendor → cloud metadata
    // endpoint), captured separately from the control-panel/distro brand so a
    // cPanel box on AWS still shows "AWS · us-east-1". Same non-fatal contract.
    let (provider, region) = detect_provider_region(
        &conn,
        password.as_deref(),
        proxy_password.as_deref(),
        passphrase.as_deref(),
    )
    .await;
    touch_metadata(&state, &conn.id, |m| {
        m.detected_os = Some(os.clone());
        if let Some(env) = environment.as_deref() {
            if env != "generic" || m.environment.is_none() {
                m.environment = Some(env.to_string());
            }
        }
        // A detected provider/region is authoritative; only write it when found
        // so an undetectable host keeps any value already on record.
        if let Some(p) = provider.as_deref() {
            if p != "generic" {
                m.provider = Some(p.to_string());
            }
        }
        if let Some(r) = region.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            m.region = Some(r.to_string());
        }
        m.last_used = Some(now_epoch_secs());
    })?;
    Ok(os)
}

/// Probe a host's reachability, latency, host-key fingerprint, and trust with a
/// single unauthenticated handshake — the data behind the dashboard's Health
/// column and the detail panel's fingerprint / Host Trust card. Read-only: it
/// sends no credentials and never touches `known_hosts`.
#[tauri::command]
pub async fn ssh_connection_probe(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<ProbeResult> {
    let conn = {
        let registry = load_registry(&state)?;
        let raw = registry
            .get_ssh_connection(&SshConnectionId::new(&id))
            .ok_or_else(|| AppError::BadInput(format!("SSH connection `{id}` not found")))?;
        registry.effective_ssh_connection(raw)
    };
    Ok(probe_connection(&conn).await)
}

/// Which secret the credential prompt is persisting.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshCredentialKind {
    /// The host login password (keyed by connection id).
    Password,
    /// The connection key's passphrase (keyed `passphrase:<id>`).
    Passphrase,
}

/// Persist a credential the user entered in the VS Code-style prompt, so the
/// retried connect — and future ones, if they chose "Remember" — find it in the
/// keychain. Paired with [`ssh_clear_credential`] for the "don't remember" path.
#[tauri::command]
pub async fn ssh_set_credential(
    id: String,
    kind: SshCredentialKind,
    secret: String,
) -> AppResult<()> {
    let cid = SshConnectionId::new(id);
    match kind {
        // Force the password branch — the prompt only fires for password-auth
        // hosts, and the connection is already that kind.
        SshCredentialKind::Password => {
            store_password_if_present(SshAuthKind::Password, &cid, Some(secret.as_str()))?
        }
        SshCredentialKind::Passphrase => store_key_passphrase(&cid, &secret)?,
    }
    Ok(())
}

/// Drop a credential persisted by [`ssh_set_credential`] — used after a
/// connect when the user did not choose "Remember", so the secret lives only
/// for that one attempt.
#[tauri::command]
pub async fn ssh_clear_credential(id: String, kind: SshCredentialKind) -> AppResult<()> {
    let cid = SshConnectionId::new(id);
    match kind {
        SshCredentialKind::Password => clear_stored_password(&cid),
        SshCredentialKind::Passphrase => clear_stored_key_passphrase(&cid),
    }
    Ok(())
}

/// Whether a password or key passphrase is saved in the OS keychain for this
/// connection — drives the host panel's "Forget saved secret" affordance, so it
/// only shows when there is something to forget.
#[tauri::command]
pub async fn ssh_has_stored_credential(id: String) -> AppResult<bool> {
    let cid = SshConnectionId::new(id);
    Ok(load_stored_password(&cid)?.is_some() || load_stored_key_passphrase(&cid)?.is_some())
}

/// Forget every prompt-saved secret (password + key passphrase) for this
/// connection — the host panel's one-click "Forget saved secret". The next
/// connect re-prompts. Proxy passwords are managed separately and untouched.
#[tauri::command]
pub async fn ssh_forget_credentials(id: String) -> AppResult<()> {
    let cid = SshConnectionId::new(id);
    clear_stored_password(&cid);
    clear_stored_key_passphrase(&cid);
    Ok(())
}

/// Stamp a connection as just-used (for dashboard ordering) when the user opens
/// it. Cheap and idempotent; a missing connection is a no-op.
#[tauri::command]
pub async fn ssh_connection_touch(state: State<'_, AppState>, id: String) -> AppResult<()> {
    touch_metadata(&state, &SshConnectionId::new(id), |m| {
        m.last_used = Some(now_epoch_secs());
    })
}

/// Tear down every live session for a host in one shot — cached exec/deploy,
/// SFTP, agent, and any open terminal shells. This is the explicit "log out of
/// this host" action: after it returns, nothing holds an authenticated
/// connection, and the next action re-authenticates (silently from a stored
/// credential, or with a one-shot prompt otherwise).
#[tauri::command]
pub async fn ssh_host_disconnect(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.exec.lock().await.disconnect(&id);
    state.sftp.lock().await.disconnect(&id);
    state.agent.lock().await.disconnect(&id);
    state.pty.lock().await.disconnect_connection(&id);
    Ok(())
}

/// Whether any authenticated session (exec, SFTP, agent, or a terminal shell)
/// is currently open to this host. Read-only: never connects, and the checks
/// don't bump idle timers, so polling this can't keep a session alive.
#[tauri::command]
pub async fn ssh_host_connected(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    if state.exec.lock().await.has_session(&id) {
        return Ok(true);
    }
    if state.sftp.lock().await.has_session(&id) {
        return Ok(true);
    }
    if state.agent.lock().await.has_session(&id) {
        return Ok(true);
    }
    Ok(state.pty.lock().await.has_connection(&id))
}

/// Parse `~/.ssh/config` and return importable host candidates for the user to
/// pick from. Read-only: it never writes the registry. The user's picks are
/// saved through the normal [`ssh_connection_save`] path, which assigns fresh,
/// collision-free ids, so importing can never overwrite an existing connection.
/// A missing config file is not an error — it yields an empty list.
#[tauri::command]
pub async fn ssh_config_import(state: State<'_, AppState>) -> AppResult<Vec<SshConfigCandidate>> {
    let path = ssh_config_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(AppError::Internal(format!(
                "couldn't read {}: {e}",
                path.display()
            )))
        }
    };

    let registry = load_registry(&state)?;
    let mut candidates = parse_ssh_config(&text);
    // Flag candidates whose proposed id already names a saved connection so the
    // UI can warn that importing makes a duplicate (it never overwrites).
    for candidate in &mut candidates {
        let proposed = SshConnectionId::new(crate::util::slugify(&candidate.host_alias));
        candidate.already_exists =
            !proposed.as_str().is_empty() && registry.get_ssh_connection(&proposed).is_some();
    }
    Ok(candidates)
}

/// Absolute path to the user's OpenSSH client config (`~/.ssh/config`).
fn ssh_config_path() -> AppResult<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .map_err(|_| AppError::Internal("HOME is not set; can't locate ~/.ssh/config".into()))?;
    Ok(std::path::Path::new(&home).join(".ssh").join("config"))
}

/// Run the OS-detection commands and return a trimmed, single-line description.
/// This is the first authenticated connect "Detect OS" makes, so it carries the
/// host-key `interactor`; the best-effort environment/provider probes that
/// follow run only after it succeeds (i.e. after the key is trusted).
async fn detect_os_string(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> AppResult<String> {
    let primary = run_command(
        conn,
        password,
        proxy_password,
        passphrase,
        "uname -srm",
        None,
        interactor.clone(),
    )
    .await
    .map_err(AppError::Ssh)?;
    let primary_os = primary.stdout.trim();
    if !primary_os.is_empty() {
        return Ok(primary_os.to_string());
    }
    // Minimal images may lack `uname`; fall back to the distro pretty-name.
    let fallback = run_command(
        conn,
        password,
        proxy_password,
        passphrase,
        ". /etc/os-release 2>/dev/null && printf '%s' \"$PRETTY_NAME\"",
        None,
        interactor,
    )
    .await
    .map_err(AppError::Ssh)?;
    let fallback_os = fallback.stdout.trim();
    if fallback_os.is_empty() {
        Err(AppError::Internal(
            "couldn't determine the remote OS (no `uname` or /etc/os-release)".into(),
        ))
    } else {
        Ok(fallback_os.to_string())
    }
}

/// Probe shell that prints exactly one environment token. Control-panel markers
/// win first, then PaaS marker env vars (Heroku/Render/Fly/Railway), then the
/// cloud vendor (DMI), then the OS-release distro id, then a `generic`
/// fallback. One round-trip; no side effects on the remote host.
///
/// The cloud check reads several DMI fields, not just `sys_vendor`: Xen-era EC2
/// instances report `sys_vendor=Xen` and only carry "amazon" in
/// `bios_version`/`product_version`, and Oracle Cloud marks itself in
/// `chassis_asset_tag`. The Xen-EC2 `/sys/hypervisor/uuid` check is
/// prefix-anchored (`ec2*`) so a random uuid containing "ec2" can't match.
const ENVIRONMENT_PROBE: &str = r#"
if [ -e /usr/local/cpanel/version ]; then echo cpanel;
elif [ -e /usr/local/psa/version ]; then echo plesk;
elif [ -e /usr/local/directadmin ]; then echo directadmin;
elif [ -e /usr/local/CyberCP ] || [ -e /usr/local/lscp ]; then echo cyberpanel;
elif [ -e /etc/webmin/virtual-server ] || [ -e /usr/share/webmin/virtual-server ]; then echo virtualmin;
elif [ -e /usr/local/ispconfig ]; then echo ispconfig;
elif [ -e /etc/webmin ] || [ -e /usr/share/webmin ]; then echo webmin;
elif [ -n "$DYNO" ]; then echo heroku;
elif [ -n "$RENDER" ]; then echo render;
elif [ -n "$FLY_APP_NAME" ]; then echo flyio;
elif [ -n "$RAILWAY_ENVIRONMENT" ] || [ -n "$RAILWAY_PROJECT_ID" ]; then echo railway;
else
  d=/sys/class/dmi/id;
  v=$(cat $d/sys_vendor $d/bios_vendor $d/bios_version $d/product_version $d/board_vendor $d/chassis_asset_tag 2>/dev/null);
  h=$(cat /sys/hypervisor/uuid 2>/dev/null);
  case "$h" in ec2*|EC2*) v="amazon $v";; esac;
  case "$v" in
    *[Aa]mazon*|*EC2*) echo aws;;
    *DigitalOcean*) echo digitalocean;;
    *Google*) echo gcp;;
    *Microsoft*) echo azure;;
    *Hetzner*) echo hetzner;;
    *Vultr*) echo vultr;;
    *OVH*) echo ovh;;
    *Contabo*) echo contabo;;
    *OracleCloud*) echo oraclecloud;;
    *)
      id=$(. /etc/os-release 2>/dev/null; echo "$ID");
      case "$id" in
        ubuntu) echo ubuntu;; debian) echo debian;; alpine) echo alpine;;
        rhel) echo rhel;; rocky) echo rocky;; almalinux) echo almalinux;;
        centos) echo centos;; fedora) echo fedora;; opensuse*) echo opensuse;;
        amzn) echo amazonlinux;; arch) echo arch;;
        *) echo generic;;
      esac;;
  esac;
fi
"#;

/// Run [`ENVIRONMENT_PROBE`] and return its single token, or `None` on any
/// failure (best-effort — it never blocks OS detection).
async fn detect_environment(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
) -> Option<String> {
    let out = run_command(
        conn,
        password,
        proxy_password,
        passphrase,
        ENVIRONMENT_PROBE,
        None,
        None,
    )
    .await
    .ok()?;
    let token = out.stdout.trim();
    if token.is_empty() {
        None
    } else {
        // Defensive: keep only the first whitespace-delimited word.
        token.split_whitespace().next().map(ToString::to_string)
    }
}

/// Probe shell that identifies the cloud **provider** (DMI vendor) and, for the
/// known clouds, its **region** from that cloud's metadata endpoint. Prints one
/// line: `<provider>\t<region>` (region may be blank). Network calls are capped
/// at 2s each and fully best-effort — a non-cloud box just prints `generic\t`.
///
/// Same multi-field DMI read as [`ENVIRONMENT_PROBE`] (Xen-era EC2 hides
/// "amazon" outside `sys_vendor`). AWS region speaks IMDSv2 (token PUT, then
/// the v1 plain GET as a fallback) — IMDSv2-only instances reject tokenless
/// reads. If DMI is unreadable entirely, a 1s IMDSv2 token probe is the last
/// resort: only AWS answers a PUT on that link-local path.
const PROVIDER_REGION_PROBE: &str = r#"
d=/sys/class/dmi/id
v=$(cat $d/sys_vendor $d/bios_vendor $d/bios_version $d/product_version $d/board_vendor $d/chassis_asset_tag 2>/dev/null)
h=$(cat /sys/hypervisor/uuid 2>/dev/null)
case "$h" in ec2*|EC2*) v="amazon $v";; esac
if [ -z "$v" ]; then
  t=$(curl -s -X PUT --max-time 1 -H "X-aws-ec2-metadata-token-ttl-seconds: 60" http://169.254.169.254/latest/api/token 2>/dev/null)
  [ -n "$t" ] && v=amazon
fi
prov=generic; region=
case "$v" in
  *[Aa]mazon*|*EC2*)
    prov=aws
    tok=$(curl -s -X PUT --max-time 2 -H "X-aws-ec2-metadata-token-ttl-seconds: 60" http://169.254.169.254/latest/api/token 2>/dev/null)
    if [ -n "$tok" ]; then
      region=$(curl -s --max-time 2 -H "X-aws-ec2-metadata-token: $tok" http://169.254.169.254/latest/meta-data/placement/region 2>/dev/null)
    else
      region=$(curl -s --max-time 2 http://169.254.169.254/latest/meta-data/placement/region 2>/dev/null)
    fi
    ;;
  *DigitalOcean*)
    prov=digitalocean
    region=$(curl -s --max-time 2 http://169.254.169.254/metadata/v1/region 2>/dev/null)
    ;;
  *Google*)
    prov=gcp
    z=$(curl -s --max-time 2 -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/zone 2>/dev/null)
    region=${z##*/}
    ;;
  *Microsoft*)
    prov=azure
    region=$(curl -s --max-time 2 -H Metadata:true "http://169.254.169.254/metadata/instance/compute/location?api-version=2021-02-01&format=text" 2>/dev/null)
    ;;
  *Hetzner*)
    prov=hetzner
    ;;
  *Vultr*)
    prov=vultr
    region=$(curl -s --max-time 2 http://169.254.169.254/v1/region/regioncode 2>/dev/null)
    ;;
  *OVH*)
    prov=ovh
    ;;
  *Contabo*)
    prov=contabo
    ;;
  *OracleCloud*)
    prov=oraclecloud
    region=$(curl -s --max-time 2 -H "Authorization: Bearer Oracle" http://169.254.169.254/opc/v2/instance/region 2>/dev/null)
    [ -n "$region" ] || region=$(curl -s --max-time 2 http://169.254.169.254/opc/v1/instance/region 2>/dev/null)
    ;;
esac
if [ "$prov" = generic ]; then
  if [ -n "$DYNO" ]; then prov=heroku;
  elif [ -n "$RENDER" ]; then prov=render;
  elif [ -n "$FLY_APP_NAME" ]; then prov=flyio; region=$FLY_REGION;
  elif [ -n "$RAILWAY_ENVIRONMENT" ] || [ -n "$RAILWAY_PROJECT_ID" ]; then prov=railway; region=$RAILWAY_REPLICA_REGION;
  fi
fi
printf '%s\t%s' "$prov" "$region"
"#;

/// Run [`PROVIDER_REGION_PROBE`] and split its `provider\tregion` line. Returns
/// `(None, None)` on any failure — never blocks OS detection. A `generic`
/// provider (non-cloud host) is passed through so the caller can ignore it.
async fn detect_provider_region(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Ok(out) = run_command(
        conn,
        password,
        proxy_password,
        passphrase,
        PROVIDER_REGION_PROBE,
        None,
        None,
    )
    .await
    else {
        return (None, None);
    };
    let line = out.stdout.trim();
    if line.is_empty() {
        return (None, None);
    }
    let mut parts = line.splitn(2, '\t');
    let provider = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let region = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    (provider, region)
}

/// Load the registry, mutate one connection's metadata, and persist. A missing
/// connection is a silent no-op (it may have just been deleted).
fn touch_metadata(
    state: &State<'_, AppState>,
    id: &SshConnectionId,
    mutate: impl FnOnce(&mut SshConnectionMeta),
) -> AppResult<()> {
    let mut registry = load_registry(state)?;
    let Some(mut connection) = registry.get_ssh_connection(id).cloned() else {
        return Ok(());
    };
    mutate(&mut connection.metadata);
    registry
        .update_ssh_connection(connection)
        .map_err(AppError::Registry)?;
    save_registry(state, &registry)?;
    Ok(())
}

/// Remove this host's recorded key from `~/.ssh/known_hosts` (the GUI form of
/// `ssh-keygen -R host`). Use it to clear a stale entry after a "key changed"
/// warning or to forget a host; the next connect re-establishes trust (TOFU).
/// Returns the number of entries removed. Never touches PortBay's registry.
#[tauri::command]
pub async fn ssh_known_host_remove(state: State<'_, AppState>, id: String) -> AppResult<usize> {
    let registry = load_registry(&state)?;
    let raw = registry
        .get_ssh_connection(&SshConnectionId::new(&id))
        .ok_or_else(|| AppError::BadInput(format!("SSH connection `{id}` not found")))?;
    let conn = registry.effective_ssh_connection(raw);
    let host = conn.ssh_host.clone();
    let port = conn.ssh_port;
    tokio::task::spawn_blocking(move || crate::ssh::known_hosts::remove_host(&host, port))
        .await
        .map_err(|e| AppError::Internal(format!("known_hosts task failed: {e}")))?
        .map_err(|e| AppError::Internal(format!("couldn't edit known_hosts: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(name: &str, last_used: Option<u64>) -> SshConnectionView {
        SshConnectionView {
            connection: SshConnection {
                id: SshConnectionId::new(name),
                name: name.to_string(),
                ssh_host: "h".into(),
                ssh_port: 22,
                ssh_user: "u".into(),
                auth_kind: SshAuthKind::Key,
                key_path: None,
                proxy_jump: None,
                identity_id: None,
                proxy: None,
                metadata: SshConnectionMeta {
                    last_used,
                    ..Default::default()
                },
            },
            tunnel_count: 0,
            in_use: false,
        }
    }

    #[test]
    fn sort_orders_by_last_used_desc_then_name_with_never_used_last() {
        let mut views = vec![
            view("zeta", Some(100)),
            view("alpha", None),
            view("beta", Some(200)),
            view("Gamma", None),
        ];
        sort_views(&mut views);
        let order: Vec<&str> = views.iter().map(|v| v.connection.name.as_str()).collect();
        // 200 then 100 (used, desc), then the never-used by case-insensitive name.
        assert_eq!(order, vec!["beta", "zeta", "alpha", "Gamma"]);
    }

    #[test]
    fn clean_tags_trims_drops_empty_and_dedupes() {
        let tags = clean_tags(vec![
            " prod ".into(),
            "prod".into(),
            "".into(),
            "  ".into(),
            "db".into(),
        ]);
        assert_eq!(tags, vec!["prod".to_string(), "db".to_string()]);
    }

    #[test]
    fn trimmed_opt_blanks_to_none() {
        assert_eq!(trimmed_opt(Some("  ".into())), None);
        assert_eq!(trimmed_opt(Some(" x ".into())), Some("x".to_string()));
        assert_eq!(trimmed_opt(None), None);
    }
}
