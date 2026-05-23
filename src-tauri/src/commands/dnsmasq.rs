//! dnsmasq-related commands: resolver-file install / uninstall /
//! status, plus sidecar restart.
//!
//! Resolver-install is the gate that makes dnsmasq actually answer
//! real queries. Until the user clicks the Settings → DNS button (or
//! invokes this from the CLI), the daemon runs harmlessly on
//! loopback and macOS never routes anything to it.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::dnsmasq::resolver;
use crate::error::{AppError, AppResult};
use crate::hosts::HostsManager;
use crate::registry::{store, DnsmasqSettings};
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
    let suffix = state.domain_suffix.clone();
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
    let suffix = state.domain_suffix.clone();
    let port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();

    // Run the osascript prompt off the async runtime — it blocks on
    // the macOS auth dialog and can take seconds (or never resolve if
    // the user walks away).
    let result =
        tokio::task::spawn_blocking(move || resolver::install_via_osascript(&suffix, port))
            .await
            .map_err(|e| AppError::Internal(format!("install join: {e}")))?;

    result.map_err(AppError::from)
}

#[tauri::command]
pub async fn dnsmasq_uninstall_resolver(state: State<'_, AppState>) -> AppResult<()> {
    let suffix = state.domain_suffix.clone();
    let result = tokio::task::spawn_blocking(move || resolver::uninstall_via_osascript(&suffix))
        .await
        .map_err(|e| AppError::Internal(format!("uninstall join: {e}")))?;
    result.map_err(AppError::from)
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
