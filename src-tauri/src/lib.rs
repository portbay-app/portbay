// PortBay — Tauri 2 + Rust core.

pub mod caddy;
pub mod commands;
pub mod error;
pub mod hosts;
pub mod mkcert;
pub mod process_compose;
pub mod registry;
pub mod state;

use tauri::Manager;

use crate::registry::store;
use crate::state::AppState;

/// Domain suffix used when the registry doesn't yet exist on disk.
/// Matches the CLI's default (`bin/portbay.rs::CliContext::load_registry`).
const DEFAULT_DOMAIN_SUFFIX: &str = "test";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Resolve the registry location once; commands read it from state.
            let registry_path = store::default_path().map_err(|e| -> Box<dyn std::error::Error> {
                Box::<dyn std::error::Error>::from(e.to_string())
            })?;

            app.manage(AppState::new(registry_path, DEFAULT_DOMAIN_SUFFIX));
            app.manage(commands::metrics::MetricsState::new());

            // Start the process-compose and caddy sidecars via the
            // shared helpers — same code path as the `restart_*` Tauri
            // commands so the cached clients never desync from the
            // spawned children. `boot_caddy` is async (it polls the
            // admin endpoint for readiness) so we drive it with a
            // block_on; the wait is bounded by `CADDY_READINESS_TIMEOUT`.
            let state: tauri::State<AppState> = app.state();
            state.boot_pc(&app.handle()).map_err(|e| -> Box<dyn std::error::Error> {
                Box::<dyn std::error::Error>::from(e.to_string())
            })?;

            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(async {
                let state: tauri::State<AppState> = app_handle.state();
                state.boot_caddy(&app_handle).await
            })
            .map_err(|e| -> Box<dyn std::error::Error> {
                Box::<dyn std::error::Error>::from(e.to_string())
            })?;

            // Spawn the status poller + metrics poller. Both run for the
            // lifetime of the app.
            commands::events::spawn_status_poller(app.handle().clone());
            commands::metrics::spawn_metrics_poller(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state: tauri::State<AppState> = window.state();
                state.shutdown_pc();
                state.shutdown_caddy();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::list_projects,
            commands::projects::get_project,
            commands::projects::add_project,
            commands::projects::update_project,
            commands::projects::remove_project,
            commands::projects::detect_project,
            commands::lifecycle::start_project,
            commands::lifecycle::stop_project,
            commands::lifecycle::restart_project,
            commands::lifecycle::stop_all,
            commands::lifecycle::open_project,
            commands::sidecars::sidecar_status,
            commands::sidecars::pc_alive,
            commands::sidecars::restart_pc,
            commands::sidecars::restart_caddy,
            commands::sidecars::reconcile_hosts,
            commands::system::doctor,
            commands::system::tail_logs,
            commands::metrics::system_metrics,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

