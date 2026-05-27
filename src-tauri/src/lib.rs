// PortBay — Tauri 2 + Rust core.

pub mod auth;
pub mod caddy;
pub mod commands;
pub mod databases;
pub mod dnsmasq;
pub mod domain;
pub mod entitlements;
pub mod error;
pub mod flags;
pub mod hosts;
pub mod hosts_helper;
pub mod import;
pub mod mailpit;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod mkcert;
pub mod php;
pub mod port_holder;
pub mod portfile;
pub mod preferences;
pub mod process_compose;
pub mod project_runtime;
pub mod reconciler;
pub mod registry;
pub mod runtimes;
pub mod sandbox;
pub mod sidecar_probe;
pub mod smoke;
pub mod state;
pub mod sync;
pub mod telemetry;
pub mod tray;
pub mod tunnel;
pub mod util;
pub mod vibrancy;
pub mod webservers;

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::mkcert::Mkcert;
use crate::reconciler::Reconciler;
use crate::registry::store;
use crate::state::AppState;

/// Domain suffix used when the registry doesn't yet exist on disk.
/// Matches the CLI's default (`bin/portbay.rs::CliContext::load_registry`).
/// Branded `portbay.test`, so a fresh install's projects resolve at
/// `<project>.portbay.test`. `.test` is RFC 6761-reserved for local use.
const DEFAULT_DOMAIN_SUFFIX: &str = "portbay.test";

/// Cadence for the reconcile loop's safety tick. Most ticks fire on the
/// dirty-notify channel from CRUD commands; this is the fallback that
/// catches drift from CLI writes to the same registry file.
const RECONCILE_SAFETY_PERIOD: Duration = Duration::from_secs(30);

/// Install the global `tracing` subscriber. Idempotent — repeated calls
/// (e.g. from tests) silently no-op via `try_init`. Filter follows the
/// standard `PORTBAY_LOG` env var with an `info` default; the
/// `tauri_plugin_shell` and `reqwest` crates are quieted to `warn` so the
/// per-tick reconcile log isn't drowned in dependency noise.
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("PORTBAY_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tauri_plugin_shell=warn,reqwest=warn,hyper=warn"));
    let _ = fmt().with_env_filter(filter).try_init();
}

/// Resolve the mkcert binary the reconciler should use. Tries, in order:
///
/// 1. **Bundled sidecar** under the Tauri resource directory at
///    `binaries/mkcert-<target-triple>`. This is the production path
///    once the .app is built; in dev it picks up the binary the
///    `scripts/fetch-mkcert.sh` script writes into `src-tauri/binaries`.
/// 2. **Next to the running executable** (Tauri's shell plugin copies
///    sidecars here for `cargo run`).
/// 3. **PATH** via `which::which("mkcert")` — final fallback for users
///    who have mkcert installed via Homebrew and the bundle didn't ship
///    with one (e.g. a future Linux build).
///
/// Returns `None` if all three fail. The reconciler degrades gracefully:
/// HTTPS projects won't get a cert, Caddy still serves the route, and
/// the user surfaces the missing-binary state via the existing mkcert
/// slot in the sidebar.
fn resolve_mkcert_binary(app: &AppHandle) -> Option<PathBuf> {
    use std::env::consts::{ARCH, OS};

    let triple = match (OS, ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    };

    if let Some(triple) = triple {
        if let Ok(resource_dir) = app.path().resource_dir() {
            let candidate = resource_dir.join(format!("binaries/mkcert-{triple}"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let candidate = dir.join(format!("mkcert-{triple}"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    which::which("mkcert").ok()
}

/// Default location for per-process logs. `<data_dir>/PortBay/logs/`.
/// Created idempotently at setup; PC writes one file per project here.
fn resolve_logs_dir() -> std::io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    dir.push("logs");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    telemetry::install_panic_hook(env!("CARGO_PKG_VERSION"));

    // Merge the user's login-shell PATH into the process environment
    // before *anything* else spawns. GUI launches on macOS inherit a
    // minimal PATH (no shell rc files run), so brew/asdf/mise/nvm
    // installs are invisible until we ask the user's shell for its
    // actual PATH. Must run before sidecar boot (which spawns
    // process-compose) and before runtime detection.
    crate::runtimes::env::bootstrap_user_env();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Box-the-error helper — most fallible setup steps just
            // need their error stringified for Tauri's setup signature.
            fn boxed(e: impl std::fmt::Display) -> Box<dyn std::error::Error> {
                Box::<dyn std::error::Error>::from(e.to_string())
            }

            let registry_path = store::default_path().map_err(boxed)?;
            let logs_dir = resolve_logs_dir().map_err(boxed)?;
            let yaml_path = reconciler::default_yaml_path().map_err(boxed)?;

            // First run = the registry file doesn't exist yet. Detect it
            // before the load (which would create an in-memory default).
            let first_run = !registry_path.exists();

            // Build the initial PC YAML from the registry so PC boots
            // against the real config rather than the deleted bootstrap
            // placeholder. Empty registry → empty `processes: {}`, PC
            // starts and waits.
            let mut initial_registry =
                store::load_or_default(&registry_path, DEFAULT_DOMAIN_SUFFIX).map_err(boxed)?;

            // On a brand-new install, seed the "PortBay smoke" canary so the
            // user can press Play once and confirm DNS → Caddy → file serving
            // all work, before wiring up a real project. Persist it so it
            // shows up like any other project.
            if first_run {
                match crate::smoke::seed_if_absent(&mut initial_registry) {
                    Ok(true) => {
                        if let Err(e) = store::save_to(&initial_registry, &registry_path) {
                            tracing::warn!(error = %e, "failed to persist seeded smoke canary");
                        }
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!(error = %e, "smoke canary scaffold failed"),
                }
            }
            // Materialise the canary's files on every boot so it survives a
            // /tmp wipe or a deleted site dir. Cheap + idempotent.
            crate::smoke::ensure_site_files(&initial_registry);
            let initial_yaml =
                reconciler::build_initial_yaml(&initial_registry, &logs_dir).map_err(boxed)?;
            std::fs::write(&yaml_path, &initial_yaml).map_err(boxed)?;

            // Resolve mkcert; None is a tolerable degraded state.
            let mkcert_binary = resolve_mkcert_binary(app.handle());
            let mkcert = mkcert_binary
                .as_ref()
                .and_then(|p| Mkcert::default_in_data_dir(p.clone()));

            let reconciler = Reconciler::new(logs_dir.clone(), yaml_path.clone());

            app.manage(AppState::new(
                registry_path,
                // Seed the first-run fallback from the registry we just
                // loaded so `state.domain_suffix` reflects on-disk reality
                // rather than the compiled-in default.
                initial_registry.domain_suffix.clone(),
                logs_dir,
                mkcert,
                reconciler,
            ));
            app.manage(commands::metrics::MetricsState::new());

            // Boot the sidecars. PC against the just-written registry-
            // derived YAML; Caddy against its admin-only bootstrap.
            // `boot_caddy` is async (it polls the admin endpoint for
            // readiness) so we drive it with a block_on; the wait is
            // bounded by `CADDY_READINESS_TIMEOUT`.
            let state: tauri::State<AppState> = app.state();

            // Reap any process-compose left over from a previous run before we
            // boot our own. A crash or force-quit can orphan PC (and the dev
            // servers it supervised) to launchd; on a clean boot none of ours
            // should be running yet, so anything carrying our config path is
            // stale. Without this, stale instances accumulate across sessions
            // and squat on the PC admin port. SIGTERM-shutdown on quit handles
            // the prevent-leak half; this is the recover-on-boot half.
            let reaped = process_compose::lifecycle::sweep_stale(
                &yaml_path,
                None,
                process_compose::lifecycle::SweepMode::All,
            );
            if reaped > 0 {
                tracing::info!(
                    count = reaped,
                    "reaped stale process-compose instances at boot"
                );
            }

            // Same recover-on-boot half for cloudflared: a crash / SIGKILL runs
            // no `Drop`, so a quick tunnel can outlive the app — orphaned to
            // launchd and still tunneling a dead origin. Nothing of ours is up
            // yet at boot, so any cloudflared on our `--config` marker is a
            // leftover; reap it before the user starts a fresh share.
            let tunnels_reaped = tunnel::sweep_stale_cloudflared();
            if tunnels_reaped > 0 {
                tracing::info!(
                    count = tunnels_reaped,
                    "reaped stale cloudflared tunnels at boot"
                );
            }
            // Clear the cross-process tunnel mirror: nothing of ours is tunneling
            // at boot, so a stale file from a crashed prior run must not make the
            // CLI / MCP server report phantom tunnels.
            state.persist_tunnel_state();

            state.boot_pc(app.handle(), &yaml_path).map_err(boxed)?;

            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(async {
                let state: tauri::State<AppState> = app_handle.state();
                state.boot_caddy(&app_handle).await
            })
            .map_err(boxed)?;

            // Best-effort dnsmasq boot. Until the resolver-file install
            // command lands, dnsmasq running is harmless background
            // noise — no production queries route through it yet — so
            // a binary-missing or spawn failure is logged but does
            // not block startup.
            if let Err(e) = state.boot_dnsmasq(app.handle()) {
                tracing::warn!(error = %e, "dnsmasq sidecar did not start");
            }

            // Best-effort Mailpit boot. Same degraded-mode story as
            // dnsmasq: useful for catching outgoing SMTP from local
            // projects, but not on the critical path of any other
            // sidecar.
            if let Err(e) = state.boot_mailpit(app.handle()) {
                tracing::warn!(error = %e, "mailpit sidecar did not start");
            }

            // Prime the PC sub-cache with the hash of the YAML we just
            // wrote + booted against — without this, the first tick's
            // PC sub-reconciler re-restarts the daemon that boot_pc
            // spawned moments ago. (Caddy and hosts don't need priming:
            // Caddy's bootstrap config differs from the registry-driven
            // one whenever projects exist, and an empty registry's
            // POST /load against the running bootstrap is sub-50 ms.)
            let yaml_for_prime = initial_yaml.clone();
            tauri::async_runtime::block_on(async {
                let state: tauri::State<AppState> = app.state();
                state
                    .reconciler
                    .prime_pc_cache_from_yaml(&yaml_for_prime)
                    .await;
            });

            // Spawn the reconcile loop. Kick an immediate first tick so
            // the registry-driven Caddy config + hosts + certs land
            // alongside the cold boot.
            let state_ref: tauri::State<AppState> = app.state();
            state_ref.reconciler.mark_dirty();
            reconciler::spawn_reconcile_loop(app.handle().clone(), RECONCILE_SAFETY_PERIOD);

            // Spawn the status poller + metrics poller. Both run for the
            // lifetime of the app.
            commands::events::spawn_status_poller(app.handle().clone());
            commands::metrics::spawn_metrics_poller(app.handle().clone());

            // Tail Caddy's JSON access log → `portbay://request` events for the
            // HTTP request inspector. Idle until Caddy writes its first entry.
            commands::http_inspector::spawn_request_tailer(app.handle().clone());

            // Background build-artifact auto-clean. No-op unless the user opted
            // into a weekly/monthly cadence in Settings; the cadence gate lives
            // inside the scheduler.
            commands::artifacts::spawn_auto_clean_scheduler(app.handle().clone());

            // Reopen previously-running projects, if the user enabled it. Runs in
            // the background: waits for the PC daemon to accept commands, starts
            // each project from the persisted session, then reconciles routing.
            {
                let app_h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let st: tauri::State<AppState> = app_h.state();
                    if !st.preferences_snapshot().reopen_previous_projects {
                        return;
                    }
                    let ids = commands::lifecycle::load_session(&st);
                    if ids.is_empty() {
                        return;
                    }
                    let Ok(client) = st.pc_client() else {
                        return;
                    };
                    // Wait up to ~15s for the daemon to come up.
                    let mut live = false;
                    for _ in 0..30 {
                        if client.live().await.unwrap_or(false) {
                            live = true;
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                    if !live {
                        return;
                    }
                    let Ok(registry) = crate::registry::store::load_or_default(
                        &st.registry_path,
                        &st.domain_suffix,
                    ) else {
                        return;
                    };
                    for id in ids {
                        if let Some(proc_id) = registry
                            .get_project(&crate::registry::ProjectId::new(&id))
                            .and_then(|p| p.process_compose_id())
                        {
                            let _ = client.start(&proc_id).await;
                        }
                    }
                    let _ = st.reconciler.tick(&app_h).await;
                });
            }

            // Native window vibrancy for the translucent shell. Platform-aware:
            // a real NSVisualEffectView on macOS, safe no-op fallbacks elsewhere
            // (Windows Mica placeholder, Linux unchanged). See `crate::vibrancy`.
            if let Some(main_win) = app.get_webview_window("main") {
                crate::vibrancy::apply_main(&main_win);
            }
            if let Some(tray_panel) = app.get_webview_window("tray-panel") {
                crate::vibrancy::apply_tray_panel(&tray_panel);
            }

            // Install the menu-bar tray if the user hasn't disabled it.
            // Failures degrade gracefully — the dashboard still works
            // without a tray — so a tray-install error is warn-logged
            // rather than treated as a setup failure.
            let prefs = {
                let state: tauri::State<AppState> = app.state();
                state.preferences_snapshot()
            };
            if prefs.show_tray_icon {
                if let Err(e) = crate::tray::install(app.handle()) {
                    tracing::warn!(error = %e, "menu-bar tray failed to initialise");
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let label = window.label();

            // Tray popover: hide on blur so a click outside dismisses
            // the panel (sticky-popover behaviour macOS users expect).
            // Other events on this window are ignored — its close/quit
            // semantics never reach the user (it's frameless + hidden).
            if label == crate::tray::PANEL_WINDOW_LABEL {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = window.hide();
                }
                return;
            }

            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // "Close to menu bar" semantics: when the toggle is on
                    // and the tray is installed, intercept the window's
                    // close and hide it instead of letting Tauri tear it
                    // down. The tray stays the user's escape hatch — Quit
                    // from there fires `app.exit(0)` which destroys the
                    // window for real and triggers the shutdown sweep.
                    let state: tauri::State<AppState> = window.state();
                    let prefs = state.preferences_snapshot();
                    if prefs.close_to_menu_bar && prefs.show_tray_icon {
                        api.prevent_close();
                        let _ = window.hide();
                        // First-run hint: tell the user the app is still
                        // alive in the menu bar. The frontend marks the
                        // flag persistently once the toast is acknowledged.
                        if !prefs.close_to_menu_bar_toast_seen {
                            let _ = window
                                .app_handle()
                                .emit(crate::tray::CLOSE_TOAST_CHANNEL, ());
                        }
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    // Main window torn down → the app is quitting. The
                    // app-level RunEvent::Exit handler also calls this; it's
                    // idempotent, so whichever signal fires first wins and the
                    // other is a no-op.
                    let state: tauri::State<AppState> = window.state();
                    state.shutdown_all();
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::list_projects,
            commands::projects::get_project,
            commands::projects::add_project,
            commands::projects::clone_git_project_sandboxed,
            commands::projects::update_project,
            commands::projects::remove_project,
            commands::projects::detect_project,
            commands::projects::detect_workspace_apps,
            commands::projects::validate_project_folder,
            commands::lifecycle::start_project,
            commands::lifecycle::start_project_sandboxed,
            commands::lifecycle::force_start_project,
            commands::lifecycle::stop_project,
            commands::lifecycle::restart_project,
            commands::lifecycle::promote_project_to_local,
            commands::lifecycle::sandbox_violations,
            commands::lifecycle::stop_all,
            commands::lifecycle::open_project,
            commands::lifecycle::preview_port_conflict,
            commands::integrations::installed_dev_tools,
            commands::integrations::open_in_ide,
            commands::integrations::open_privacy_settings,
            commands::integrations::resolve_mcp_binary_path,
            commands::sidecars::sidecar_status,
            commands::sidecars::pc_alive,
            commands::sidecars::restart_pc,
            commands::sidecars::restart_caddy,
            commands::sidecars::reconcile_hosts,
            commands::certs::install_mkcert_ca,
            commands::certs::cert_info,
            commands::certs::reissue_cert,
            commands::webservers::webserver_overview,
            commands::system::doctor,
            commands::system::tail_logs,
            commands::system::read_dotenv,
            commands::dbconn::project_db_connections,
            commands::artifacts::scan_artifacts,
            commands::artifacts::clean_artifact,
            commands::artifacts::clean_all_artifacts,
            commands::system::quit_app,
            commands::system::open_main_window,
            commands::log_stream::subscribe_logs,
            commands::http_inspector::recent_requests,
            commands::http_inspector::clear_requests,
            commands::import::detect_sources,
            commands::import::preview_import,
            commands::import::import_projects,
            commands::portfile::export_portfile,
            commands::portfile::detect_portfile,
            commands::portfile::import_portfile_preview,
            commands::portfile::import_portfile_commit,
            commands::dnsmasq::dnsmasq_resolver_status,
            commands::dnsmasq::dnsmasq_install_resolver,
            commands::dnsmasq::dnsmasq_uninstall_resolver,
            commands::dnsmasq::restart_dnsmasq,
            commands::dnsmasq::get_dnsmasq_settings,
            commands::dnsmasq::set_dnsmasq_settings,
            commands::dnsmasq::list_dns_records,
            commands::dnsmasq::list_managed_hosts,
            commands::dnsmasq::dns_preflight,
            commands::dnsmasq::install_privileged_helper,
            commands::dnsmasq::setup_local_dns,
            commands::tunnel::start_tunnel,
            commands::tunnel::stop_tunnel,
            commands::tunnel::list_tunnels,
            commands::tunnel::tunnel_status,
            commands::onboarding::onboarding_status,
            commands::onboarding::mark_onboarded,
            commands::onboarding::reset_onboarding,
            commands::onboarding::scaffold_template,
            commands::groups::list_groups,
            commands::groups::add_group,
            commands::groups::update_group,
            commands::groups::remove_group,
            commands::groups::start_group,
            commands::groups::stop_group,
            commands::groups::restart_group,
            commands::projects::set_xdebug_mode,
            commands::metrics::system_metrics,
            commands::preferences::get_preferences,
            commands::preferences::set_preferences,
            commands::preferences::get_domain_settings,
            commands::preferences::update_domain_suffix,
            commands::preferences::mark_close_toast_seen,
            commands::entitlements::get_entitlement,
            commands::entitlements::refresh_entitlement,
            commands::entitlements::clear_entitlement,
            commands::auth::begin_login,
            commands::auth::poll_login,
            commands::auth::cancel_login,
            commands::auth::logout,
            commands::auth::account_resync,
            commands::sync::sync_state,
            commands::sync::enable_sync,
            commands::sync::get_recovery_key,
            commands::sync::set_recovery_key,
            commands::sync::disable_sync,
            commands::sync::sync_push,
            commands::sync::sync_pull,
            commands::sync::list_sync_devices,
            commands::sync::revoke_sync_device,
            commands::runtimes::list_runtimes,
            commands::runtimes::add_runtime_by_path,
            commands::runtimes::remove_runtime_path,
            commands::runtimes::remove_managed_runtime,
            commands::runtimes::set_default_runtime,
            commands::runtimes::update_runtime_config,
            commands::runtimes::install_runtime,
            commands::databases::list_database_engines,
            commands::databases::install_database_engine,
            commands::databases::list_database_instances,
            commands::databases::create_database_instance,
            commands::databases::remove_database_instance,
            commands::databases::start_database_instance,
            commands::databases::stop_database_instance,
            commands::databases::restart_database_instance,
            commands::databases::link_database_to_project,
            commands::databases::unlink_database_from_project,
            commands::databases::set_database_auto_start,
            commands::databases::open_database_client,
            commands::telemetry::telemetry_settings,
            commands::telemetry::list_crash_reports,
            commands::telemetry::read_crash_report,
            commands::telemetry::discard_crash_report,
            commands::telemetry::send_crash_report,
            commands::telemetry::record_js_error,
            commands::telemetry::record_telemetry_event,
            commands::updater::check_for_update,
            commands::updater::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Guarantee the teardown runs on EVERY quit path — ⌘Q, the tray
            // "Quit" item (`app.exit`), or the last window closing — not only
            // the main window's `Destroyed` event the old handler relied on.
            // `shutdown_all` is idempotent, so firing from both is safe.
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                app_handle.state::<AppState>().shutdown_all();
            }
        });
}
