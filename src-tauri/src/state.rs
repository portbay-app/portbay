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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
use crate::preferences::Preferences;
use crate::process_compose::{PcClient, SidecarManager};
use crate::reconciler::Reconciler;
use crate::registry::store;
use crate::tray::TrayState;
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

    /// User-visible behavioural toggles (tray visibility, close-to-menubar).
    /// Held in a mutex so the close-window handler and the Tauri commands
    /// can both read/write without crossing await points.
    pub preferences: Mutex<Preferences>,

    /// Menu-bar tray icon handle + change-gate metadata. `None` when the
    /// user has disabled the tray via preferences.
    pub tray: TrayState,

    /// Recently-requested explicit Stop operations, keyed by project id.
    /// When the user clicks Stop, we record the timestamp here; the
    /// status poller checks this map before classifying an exit as a
    /// crash. Wrapping tools (npm, turbo) often translate SIGTERM into
    /// `exit(1)`, which would otherwise paint a clean stop as a red
    /// Crashed badge. Entries older than `STOP_INTENT_WINDOW` are
    /// considered stale and ignored.
    pub stop_intents: Mutex<HashMap<String, Instant>>,

    /// Set once [`AppState::shutdown_all`] has run, so the teardown is
    /// idempotent across the multiple quit signals Tauri can deliver (the
    /// window `Destroyed` event AND the app-level `RunEvent::Exit`).
    shutdown_done: AtomicBool,

    /// In-flight account login. Holds the opaque poll token for the pending
    /// `/auth/session/*` handshake so the frontend can poll without ever
    /// seeing tokens; cleared when the login completes or expires. Tokens
    /// themselves never live here — they go straight to the OS keychain.
    pub pending_login: Mutex<Option<crate::auth::PendingLogin>>,
}

/// How long after a Stop request a non-zero exit is still considered
/// the result of that stop. Long enough for the child to fully wind
/// down (npm post-hooks, file watchers), short enough that a genuine
/// crash a minute later isn't misclassified as a clean stop.
pub const STOP_INTENT_WINDOW: Duration = Duration::from_secs(15);

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
            preferences: Mutex::new(Preferences::load()),
            tray: Mutex::new(Default::default()),
            stop_intents: Mutex::new(HashMap::new()),
            shutdown_done: AtomicBool::new(false),
            pending_login: Mutex::new(None),
        }
    }

    /// Snapshot the current preferences. Returns by value so the lock
    /// is released before the caller does anything async.
    pub fn preferences_snapshot(&self) -> Preferences {
        self.preferences
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Record that the user just asked PortBay to stop this project.
    /// The status poller consults this map before classifying the next
    /// exit as a crash, so a clean Stop never gets painted red even
    /// when the child runtime exits with a non-zero code.
    pub fn mark_stop_requested(&self, project_id: &str) {
        let mut guard = self.stop_intents.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(project_id.to_string(), Instant::now());
        // Garbage-collect entries older than the intent window so the
        // map can't grow unboundedly in long-running sessions.
        guard.retain(|_, ts| ts.elapsed() < STOP_INTENT_WINDOW);
    }

    /// True if Stop was requested for this project recently enough that
    /// the next observed exit should still be attributed to that Stop.
    pub fn recently_stop_requested(&self, project_id: &str) -> bool {
        let guard = self.stop_intents.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(project_id)
            .map(|ts| ts.elapsed() < STOP_INTENT_WINDOW)
            .unwrap_or(false)
    }

    /// Ports declared by registered projects (primary + extra). Sidecars feed
    /// this into their free-port scan so a dynamic sidecar port never lands on a
    /// port a dev server expects.
    fn registered_project_ports(&self) -> Vec<u16> {
        store::load_or_default(&self.registry_path, &self.domain_suffix)
            .map(|reg| {
                reg.list_projects()
                    .iter()
                    .flat_map(|p| p.port.into_iter().chain(p.extra_ports.iter().copied()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Borrow a cloned client. Returns `SidecarDown` when PC hasn't come up.
    /// Cloning is cheap — `reqwest::Client` is internally reference-counted.
    pub fn pc_client(&self) -> Result<PcClient, crate::error::AppError> {
        self.pc_client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or(crate::error::AppError::SidecarDown("process-compose"))
    }

    /// Borrow a cloned Caddy client. Returns `SidecarDown` when Caddy
    /// hasn't been booted yet. Reqwest internally reference-counts so
    /// cloning is essentially free.
    pub fn caddy_client(&self) -> Result<CaddyClient, crate::error::AppError> {
        self.caddy_client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
        let avoid = self.registered_project_ports();
        let client = self
            .pc
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(app, config_path, &avoid)?;
        *self.pc_client.lock().unwrap_or_else(|e| e.into_inner()) = Some(client);
        Ok(())
    }

    /// Stop the bundled process-compose sidecar and clear the cached client.
    pub fn shutdown_pc(&self) {
        self.pc.lock().unwrap_or_else(|e| e.into_inner()).stop();
        *self.pc_client.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        let avoid = self.registered_project_ports();
        let admin_port =
            find_free_port(DEFAULT_ADMIN_PORT, ADMIN_SCAN_RANGE, &avoid).ok_or(
                CaddyError::NoFreePort {
                    start: DEFAULT_ADMIN_PORT,
                },
            )?;
        let https_port = find_free_https_port(443, DEFAULT_HTTPS_PORT, &avoid);
        let config_path = write_caddy_bootstrap_config(admin_port, https_port)?;

        let client = self.caddy.lock().unwrap_or_else(|e| e.into_inner()).start(
            app,
            &config_path,
            admin_port,
        )?;
        *self.caddy_client.lock().unwrap_or_else(|e| e.into_inner()) = Some(client.clone());

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
        self.caddy.lock().unwrap_or_else(|e| e.into_inner()).stop();
        *self.caddy_client.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        let avoid = self.registered_project_ports();
        let port = dnsmasq::find_free_port(DNSMASQ_DEFAULT_PORT, DNSMASQ_PORT_SCAN_RANGE, &avoid)
            .ok_or(crate::dnsmasq::DnsmasqError::NoFreePort {
                start: DNSMASQ_DEFAULT_PORT,
            })?;
        // The registry is the source of truth for both the wildcard suffix
        // and the tunable dnsmasq settings; `self.domain_suffix` is only the
        // first-run fallback. Reading it here means a suffix migration or a
        // settings change is picked up on the next boot/restart.
        let reg = store::load_or_default(&self.registry_path, &self.domain_suffix)?;
        let config_path = dnsmasq::write_config(&reg.domain_suffix, port, &reg.dnsmasq)?;
        self.dnsmasq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(app, &config_path, port)?;

        // Drift guard: dnsmasq picks a fresh free port on every boot, but
        // `/etc/resolver/<suffix>` is written once and would otherwise keep
        // pointing at a now-dead port after a restart. If DNS routing was
        // previously set up (the file exists) and our privileged helper is
        // reachable, silently re-point the file at the port we just bound.
        // Best-effort — if this fails, PortBay falls back to exact /etc/hosts
        // entries on the next reconcile tick.
        if crate::dnsmasq::resolver::read_installed(&reg.domain_suffix).is_some() {
            let helper = crate::hosts_helper::HostsHelperClient::system();
            if helper.is_available() {
                let _ = helper.install_resolver(&reg.domain_suffix, port);
            }
        }
        Ok(())
    }

    /// Stop the dnsmasq sidecar. Idempotent.
    pub fn shutdown_dnsmasq(&self) {
        self.dnsmasq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop();
    }

    /// Start the Mailpit sidecar with SMTP + web UI listeners on
    /// loopback. Best-effort: missing binary returns Ok and the status
    /// surface flags it as NotInstalled.
    pub fn boot_mailpit(&self, app: &AppHandle) -> Result<(), crate::error::AppError> {
        if !mailpit::binary_available(app) {
            return Ok(());
        }
        // Never claim a port a registered project expects. Mailpit's default
        // ranges (1025–1040 / 8025–8040) overlap common dev-server ports, so we
        // feed the registry's project ports (incl. extra_ports) into the scan.
        let avoid = self.registered_project_ports();
        let smtp = mailpit::find_free_port(DEFAULT_SMTP_PORT, MAILPIT_PORT_SCAN_RANGE, &avoid)
            .ok_or(crate::mailpit::MailpitError::NoFreePort {
                start: DEFAULT_SMTP_PORT,
            })?;
        // Also avoid the SMTP port for the UI scan (defensive — ranges differ).
        let ui_avoid: Vec<u16> = avoid.iter().copied().chain(std::iter::once(smtp)).collect();
        let ui = mailpit::find_free_port(DEFAULT_UI_PORT, MAILPIT_PORT_SCAN_RANGE, &ui_avoid)
            .ok_or(crate::mailpit::MailpitError::NoFreePort {
                start: DEFAULT_UI_PORT,
            })?;
        let db_path = mailpit::lifecycle::default_db_path()?;
        self.mailpit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start(app, smtp, ui, &db_path)?;
        Ok(())
    }

    /// Stop the Mailpit sidecar. Idempotent.
    pub fn shutdown_mailpit(&self) {
        self.mailpit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop();
    }

    /// Tear down everything PortBay started — Process Compose (which stops
    /// every project it supervises), Caddy, dnsmasq, Mailpit, and all
    /// Cloudflare tunnels — in one place. Idempotent: the first call wins and
    /// flips `shutdown_done`; later calls are no-ops. This matters because a
    /// single quit can deliver more than one teardown signal (the main
    /// window's `Destroyed` event AND the app-level `RunEvent::Exit`), and we
    /// must guarantee the sweep runs exactly once on *every* quit path —
    /// ⌘Q, the tray "Quit" item, or the last window closing — not only the
    /// one the old window-event handler caught.
    pub fn shutdown_all(&self) {
        if self.shutdown_done.swap(true, Ordering::SeqCst) {
            return;
        }
        tracing::info!("shutdown: stopping process-compose, sidecars, and tunnels");
        // PC first: its SIGTERM cascades to the dev servers it supervises, so
        // they wind down before we cut the routing layer out from under them.
        self.shutdown_pc();
        self.shutdown_caddy();
        self.shutdown_dnsmasq();
        self.shutdown_mailpit();
        // Replace the tunnel manager so its `Drop` kills every cloudflared
        // child — nothing PortBay spawned should outlive the app.
        *self.tunnels.lock().unwrap_or_else(|e| e.into_inner()) =
            crate::tunnel::TunnelManager::new();
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
