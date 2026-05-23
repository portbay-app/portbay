//! App-wide state held in `tauri::State`.
//!
//! Held behind `std::sync::Mutex` per Tauri 2 guidance: a guard MUST be
//! dropped before any `.await` in a command. `tokio::sync::Mutex` is only
//! needed when a guard needs to live across an await point — which most
//! of this struct's fields never do. The exception is the `reconciler`,
//! whose tick crosses await points and owns its own internal tokio
//! mutex (see `reconciler::Reconciler`).
//!
//! The registry is *not* cached here — every command loads it from disk,
//! mutates, saves. Registry is small (<10 KB typical), loads in <1 ms, and
//! this matches the CLI's pattern so the two binaries can never drift.
//! See `bin/portbay.rs`'s `CliContext` for the parallel.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use tauri::AppHandle;

use crate::caddy::{
    bootstrap_config, find_free_https_port, find_free_port, CaddyClient, CaddyError, CaddySidecar,
    ADMIN_SCAN_RANGE, DEFAULT_ADMIN_PORT, DEFAULT_HTTPS_PORT,
};
use crate::dnsmasq::{
    self, DnsmasqSidecar, DEFAULT_PORT as DNSMASQ_DEFAULT_PORT,
    PORT_SCAN_RANGE as DNSMASQ_PORT_SCAN_RANGE,
};
use crate::mailpit::{
    self, MailpitSidecar, DEFAULT_SMTP_PORT, DEFAULT_UI_PORT,
    PORT_SCAN_RANGE as MAILPIT_PORT_SCAN_RANGE,
};
use crate::mkcert::Mkcert;
use crate::process_compose::{PcClient, SidecarManager};
use crate::reconciler::Reconciler;
use crate::tunnel::TunnelManager;

/// How long `boot_caddy` polls the admin endpoint for readiness before
/// giving up. Caddy comes up well under 1 s on a warm bundle; 5 s leaves
/// generous headroom for first-launch xattr work on macOS.
const CADDY_READINESS_TIMEOUT: Duration = Duration::from_secs(5);

/// Poll interval inside the readiness window. 100 ms gives a snappy boot
/// while staying nowhere near the daemon's startup cost.
const CADDY_READINESS_POLL: Duration = Duration::from_millis(100);

pub struct AppState {
    /// On-disk path to the registry JSON. Resolved once at setup.
    pub registry_path: PathBuf,

    /// Domain suffix used when the registry doesn't exist yet (first run).
    pub domain_suffix: String,

    /// Per-process log directory; passed to PC's YAML generator and
    /// created on first use.
    pub logs_dir: PathBuf,

    /// The bundled process-compose sidecar manager.
    pub pc: Mutex<SidecarManager>,

    /// Cached client to the running PC daemon. `None` until `setup` has
    /// successfully started the sidecar.
    pub pc_client: Mutex<Option<PcClient>>,

    /// The bundled caddy sidecar manager.
    pub caddy: Mutex<CaddySidecar>,

    /// Cached client to the running Caddy daemon. `None` until `setup`
    /// (or a `restart_caddy` invocation) has started the sidecar.
    pub caddy_client: Mutex<Option<CaddyClient>>,

    /// Wrapper around the bundled mkcert binary. `None` if the binary
    /// could not be resolved at setup (degrades to "no cert issuance",
    /// surfaced via the mkcert sidecar slot).
    pub mkcert: Option<Mkcert>,

    /// The bundled dnsmasq sidecar manager. May not be running if no
    /// binary is available; the sidecar status row surfaces that state.
    pub dnsmasq: Mutex<DnsmasqSidecar>,

    /// The bundled Mailpit sidecar manager. Catches outgoing SMTP from
    /// local projects on the configured loopback port; status row
    /// surfaces the listening ports.
    pub mailpit: Mutex<MailpitSidecar>,

    /// Per-app Cloudflare Tunnel manager. Holds one cloudflared child
    /// per active project; replacing the manager on window-destroy
    /// kills any leaked children via `Drop`.
    pub tunnels: Mutex<TunnelManager>,

    /// Convergence engine — owns hash caches for the four sub-steps and
    /// the dirty-notify primitive the background loop awaits.
    pub reconciler: Reconciler,
}

impl AppState {
    pub fn new(
        registry_path: PathBuf,
        domain_suffix: impl Into<String>,
        logs_dir: PathBuf,
        mkcert: Option<Mkcert>,
        reconciler: Reconciler,
    ) -> Self {
        Self {
            registry_path,
            domain_suffix: domain_suffix.into(),
            logs_dir,
            pc: Mutex::new(SidecarManager::new()),
            pc_client: Mutex::new(None),
            caddy: Mutex::new(CaddySidecar::new()),
            caddy_client: Mutex::new(None),
            mkcert,
            dnsmasq: Mutex::new(DnsmasqSidecar::new()),
            mailpit: Mutex::new(MailpitSidecar::new()),
            tunnels: Mutex::new(TunnelManager::new()),
            reconciler,
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

    /// Borrow a cloned Caddy client. Returns `SidecarDown` when Caddy
    /// hasn't been booted yet. Reqwest internally reference-counts so
    /// cloning is essentially free.
    pub fn caddy_client(&self) -> Result<CaddyClient, crate::error::AppError> {
        self.caddy_client
            .lock()
            .expect("caddy_client mutex poisoned")
            .clone()
            .ok_or(crate::error::AppError::SidecarDown("caddy"))
    }

    /// Start (or restart) the bundled process-compose sidecar against
    /// the given config path. The reconciler is the canonical producer
    /// of that path — `lib::run::setup` writes the initial registry-
    /// derived YAML before this is first called, and the PC sub-
    /// reconciler rewrites it on every YAML-hash change.
    pub fn boot_pc(
        &self,
        app: &AppHandle,
        config_path: &Path,
    ) -> Result<(), crate::error::AppError> {
        let client = self
            .pc
            .lock()
            .expect("pc mutex poisoned")
            .start(app, config_path)?;
        *self.pc_client.lock().expect("pc_client mutex poisoned") = Some(client);
        Ok(())
    }

    /// Stop the bundled process-compose sidecar and clear the cached client.
    pub fn shutdown_pc(&self) {
        self.pc.lock().expect("pc mutex poisoned").stop();
        *self.pc_client.lock().expect("pc_client mutex poisoned") = None;
    }

    /// Start (or restart) the bundled Caddy sidecar against the bootstrap
    /// admin-only config, then poll `/config/` for readiness so the
    /// caller knows the daemon is actually accepting admin pushes by the
    /// time this returns. Used by both `lib::run`'s setup and the
    /// `restart_caddy` Tauri command.
    ///
    /// Errors:
    /// - `SidecarDown("caddy")` if the daemon doesn't accept admin
    ///   requests within [`CADDY_READINESS_TIMEOUT`]. The child process
    ///   is left running so the next `restart_caddy` can retry cleanly
    ///   without the lifecycle thinking the slot is free.
    pub async fn boot_caddy(&self, app: &AppHandle) -> Result<(), crate::error::AppError> {
        let admin_port =
            find_free_port(DEFAULT_ADMIN_PORT, ADMIN_SCAN_RANGE).ok_or(CaddyError::NoFreePort {
                start: DEFAULT_ADMIN_PORT,
            })?;
        let https_port = find_free_https_port(443, DEFAULT_HTTPS_PORT);
        let config_path = write_caddy_bootstrap_config(admin_port, https_port)?;

        let client = self.caddy.lock().expect("caddy mutex poisoned").start(
            app,
            &config_path,
            admin_port,
        )?;
        *self
            .caddy_client
            .lock()
            .expect("caddy_client mutex poisoned") = Some(client.clone());

        // Poll admin endpoint until the daemon responds. The sidecar
        // command line was already accepted; the child may still be in
        // its tls-cert-prep or listener-bind phase. Without this wait,
        // the first `POST /load` from the reconcile loop races and 500s.
        let deadline = std::time::Instant::now() + CADDY_READINESS_TIMEOUT;
        loop {
            if let Ok(true) = client.is_alive().await {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(crate::error::AppError::SidecarDown("caddy"));
            }
            tokio::time::sleep(CADDY_READINESS_POLL).await;
        }
    }

    /// Stop the bundled Caddy sidecar and clear the cached client.
    pub fn shutdown_caddy(&self) {
        self.caddy.lock().expect("caddy mutex poisoned").stop();
        *self
            .caddy_client
            .lock()
            .expect("caddy_client mutex poisoned") = None;
    }

    /// Start the dnsmasq sidecar against the registry's domain suffix.
    /// Best-effort: if the binary isn't available, this returns Ok and
    /// the sidecar status surface flags it as NotInstalled. dnsmasq is
    /// not yet on the critical path — until the resolver-file install
    /// command lands, no production queries flow through it.
    pub fn boot_dnsmasq(&self, app: &AppHandle) -> Result<(), crate::error::AppError> {
        if !dnsmasq::binary_available(app) {
            return Ok(());
        }
        let port = dnsmasq::find_free_port(DNSMASQ_DEFAULT_PORT, DNSMASQ_PORT_SCAN_RANGE).ok_or(
            crate::dnsmasq::DnsmasqError::NoFreePort {
                start: DNSMASQ_DEFAULT_PORT,
            },
        )?;
        let config_path = dnsmasq::write_config(&self.domain_suffix, port)?;
        self.dnsmasq
            .lock()
            .expect("dnsmasq mutex poisoned")
            .start(app, &config_path, port)?;
        Ok(())
    }

    /// Stop the dnsmasq sidecar. Idempotent.
    pub fn shutdown_dnsmasq(&self) {
        self.dnsmasq.lock().expect("dnsmasq mutex poisoned").stop();
    }

    /// Start the Mailpit sidecar with SMTP + web UI listeners on
    /// loopback. Best-effort: missing binary returns Ok and the status
    /// surface flags it as NotInstalled.
    pub fn boot_mailpit(&self, app: &AppHandle) -> Result<(), crate::error::AppError> {
        if !mailpit::binary_available(app) {
            return Ok(());
        }
        let smtp = mailpit::find_free_port(DEFAULT_SMTP_PORT, MAILPIT_PORT_SCAN_RANGE).ok_or(
            crate::mailpit::MailpitError::NoFreePort {
                start: DEFAULT_SMTP_PORT,
            },
        )?;
        let ui = mailpit::find_free_port(DEFAULT_UI_PORT, MAILPIT_PORT_SCAN_RANGE).ok_or(
            crate::mailpit::MailpitError::NoFreePort {
                start: DEFAULT_UI_PORT,
            },
        )?;
        let db_path = mailpit::lifecycle::default_db_path()?;
        self.mailpit
            .lock()
            .expect("mailpit mutex poisoned")
            .start(app, smtp, ui, &db_path)?;
        Ok(())
    }

    /// Stop the Mailpit sidecar. Idempotent.
    pub fn shutdown_mailpit(&self) {
        self.mailpit.lock().expect("mailpit mutex poisoned").stop();
    }
}

/// Serialise the minimal admin-only Caddy config and write it to the
/// PortBay app-data directory. The reconcile loop overwrites this via
/// `POST /load` once projects exist; the file on disk is only relevant at
/// boot.
pub fn write_caddy_bootstrap_config(admin_port: u16, https_port: u16) -> std::io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("caddy.bootstrap.json");
    let cfg = bootstrap_config(admin_port, https_port);
    let json = serde_json::to_vec_pretty(&cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caddy_bootstrap_config_is_written_with_expected_ports() {
        let path = write_caddy_bootstrap_config(2099, 18443).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["admin"]["listen"], "localhost:2099");
        let listen = &v["apps"]["http"]["servers"]["portbay"]["listen"];
        assert_eq!(listen[0], ":18443");
    }
}
