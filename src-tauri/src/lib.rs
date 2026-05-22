// PortBay — Tauri 2 + Rust core.
// Spike scaffold: spawns process-compose as a sidecar at startup,
// exposes a Tauri command that queries its REST API.

pub mod registry;

use std::sync::Mutex;
use tauri::{Manager, State};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

const PC_PORT: u16 = 9999;

struct AppState {
    sidecar: Mutex<Option<CommandChild>>,
}

#[tauri::command]
async fn pc_processes() -> Result<serde_json::Value, String> {
    let url = format!("http://localhost:{}/processes", PC_PORT);
    let body = reqwest::get(&url)
        .await
        .map_err(|e| format!("request failed: {}", e))?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("parse failed: {}", e))?;
    Ok(body)
}

#[tauri::command]
async fn pc_alive() -> Result<bool, String> {
    let url = format!("http://localhost:{}/live", PC_PORT);
    match reqwest::get(&url).await {
        Ok(r) => Ok(r.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            sidecar: Mutex::new(None),
        })
        .setup(|app| {
            let config_path = write_bootstrap_config()?;
            let cmd = app
                .shell()
                .sidecar("process-compose")
                .expect("failed to create sidecar command")
                .args([
                    "-f",
                    &config_path,
                    "--port",
                    &PC_PORT.to_string(),
                    "--tui=false",
                    "--keep-project",
                    "up",
                ]);
            let (_rx, child) = cmd.spawn().expect("failed to spawn process-compose sidecar");
            let state: State<AppState> = app.state();
            *state.sidecar.lock().unwrap() = Some(child);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state: State<AppState> = window.state();
                let child = state.sidecar.lock().unwrap().take();
                if let Some(c) = child {
                    let _ = c.kill();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![pc_processes, pc_alive])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn write_bootstrap_config() -> Result<String, Box<dyn std::error::Error>> {
    let mut dir = dirs::data_dir().ok_or("no data dir")?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("process-compose.spike.yaml");
    let yaml = r#"version: "0.5"
processes:
  ping:
    description: "Long-running stub for the Tauri spike"
    command: "while true; do echo ping; sleep 5; done"
    availability:
      restart: "no"
"#;
    std::fs::write(&path, yaml)?;
    Ok(path.to_string_lossy().into_owned())
}
