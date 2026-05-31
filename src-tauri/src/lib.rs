// PortBay — Tauri 2 + Rust core.

pub mod agents;
pub mod auth;
pub mod avatar;
pub mod caddy;
pub mod commands;
// Proprietary task board. Source injected from `portbay-cloud/desktop-pro` for
// official builds; absent from the public OSS tree (no `tasks` feature).
#[cfg(feature = "tasks")]
pub mod context;
pub mod databases;
pub mod dnsmasq;
pub mod dock_icon;
pub mod doctor;
pub mod domain;
pub mod entitlements;
pub mod error;
pub mod flags;
pub mod hosts;
pub mod hosts_helper;
pub mod import;
pub mod install_proxy;
pub mod mailpit;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod mkcert;
pub mod mobile;
pub mod notifications;
pub mod php;
pub mod port_holder;
pub mod portfile;
pub mod preferences;
pub mod process_compose;
pub mod project_icon;
pub mod project_runtime;
pub mod reconciler;
pub mod registry;
pub mod runtimes;
pub mod sandbox;
pub mod sidecar_probe;
pub mod sidecar_reclaim;
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

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    // Single-instance guard MUST be the first plugin registered (Tauri
    // requirement). When a user launches PortBay while a copy is already
    // running, the second process invokes this callback in the *existing*
    // instance and then exits — so it never reaches `setup` and never spawns a
    // second sidecar stack squatting the canonical ports. We focus the live
    // window so the relaunch feels like "bring to front", the standard desktop
    // UX. (macOS's own Reopen event does the same on Dock-icon clicks; this
    // covers `open -n`, the CLI, and `tauri dev`.)
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }));
    }

    builder
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

            // Recover-on-boot for the three sidecars spawned *directly* as app
            // children — caddy, dnsmasq, mailpit. Unlike process-compose (swept
            // above) these had no boot reclamation, so a crash / `tauri dev`
            // rebuild left them orphaned to launchd, still holding :443 / the DNS
            // port / the SMTP port. The fresh stack then couldn't bind :443 and
            // silently fell back to an alternate port, serving no TLS for the
            // canonical host (the ERR_SSL_PROTOCOL_ERROR incident). Reaping them
            // here — before we boot our own — frees the canonical ports so the
            // fresh Caddy binds :443. `All` mode is safe: none of ours is up yet,
            // so any match is by definition stale. For Caddy the reclaim also
            // waits for :443 to actually release before returning, closing the
            // kill→rebind race. Foreign caddy/dnsmasq/mailpit (ServBay, Homebrew)
            // never match — the signature keys on PortBay's own config paths.
            //
            // php-fpm is reaped here too — it's a process-compose child, so the
            // stale-PC sweep above orphans it (PC gets SIGKILLed before it can
            // drain a 5 s FPM shutdown), and an orphaned FPM master keeps its
            // unix socket bound. The fresh build's php-fpm then fails to start
            // ("Another FPM instance seems to already listen…") and surfaces as a
            // spurious "php-fpm crashed" notification. Reaping it now — after the
            // PC sweep, before we boot our own PC — frees the socket.
            for kind in [
                sidecar_reclaim::SidecarKind::Caddy,
                sidecar_reclaim::SidecarKind::Dnsmasq,
                sidecar_reclaim::SidecarKind::Mailpit,
                sidecar_reclaim::SidecarKind::PhpFpm,
            ] {
                sidecar_reclaim::reclaim_stale(kind, sidecar_reclaim::SweepMode::All);
            }

            // Advisory pidfile so the CLI / `portbay doctor` can tell the app is
            // live (and reclaim only orphans, never the live app's children).
            // The single-instance plugin is the real double-spawn guard; this is
            // just out-of-process visibility. Removed on graceful shutdown;
            // `app_running` re-checks PID liveness so a crash-leftover is ignored.
            sidecar_reclaim::write_pidfile();

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
            // Watch every project's audit log for new agent activity (comments,
            // blocks, warnings) and surface it to the topbar bell + desktop.
            // Board-only: the bell shell ships in every build, but the scanner
            // that feeds it is injected with the `tasks` feature.
            #[cfg(feature = "tasks")]
            notifications::spawn_scanner(app.handle().clone());

            // Tail Caddy's JSON access log → `portbay://request` events for the
            // HTTP request inspector. Idle until Caddy writes its first entry.
            commands::http_inspector::spawn_request_tailer(app.handle().clone());

            // Reconcile per-project task-board leases on boot: a board dispatched
            // an agent, then the app (or laptop) went down. Any lease whose
            // process is gone / heartbeat expired is reclaimed (card → To Do,
            // reason logged) so a crashed run never wedges the board (edge cases
            // #2/#11). Best-effort; never blocks startup. Board-only.
            #[cfg(feature = "tasks")]
            {
                let app_h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let st: tauri::State<AppState> = app_h.state();
                    if let Ok(reg) = store::load_or_default(&st.registry_path, &st.domain_suffix) {
                        for project in reg.list_projects() {
                            let _ = crate::context::automation::reconcile(project);
                        }
                    }
                });
            }

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
                // Reveal the window from Rust, *after* vibrancy is in place. It's
                // created hidden (`visible: false`) so the blur material is set
                // before it appears; doing the reveal here — rather than relying
                // on the frontend calling `.show()` — guarantees the window opens
                // on launch even if the webview is slow or errors. Previously a
                // failed frontend reveal left the window openable only via the
                // tray.
                let _ = main_win.show();
                let _ = main_win.set_focus();

                // Appearance-aware Dock icon: match the current Light/Dark
                // appearance now (read from NSApplication.effectiveAppearance),
                // and keep it in sync on the ThemeChanged window event below.
                // See `crate::dock_icon`.
                crate::dock_icon::apply();
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

            // Dock-icon visibility: default Regular (icon in the Dock). If the
            // user turned it off, run as an Accessory — no Dock tile, present
            // only in the menu-bar tray. The default `tauri.conf.json` policy is
            // Regular, so we only act when the preference is off.
            #[cfg(target_os = "macos")]
            if !prefs.show_dock_icon {
                if let Err(e) = app
                    .handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory)
                {
                    tracing::warn!(error = %e, "failed to set accessory activation policy");
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
                tauri::WindowEvent::ThemeChanged(_) => {
                    // System appearance flipped — re-skin the Dock icon to the
                    // matching light/dark variant. See `crate::dock_icon`.
                    crate::dock_icon::apply();
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
            commands::projects::project_icon,
            commands::lifecycle::start_project,
            commands::lifecycle::start_project_sandboxed,
            commands::lifecycle::force_start_project,
            commands::lifecycle::stop_project,
            commands::lifecycle::restart_project,
            commands::lifecycle::promote_project_to_local,
            commands::lifecycle::sandbox_violations,
            commands::lifecycle::install_project_sandboxed,
            commands::lifecycle::stop_all,
            commands::lifecycle::open_project,
            commands::lifecycle::reveal_in_finder,
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
            commands::tunnel::list_named_tunnels,
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
            commands::entitlements::pro_checkout_url,
            commands::auth::begin_login,
            commands::auth::poll_login,
            commands::auth::cancel_login,
            commands::auth::logout,
            commands::auth::account_resync,
            commands::auth::get_account_avatar,
            commands::profile::update_display_name,
            commands::profile::upload_avatar,
            commands::profile::remove_avatar,
            commands::sync::sync_state,
            commands::sync::enable_sync,
            commands::sync::get_recovery_key,
            commands::sync::set_recovery_key,
            commands::sync::disable_sync,
            commands::sync::sync_push,
            commands::sync::sync_pull,
            commands::sync::activate_device,
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
            commands::databases::remove_managed_engine,
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
            commands::databases::list_instance_databases,
            commands::databases::create_instance_database,
            commands::databases::drop_instance_database,
            commands::databases::provision_project_database,
            commands::databases::list_database_backups,
            commands::databases::backup_database_instance,
            commands::databases::restore_database_backup,
            commands::databases::delete_database_backup,
            commands::telemetry::telemetry_settings,
            commands::telemetry::list_crash_reports,
            commands::telemetry::read_crash_report,
            commands::telemetry::discard_crash_report,
            commands::telemetry::send_crash_report,
            commands::telemetry::record_js_error,
            commands::telemetry::record_telemetry_event,
            commands::updater::check_for_update,
            commands::updater::install_update,
            // Per-project task board (Project Context & Task Authority).
            // Board-only handlers — injected with the `tasks` feature; absent
            // from the public OSS build. `generate_handler!` honours the per-
            // entry `#[cfg]`, so the public build registers none of these.
            #[cfg(feature = "tasks")]
            commands::tasks::tasks_list,
            #[cfg(feature = "tasks")]
            commands::tasks::task_get,
            #[cfg(feature = "tasks")]
            commands::tasks::board_config_get,
            #[cfg(feature = "tasks")]
            commands::tasks::agents_installed,
            #[cfg(feature = "tasks")]
            commands::tasks::set_agent_path,
            #[cfg(feature = "tasks")]
            commands::tasks::clear_agent_path,
            #[cfg(feature = "tasks")]
            commands::tasks::set_agent_launch_mode,
            #[cfg(feature = "tasks")]
            commands::tasks::board_audit,
            #[cfg(feature = "tasks")]
            commands::tasks::board_cloud_sync,
            #[cfg(feature = "tasks")]
            commands::tasks::board_templates,
            #[cfg(feature = "tasks")]
            commands::tasks::scratchpad_get,
            #[cfg(feature = "tasks")]
            commands::tasks::scratchpad_set,
            #[cfg(feature = "tasks")]
            commands::tasks::task_capture,
            #[cfg(feature = "tasks")]
            commands::tasks::task_promote,
            #[cfg(feature = "tasks")]
            commands::tasks::task_create,
            #[cfg(feature = "tasks")]
            commands::tasks::task_update,
            #[cfg(feature = "tasks")]
            commands::tasks::task_move,
            #[cfg(feature = "tasks")]
            commands::tasks::task_reorder,
            #[cfg(feature = "tasks")]
            commands::tasks::task_delete,
            #[cfg(feature = "tasks")]
            commands::tasks::task_check_item,
            #[cfg(feature = "tasks")]
            commands::tasks::task_comment,
            #[cfg(feature = "tasks")]
            commands::tasks::task_checklist_add,
            #[cfg(feature = "tasks")]
            commands::tasks::task_archive,
            #[cfg(feature = "tasks")]
            commands::tasks::task_subscribe,
            #[cfg(feature = "tasks")]
            commands::tasks::task_attach,
            #[cfg(feature = "tasks")]
            commands::tasks::task_detach,
            #[cfg(feature = "tasks")]
            commands::tasks::task_attachment_path,
            #[cfg(feature = "tasks")]
            commands::tasks::card_activity,
            #[cfg(feature = "tasks")]
            commands::tasks::task_run_log,
            #[cfg(feature = "tasks")]
            commands::tasks::task_branch,
            #[cfg(feature = "tasks")]
            commands::tasks::task_duplicate,
            #[cfg(feature = "tasks")]
            commands::tasks::task_start_with_agent,
            #[cfg(feature = "tasks")]
            commands::tasks::task_comment_dispatch,
            #[cfg(feature = "tasks")]
            commands::tasks::task_comment_edit,
            #[cfg(feature = "tasks")]
            commands::tasks::task_comment_delete,
            #[cfg(feature = "tasks")]
            commands::tasks::task_stop_agent,
            #[cfg(feature = "tasks")]
            commands::tasks::board_reconcile,
            #[cfg(feature = "tasks")]
            commands::tasks::board_config_set,
            #[cfg(feature = "tasks")]
            commands::tasks::context_sync,
            #[cfg(feature = "tasks")]
            commands::tasks::context_show,
            #[cfg(feature = "tasks")]
            commands::tasks::handoff_show,
            #[cfg(feature = "tasks")]
            commands::tasks::handoff_update,
            #[cfg(feature = "tasks")]
            commands::tasks::handoff_replace,
            #[cfg(feature = "tasks")]
            commands::tasks::tasks_watch,
            #[cfg(feature = "tasks")]
            commands::tasks::tasks_unwatch,
            commands::notifications::notifications_list,
            commands::notifications::notifications_mark_read,
            commands::notifications::notifications_mark_all_read,
            commands::notifications::notifications_clear,
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

            // Re-assert the appearance-aware Dock icon once the app has fully
            // launched. Tauri sets the bundle icon during startup (after our
            // `setup` hook), so applying here — after Ready — is what actually
            // sticks on the live Dock tile. See `crate::dock_icon`.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Ready = event {
                crate::dock_icon::apply();
            }

            // Dock-icon click (macOS) → reveal + focus the main window. Without
            // this, "close to menu bar" (or any hidden state) left clicking the
            // Dock icon doing nothing — the user could only reopen via the tray.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Some(win) = app_handle.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                }
            }
        });
}
