// PortBay — Tauri 2 + Rust core.

pub mod caddy;
pub mod mkcert;
pub mod process_compose;
pub mod registry;

use std::sync::Mutex;

use tauri::{Manager, State};

use crate::process_compose::{PcClient, Process, SidecarManager};

/// App-wide state. Lives behind `Mutex`es because Tauri's state is shared
/// across all commands and we mutate the sidecar manager from the setup
/// closure and the on-shutdown handler.
struct AppState {
    pc: Mutex<SidecarManager>,
    /// Cached client to the running daemon. None until setup has run.
    pc_client: Mutex<Option<PcClient>>,
}

#[tauri::command]
async fn pc_alive(state: State<'_, AppState>) -> Result<bool, String> {
    let client = state.pc_client.lock().unwrap().clone();
    let Some(client) = client else {
        return Ok(false);
    };
    client.live().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn pc_processes(state: State<'_, AppState>) -> Result<Vec<Process>, String> {
    let client = state
        .pc_client
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Process Compose daemon hasn't started yet".to_string())?;
    client.processes().await.map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            pc: Mutex::new(SidecarManager::new()),
            pc_client: Mutex::new(None),
        })
        .setup(|app| {
            let config_path =
                write_bootstrap_config().map_err(|e| -> Box<dyn std::error::Error> {
                    Box::<dyn std::error::Error>::from(e.to_string())
                })?;
            let state: State<AppState> = app.state();
            let client = {
                let mut pc = state.pc.lock().unwrap();
                pc.start(&app.handle(), &config_path).map_err(
                    |e| -> Box<dyn std::error::Error> {
                        Box::<dyn std::error::Error>::from(e.to_string())
                    },
                )?
            };
            *state.pc_client.lock().unwrap() = Some(client);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state: State<AppState> = window.state();
                state.pc.lock().unwrap().stop();
                *state.pc_client.lock().unwrap() = None;
            }
        })
        .invoke_handler(tauri::generate_handler![pc_processes, pc_alive])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Write a small placeholder PC config until the registry-driven generator
/// from `process_compose::config` is wired up to the Tauri commands.
///
/// TODO(phase-1): replace with `process_compose::config::to_yaml(&registry, ...)`
/// once the CLI / commands manage the registry lifecycle (kanban card
/// "P1 — CLI surface").
fn write_bootstrap_config() -> std::io::Result<std::path::PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("process-compose.bootstrap.yaml");
    let yaml = r#"version: "0.5"
processes:
  bootstrap:
    description: "Bootstrap process — replaced once the registry loads"
    command: "while true; do sleep 60; done"
    availability:
      restart: "no"
"#;
    std::fs::write(&path, yaml)?;
    Ok(path)
}
