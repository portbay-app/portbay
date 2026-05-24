//! dnsmasq-related commands: resolver-file install / uninstall /
//! status, plus sidecar restart.
//!
//! Resolver-install is the gate that makes dnsmasq actually answer
//! real queries. Until the user clicks the Settings → DNS button (or
//! invokes this from the CLI), the daemon runs harmlessly on
//! loopback and macOS never routes anything to it.

use std::net::{Ipv4Addr, TcpStream};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::dnsmasq::resolver;
use crate::error::{AppError, AppResult};
use crate::hosts::{HostsEntry, HostsManager};
use crate::hosts_helper::{self, HostsHelperClient};
use crate::registry::{store, DnsmasqSettings, Registry};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverStatus {
    /// The domain suffix this status reflects (matches
    /// `AppState::domain_suffix`).
    pub suffix: String,
    /// True iff `/etc/resolver/<suffix>` exists *and* references the
    /// current dnsmasq port. A stale file from an older boot (port
    /// mismatch) reads as `false`.
    pub installed: bool,
    /// Path of the resolver file we'd read or write.
    pub path: String,
    /// Whatever is currently in the file (for diagnostic display).
    /// `None` when the file is missing entirely.
    pub current_contents: Option<String>,
    /// Port the daemon is currently listening on, exposed so the
    /// settings UI can render "expected port: …" without re-querying.
    pub current_port: u16,
}

#[tauri::command]
pub async fn dnsmasq_resolver_status(state: State<'_, AppState>) -> AppResult<ResolverStatus> {
    let suffix = current_suffix(&state);
    let port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();
    Ok(ResolverStatus {
        path: resolver::resolver_file_path(&suffix)
            .to_string_lossy()
            .into_owned(),
        installed: resolver::is_installed(&suffix, port),
        current_contents: resolver::read_installed(&suffix),
        current_port: port,
        suffix,
    })
}

#[tauri::command]
pub async fn dnsmasq_install_resolver(state: State<'_, AppState>) -> AppResult<()> {
    let suffix = current_suffix(&state);
    let port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();

    // Prefer PortBay's privileged helper (silent — no prompt). Only fall back
    // to the per-action osascript prompt when the helper isn't installed.
    let helper = HostsHelperClient::system();
    if helper.is_available() {
        return tokio::task::spawn_blocking(move || helper.install_resolver(&suffix, port))
            .await
            .map_err(|e| AppError::Internal(format!("install join: {e}")))?
            .map_err(|e| AppError::Internal(e.to_string()));
    }

    let result =
        tokio::task::spawn_blocking(move || resolver::install_via_osascript(&suffix, port))
            .await
            .map_err(|e| AppError::Internal(format!("install join: {e}")))?;
    result.map_err(AppError::from)
}

#[tauri::command]
pub async fn dnsmasq_uninstall_resolver(state: State<'_, AppState>) -> AppResult<()> {
    let suffix = current_suffix(&state);
    let helper = HostsHelperClient::system();
    if helper.is_available() {
        return tokio::task::spawn_blocking(move || helper.remove_resolver(&suffix))
            .await
            .map_err(|e| AppError::Internal(format!("uninstall join: {e}")))?
            .map_err(|e| AppError::Internal(e.to_string()));
    }

    let result = tokio::task::spawn_blocking(move || resolver::uninstall_via_osascript(&suffix))
        .await
        .map_err(|e| AppError::Internal(format!("uninstall join: {e}")))?;
    result.map_err(AppError::from)
}

/// The active domain suffix — read from the registry (source of truth),
/// falling back to the cached startup value.
fn current_suffix(state: &AppState) -> String {
    store::load_or_default(&state.registry_path, &state.domain_suffix)
        .map(|r| r.domain_suffix)
        .unwrap_or_else(|_| state.domain_suffix.clone())
}

/// `restart_dnsmasq()` — stop the bundled dnsmasq sidecar and start
/// it again against a fresh config. Picked up by the dnsmasq card's
/// action button.
#[tauri::command]
pub async fn restart_dnsmasq(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.shutdown_dnsmasq();
    state.boot_dnsmasq(&app)
}

/// Current user-tunable dnsmasq settings (cache size, local TTL, negative
/// cache). Read from the registry, which is the source of truth.
#[tauri::command]
pub async fn get_dnsmasq_settings(state: State<'_, AppState>) -> AppResult<DnsmasqSettings> {
    let reg = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    Ok(reg.dnsmasq)
}

/// Persist new dnsmasq settings, then regenerate the config and restart the
/// daemon so the cache/TTL directives take effect immediately. Values are
/// clamped to safe ranges before they're written. Best-effort restart: the
/// settings are persisted even when no dnsmasq binary is present, so they
/// apply on the next boot.
#[tauri::command]
pub async fn set_dnsmasq_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: DnsmasqSettings,
) -> AppResult<DnsmasqSettings> {
    let sanitised = settings.sanitised();
    let mut reg = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    reg.dnsmasq = sanitised.clone();
    store::save_to(&reg, &state.registry_path)?;

    state.shutdown_dnsmasq();
    state.boot_dnsmasq(&app)?;
    Ok(sanitised)
}

/// A single resolvable name PortBay knows about, for the DNS page's records
/// list. Either the wildcard for the suffix or one project hostname.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsRecord {
    /// The name as it resolves, e.g. `*.portbay.test` or `nour-beiruti.portbay.test`.
    pub hostname: String,
    /// Always loopback for PortBay-managed names.
    pub target: String,
    /// `"wildcard"` or `"project"`.
    pub kind: &'static str,
    /// Set for project rows so the UI can deep-link into the project panel.
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    /// `"dnsmasq"` when the resolver file routes this suffix, else `"hosts"`.
    pub routed_via: &'static str,
}

/// Build the read-only DNS records view: the wildcard for the active suffix
/// plus one row per project hostname, each tagged with how it's currently
/// routed.
#[tauri::command]
pub async fn list_dns_records(state: State<'_, AppState>) -> AppResult<Vec<DnsRecord>> {
    let reg = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    let suffix = reg.domain_suffix.clone();
    let port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();
    let dns_routing = resolver::is_installed(&suffix, port);
    let suffix_tail = format!(".{suffix}");

    let mut records = Vec::with_capacity(reg.projects.len() + 1);
    records.push(DnsRecord {
        hostname: format!("*.{suffix}"),
        target: "127.0.0.1".into(),
        kind: "wildcard",
        project_id: None,
        project_name: None,
        routed_via: "dnsmasq",
    });
    for p in reg.list_projects() {
        let in_suffix = p.hostname.ends_with(&suffix_tail);
        records.push(DnsRecord {
            hostname: p.hostname.clone(),
            target: "127.0.0.1".into(),
            kind: "project",
            project_id: Some(p.id.to_string()),
            project_name: Some(p.name.clone()),
            routed_via: if dns_routing && in_suffix {
                "dnsmasq"
            } else {
                "hosts"
            },
        });
    }
    Ok(records)
}

/// One entry from PortBay's managed block in `/etc/hosts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedHostsEntry {
    pub ip: String,
    pub hostname: String,
}

/// The entries PortBay currently manages inside its `# BEGIN/END PortBay`
/// block in `/etc/hosts`. Read-only; the reconciler owns writes.
#[tauri::command]
pub async fn list_managed_hosts(_state: State<'_, AppState>) -> AppResult<Vec<ManagedHostsEntry>> {
    let entries = HostsManager::system()
        .list_managed()
        .map_err(|e| AppError::Internal(format!("read /etc/hosts: {e}")))?;
    Ok(entries
        .into_iter()
        .map(|e| ManagedHostsEntry {
            ip: e.ip.to_string(),
            hostname: e.hostname,
        })
        .collect())
}

/// First-run readiness snapshot the UI uses to decide whether to offer
/// "Set up local DNS" and to warn about a port conflict.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsPreflight {
    pub suffix: String,
    pub dnsmasq_port: u16,
    /// PortBay's privileged helper LaunchDaemon is installed + reachable.
    pub helper_installed: bool,
    /// Every project hostname is present in PortBay's `/etc/hosts` block —
    /// the primary resolution path. Vacuously true when there are no projects.
    pub hosts_active: bool,
    /// `/etc/resolver/<suffix>` points at the running dnsmasq (wildcard
    /// enhancement; not required for the critical path).
    pub resolver_installed: bool,
    /// The bundled dnsmasq sidecar is running (wildcard enhancement).
    pub dnsmasq_running: bool,
    /// Something is already listening on :80 / :443 (likely another local web
    /// server such as ServBay) — PortBay can't serve clean URLs until it's freed.
    pub port_80_in_use: bool,
    pub port_443_in_use: bool,
    /// True when the primary path is live: the helper is installed and every
    /// project hostname is in `/etc/hosts`. dnsmasq/resolver are optional and
    /// deliberately excluded so a wildcard-only hiccup never re-triggers setup.
    pub ready: bool,
}

/// True iff every project hostname in the registry is present in the supplied
/// `/etc/hosts` managed entries. Vacuously true for an empty registry. Pure so
/// it can be unit-tested without touching the real hosts file.
fn hosts_block_covers(reg: &Registry, managed: &[HostsEntry]) -> bool {
    use std::collections::HashSet;
    let present: HashSet<&str> = managed.iter().map(|e| e.hostname.as_str()).collect();
    reg.list_projects()
        .iter()
        .all(|p| present.contains(p.hostname.as_str()))
}

fn port_in_use(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().expect("valid addr"),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// Inspect the local routing setup so the UI can guide first-time users.
#[tauri::command]
pub async fn dns_preflight(state: State<'_, AppState>) -> AppResult<DnsPreflight> {
    let reg = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    let suffix = reg.domain_suffix.clone();
    let (port, dnsmasq_running) = {
        let guard = state.dnsmasq.lock().expect("dnsmasq mutex poisoned");
        (guard.port(), guard.is_running())
    };
    let helper_installed = HostsHelperClient::system().is_available();
    let resolver_installed = resolver::is_installed(&suffix, port);
    let managed = HostsManager::system().list_managed().unwrap_or_default();
    let hosts_active = hosts_block_covers(&reg, &managed);
    Ok(DnsPreflight {
        ready: helper_installed && hosts_active,
        suffix,
        dnsmasq_port: port,
        helper_installed,
        hosts_active,
        resolver_installed,
        dnsmasq_running,
        port_80_in_use: port_in_use(80),
        port_443_in_use: port_in_use(443),
    })
}

/// Resolve the helper binary that ships next to the app executable (dev: the
/// sibling in `target/debug`; production: inside the app bundle's MacOS dir).
fn resolve_helper_bin() -> AppResult<std::path::PathBuf> {
    let exe = std::env::current_exe()
        .map_err(|e| AppError::Internal(format!("locate current exe: {e}")))?;
    let candidate = exe
        .parent()
        .map(|p| p.join("portbay-hosts-helper"))
        .ok_or_else(|| AppError::Internal("no parent dir for current exe".into()))?;
    if candidate.exists() {
        Ok(candidate)
    } else {
        Err(AppError::Internal(format!(
            "helper binary not found next to the app at {}",
            candidate.display()
        )))
    }
}

/// Ensure PortBay's privileged helper LaunchDaemon is installed and its
/// socket is reachable. No-op (and no prompt) when it's already up; otherwise
/// triggers the single macOS auth prompt, then polls for the socket. Shared by
/// `install_privileged_helper` and `setup_local_dns`.
async fn ensure_helper_installed() -> AppResult<()> {
    let client = HostsHelperClient::system();
    if client.is_available() {
        return Ok(());
    }
    let helper_bin = resolve_helper_bin()?;
    tokio::task::spawn_blocking(move || hosts_helper::install_daemon(&helper_bin))
        .await
        .map_err(|e| AppError::Internal(format!("helper install join: {e}")))?
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // The daemon is bootstrapped but may take a beat to bind its socket.
    for _ in 0..30 {
        if client.is_available() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(AppError::Internal(
        "helper installed but its socket did not appear — check Console for the daemon".into(),
    ))
}

/// Install PortBay's privileged helper LaunchDaemon. One macOS auth prompt;
/// afterwards the helper performs hosts + resolver writes with no further
/// prompts. Polls for the helper socket so the caller knows it's live.
#[tauri::command]
pub async fn install_privileged_helper(_app: AppHandle) -> AppResult<()> {
    ensure_helper_installed().await
}

/// One-click first-run setup. Guarantees that every project hostname resolves
/// the instant this returns, with at most one macOS password prompt:
///
/// 1. Ensure the privileged helper is installed (the only step that can
///    prompt; silent if already installed).
/// 2. **Primary path** — synchronously write every project hostname into
///    `/etc/hosts` through the helper. This is the bulletproof guarantee: it
///    needs no port, no daemon, and no resolver file, and the system resolver
///    consults `/etc/hosts` before DNS. Callers (Play) await this, so the line
///    exists before the dev server starts.
/// 3. **Optional enhancement** — best-effort dnsmasq + `/etc/resolver` at its
///    live port so `dig` and arbitrary `*.suffix` subdomains resolve too. A
///    failure here never fails setup; `/etc/hosts` already did the job.
#[tauri::command]
pub async fn setup_local_dns(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    // 1. Privileged helper — the one prompt.
    ensure_helper_installed().await?;

    // 2. Primary path: write all project hostnames to /etc/hosts now.
    let reg = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    let suffix = reg.domain_suffix.clone();
    let pairs: Vec<(String, Ipv4Addr)> = reg
        .list_projects()
        .iter()
        .map(|p| (p.hostname.clone(), Ipv4Addr::LOCALHOST))
        .collect();
    {
        let suffix = suffix.clone();
        tokio::task::spawn_blocking(move || {
            HostsHelperClient::system().replace_all(pairs, &suffix)
        })
        .await
        .map_err(|e| AppError::Internal(format!("hosts write join: {e}")))?
        .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // 3. Optional enhancement: bring dnsmasq up and point /etc/resolver at the
    //    port it actually bound. No trailing restart — boot_dnsmasq already
    //    reads the current suffix from the registry, and a restart would
    //    re-pick a free port and strand the resolver file we just wrote.
    if state.boot_dnsmasq(&app).is_ok() {
        let (port, running) = {
            let guard = state.dnsmasq.lock().expect("dnsmasq mutex poisoned");
            (guard.port(), guard.is_running())
        };
        if running {
            let _ = tokio::task::spawn_blocking(move || {
                HostsHelperClient::system().install_resolver(&suffix, port)
            })
            .await;
        }
    }

    // Keep the reconciler's hosts cache coherent with the direct write above.
    state.reconciler.mark_dirty();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn project(id: &str, host: &str) -> Project {
        Project {
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
            extra_ports: vec![],
            hostname: host.into(),
            https: false,
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

    fn entry(host: &str) -> HostsEntry {
        HostsEntry {
            ip: Ipv4Addr::LOCALHOST,
            hostname: host.into(),
        }
    }

    #[test]
    fn empty_registry_is_vacuously_covered() {
        let reg = Registry::new("portbay.test");
        assert!(hosts_block_covers(&reg, &[]));
    }

    #[test]
    fn covered_only_when_every_hostname_present() {
        let mut reg = Registry::new("portbay.test");
        reg.add_project(project("a", "a.portbay.test")).unwrap();
        reg.add_project(project("b", "b.portbay.test")).unwrap();

        // Missing one → not covered.
        assert!(!hosts_block_covers(&reg, &[entry("a.portbay.test")]));

        // All present (extra unrelated entries don't matter) → covered.
        assert!(hosts_block_covers(
            &reg,
            &[
                entry("a.portbay.test"),
                entry("b.portbay.test"),
                entry("unrelated.test"),
            ],
        ));
    }
}
