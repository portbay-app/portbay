//! App-wide state held in `tauri::State`.
//!
//! Held behind `std::sync::Mutex` per Tauri 2 guidance: a guard MUST be
//! dropped before any `.await` in a command. `tokio::sync::Mutex` is only
//! needed when a guard needs to live across an await point — which our
//! design never does.
//!
//! The registry is *not* cached here — every command loads it from disk,
//! mutates, saves. Registry is small (<10 KB typical), loads in <1 ms, and
//! this matches the CLI's pattern so the two binaries can never drift.
//! See `bin/portbay.rs`'s `CliContext` for the parallel.

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::AppHandle;

use crate::caddy::lifecycle::CaddySidecar;
use crate::process_compose::{PcClient, SidecarManager};

pub struct AppState {
    /// On-disk path to the registry JSON. Resolved once at setup.
    pub registry_path: PathBuf,

    /// Domain suffix used when the registry doesn't exist yet (first run).
    pub domain_suffix: String,

    /// The bundled process-compose sidecar manager.
    pub pc: Mutex<SidecarManager>,

    /// Cached client to the running PC daemon. `None` until `setup` has
    /// successfully started the sidecar.
    pub pc_client: Mutex<Option<PcClient>>,

    /// The bundled caddy sidecar manager (will be wired up alongside PC
    /// once the caddy spawn lands in setup — currently dormant).
    pub caddy: Mutex<CaddySidecar>,
}

impl AppState {
    pub fn new(registry_path: PathBuf, domain_suffix: impl Into<String>) -> Self {
        Self {
            registry_path,
            domain_suffix: domain_suffix.into(),
            pc: Mutex::new(SidecarManager::new()),
            pc_client: Mutex::new(None),
            caddy: Mutex::new(CaddySidecar::new()),
        }
    }

    /// Borrow a cloned client. Returns `SidecarDown` when PC hasn't come up.
    /// Cloning is cheap — `reqwest::Client` is internally reference-counted.
    pub fn pc_client(&self) -> Result<PcClient, crate::error::AppError> {
        self.pc_client
            .lock()
            .expect("pc_client mutex poisoned")
            .clone()
            .ok_or(crate::error::AppError::SidecarDown("process-compose"))
    }

    /// Start (or restart) the bundled process-compose sidecar against the
    /// bootstrap config. Used by both `lib::run`'s setup and the
    /// `restart_pc` Tauri command — same code path either way so the
    /// cached client never desyncs from the actual child process.
    pub fn boot_pc(&self, app: &AppHandle) -> Result<(), crate::error::AppError> {
        let config_path = write_bootstrap_config()?;
        let client = self
            .pc
            .lock()
            .expect("pc mutex poisoned")
            .start(app, &config_path)?;
        *self.pc_client.lock().expect("pc_client mutex poisoned") = Some(client);
        Ok(())
    }

    /// Stop the bundled process-compose sidecar and clear the cached client.
    pub fn shutdown_pc(&self) {
        self.pc.lock().expect("pc mutex poisoned").stop();
        *self.pc_client.lock().expect("pc_client mutex poisoned") = None;
    }
}

/// Write a small placeholder PC config until the registry-driven generator
/// is wired up.
///
/// TODO(phase-2-reconcile): replace with
/// `process_compose::config::to_yaml(&registry, ...)` once the reconcile
/// loop lands as its own follow-up card.
pub fn write_bootstrap_config() -> std::io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("process-compose.bootstrap.yaml");
    let yaml = r#"version: "0.5"
processes:
  bootstrap:
    description: "Bootstrap process — replaced once the registry reconciler lands"
    command: "while true; do sleep 60; done"
    availability:
      restart: "no"
"#;
    std::fs::write(&path, yaml)?;
    Ok(path)
}
