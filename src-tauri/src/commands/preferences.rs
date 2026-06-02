//! User-preference IPC surface.
//!
//! Three commands:
//! - `get_preferences()` — return the current snapshot to the frontend
//!   on mount of the Settings page.
//! - `set_preferences(prefs)` — overwrite the persisted prefs and apply
//!   any side effects (toggle tray visibility live).
//! - `mark_close_toast_seen()` — set the first-run "still running"
//!   toast flag so it doesn't fire again.

use tauri::{AppHandle, State};

use crate::domain::{migrate_registry_suffix, DomainMigration};
use crate::error::{AppError, AppResult};
use crate::preferences::{NotificationPrefs, Preferences};
use crate::registry::store;
use crate::state::AppState;
use crate::tray;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSettings {
    pub domain_suffix: String,
    pub project_count: usize,
}

#[tauri::command]
pub async fn get_preferences(state: State<'_, AppState>) -> AppResult<Preferences> {
    Ok(state.preferences_snapshot())
}

#[tauri::command]
pub async fn get_notification_prefs(state: State<'_, AppState>) -> AppResult<NotificationPrefs> {
    Ok(state.preferences_snapshot().notifications.normalised())
}

#[tauri::command]
pub async fn set_notification_prefs(
    state: State<'_, AppState>,
    prefs: NotificationPrefs,
) -> AppResult<NotificationPrefs> {
    let prefs = prefs.normalised();
    let mut next = state.preferences_snapshot();
    next.notifications = prefs.clone();
    next.save()
        .map_err(|e| AppError::Internal(format!("failed to save preferences: {e}")))?;
    {
        let mut guard = state
            .preferences
            .lock()
            .expect("preferences mutex poisoned");
        *guard = next;
    }
    Ok(prefs)
}

/// Replace the persisted preferences and reconcile any UI side effects.
///
/// Side effects, applied in order:
/// 1. Persist to disk (fails-loudly so the frontend can show a toast).
/// 2. If the tray visibility toggled, install or uninstall it now —
///    no app restart required.
#[tauri::command]
pub async fn set_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    prefs: Preferences,
) -> AppResult<Preferences> {
    let previous = state.preferences_snapshot();

    let mut prefs = prefs;
    prefs.notifications = prefs.notifications.normalised();
    // Starting (or restarting) the auto-clean clock: when the cadence flips on
    // from "off" — or was never stamped — anchor `last_auto_clean` to now so
    // the first automatic pass is one full cadence away, never an immediate
    // surprise wipe the moment the toggle is enabled.
    if prefs.auto_clean_schedule != "off"
        && (previous.auto_clean_schedule == "off" || prefs.last_auto_clean == 0)
    {
        prefs.last_auto_clean = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }

    // Persist first; only then commit to in-memory state so a disk
    // failure leaves the running app coherent with what's on disk.
    prefs
        .save()
        .map_err(|e| AppError::Internal(format!("failed to save preferences: {e}")))?;

    {
        let mut guard = state
            .preferences
            .lock()
            .expect("preferences mutex poisoned");
        *guard = prefs.clone();
    }

    if previous.show_tray_icon != prefs.show_tray_icon {
        if prefs.show_tray_icon {
            if let Err(e) = tray::install(&app) {
                tracing::warn!(error = %e, "tray install failed");
            }
        } else {
            tray::uninstall(&app);
        }
    }

    // Dock-icon visibility toggle. Regular = icon in the Dock; Accessory =
    // no Dock tile, menu-bar tray only.
    //
    // AppKit's `setActivationPolicy:` / `setApplicationIconImage:` must run on
    // the main thread, but Tauri command handlers run on a worker thread —
    // calling them here directly was unreliable (the flip only "took" after the
    // main loop next pumped) and left the icon unset (our main-thread guard
    // bailed). So marshal the policy change onto the main thread, and re-skin
    // the icon on the main thread *after a short beat*: the new Dock tile
    // doesn't exist the instant the policy flips, so an immediate
    // `applicationIconImage` set is dropped and the tile shows the default icon.
    #[cfg(target_os = "macos")]
    if previous.show_dock_icon != prefs.show_dock_icon {
        let show = prefs.show_dock_icon;
        let app_policy = app.clone();
        let _ = app.run_on_main_thread(move || {
            let policy = if show {
                tauri::ActivationPolicy::Regular
            } else {
                tauri::ActivationPolicy::Accessory
            };
            if let Err(e) = app_policy.set_activation_policy(policy) {
                tracing::warn!(error = %e, "failed to update Dock activation policy");
            }
        });
        if show {
            let app_icon = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                let _ = app_icon.run_on_main_thread(crate::dock_icon::apply);
            });
        }
    }

    // Previously a dead toggle. Now installs/removes the platform's per-user
    // autostart entry so PortBay actually opens at login.
    if previous.launch_at_login != prefs.launch_at_login {
        if let Err(e) = apply_launch_at_login(prefs.launch_at_login) {
            tracing::warn!(error = %e, "failed to update launch-at-login entry");
        }
    }

    Ok(prefs)
}

/// Install or remove the per-user LaunchAgent that opens PortBay at login.
/// Writes `~/Library/LaunchAgents/app.portbay.autostart.plist` with `RunAtLoad`
/// pointing at the running executable; removing it on disable. Best-effort and
/// idempotent — a missing file on disable is success.
#[cfg(target_os = "macos")]
fn apply_launch_at_login(enabled: bool) -> std::io::Result<()> {
    let home = dirs::home_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    let agents = home.join("Library/LaunchAgents");
    let plist = agents.join("app.portbay.autostart.plist");

    if !enabled {
        return match std::fs::remove_file(&plist) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }

    std::fs::create_dir_all(&agents)?;
    let exe = std::env::current_exe()?;
    let contents = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>app.portbay.autostart</string>
    <key>ProgramArguments</key>
    <array><string>{exe}</string></array>
    <key>RunAtLoad</key><true/>
    <key>LimitLoadToSessionType</key><string>Aqua</string>
</dict>
</plist>
"#,
        exe = exe.display(),
    );
    // Atomic write so a crash mid-write can't leave a half-plist launchd chokes on.
    let tmp = plist.with_extension("plist.tmp");
    std::fs::write(&tmp, contents.as_bytes())?;
    std::fs::rename(&tmp, &plist)?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[cfg(not(target_os = "linux"))]
fn apply_launch_at_login(_enabled: bool) -> std::io::Result<()> {
    Ok(())
}

/// Linux desktop autostart uses the XDG Autostart spec. This is the most
/// broadly-supported equivalent to a macOS LaunchAgent for GUI sessions.
#[cfg(target_os = "linux")]
fn apply_launch_at_login(enabled: bool) -> std::io::Result<()> {
    let config = dirs::config_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir"))?;
    let autostart = config.join("autostart");
    let desktop = autostart.join("portbay.desktop");

    if !enabled {
        return match std::fs::remove_file(&desktop) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }

    std::fs::create_dir_all(&autostart)?;
    let exe = std::env::current_exe()?;
    let contents = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=PortBay\n\
         Comment=Start PortBay at login\n\
         Exec={exe}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exe = desktop_exec_quote(&exe.to_string_lossy()),
    );
    let tmp = desktop.with_extension("desktop.tmp");
    std::fs::write(&tmp, contents.as_bytes())?;
    std::fs::rename(&tmp, &desktop)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn desktop_exec_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':'))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[tauri::command]
pub async fn get_domain_settings(state: State<'_, AppState>) -> AppResult<DomainSettings> {
    let registry = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    Ok(DomainSettings {
        domain_suffix: registry.domain_suffix,
        project_count: registry.projects.len(),
    })
}

#[tauri::command]
pub async fn update_domain_suffix(
    app: AppHandle,
    state: State<'_, AppState>,
    domain_suffix: String,
) -> AppResult<DomainMigration> {
    // Customizing the domain suffix is a Pro feature. Community (anonymous/free)
    // stays on the default — it already serves the purpose, and a misconfigured
    // suffix breaks local DNS resolution for exactly the users least equipped to
    // debug it. Enforced core-side so a disabled UI field can't be bypassed.
    use crate::entitlements::EntitlementState;
    if !matches!(
        crate::entitlements::current().state,
        EntitlementState::Pro | EntitlementState::ProGrace
    ) {
        return Err(AppError::BadInput(
            "Customizing the domain suffix is a Pro feature — community projects use the default suffix.".into(),
        ));
    }
    let mut registry = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    let old_suffix = registry.domain_suffix.clone();
    let certs_root = certs_root();
    let migration = migrate_registry_suffix(&mut registry, &domain_suffix, certs_root)
        .map_err(|e| AppError::BadInput(e.to_string()))?;
    store::save_to(&registry, &state.registry_path)?;

    // dnsmasq was previously left serving the OLD suffix's wildcard, and the
    // stale `/etc/resolver/<old>` kept routing `*.<old>` to localhost. Restart
    // dnsmasq so it regenerates config for the new suffix; and if a wildcard
    // resolver was installed for the old suffix, migrate it to the new one via
    // the privileged helper (no extra prompt). Only when one actually existed,
    // so a user who never installed the resolver isn't surprised by one now.
    if old_suffix != domain_suffix {
        let old_port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();
        let had_resolver = crate::dnsmasq::resolver::is_installed(&old_suffix, old_port);

        if let Err(e) = state.boot_dnsmasq(&app) {
            tracing::warn!(error = %e, "dnsmasq restart after suffix change failed");
        }

        if had_resolver {
            let helper = crate::hosts_helper::HostsHelperClient::system();
            if helper.is_available() {
                let new_port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();
                let old = old_suffix.clone();
                let new = domain_suffix.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let h = crate::hosts_helper::HostsHelperClient::system();
                    let _ = h.remove_resolver(&old);
                    let _ = h.install_resolver(&new, new_port);
                })
                .await;
            } else {
                tracing::warn!(
                    old = %old_suffix,
                    "dnsmasq resolver for old suffix left in place — privileged helper \
                     unavailable; user can re-install the resolver for the new suffix"
                );
            }
        }
    }

    state.reconciler.mark_dirty();
    Ok(migration)
}

#[tauri::command]
pub async fn mark_close_toast_seen(state: State<'_, AppState>) -> AppResult<()> {
    let mut updated = state.preferences_snapshot();
    if updated.close_to_menu_bar_toast_seen {
        return Ok(());
    }
    updated.close_to_menu_bar_toast_seen = true;
    updated
        .save()
        .map_err(|e| AppError::Internal(format!("failed to save preferences: {e}")))?;
    *state
        .preferences
        .lock()
        .expect("preferences mutex poisoned") = updated;
    Ok(())
}

fn certs_root() -> Option<std::path::PathBuf> {
    let mut dir = dirs::data_dir()?;
    dir.push("PortBay");
    dir.push("certs");
    Some(dir)
}
