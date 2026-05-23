// PortBay — Tauri 2 + Rust core.

pub mod caddy;
pub mod commands;
pub mod dnsmasq;
pub mod error;
pub mod hosts;
pub mod import;
pub mod mailpit;
pub mod mkcert;
pub mod process_compose;
pub mod reconciler;
pub mod registry;
pub mod state;

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::mkcert::Mkcert;
use crate::reconciler::Reconciler;
use crate::registry::store;
use crate::state::AppState;

/// Domain suffix used when the registry doesn't yet exist on disk.
/// Matches the CLI's default (`bin/portbay.rs::CliContext::load_registry`).
const DEFAULT_DOMAIN_SUFFIX: &str = "test";

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

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Box-the-error helper — most fallible setup steps just
            // need their error stringified for Tauri's setup signature.
            fn boxed(e: impl std::fmt::Display) -> Box<dyn std::error::Error> {
                Box::<dyn std::error::Error>::from(e.to_string())
            }

            let registry_path = store::default_path().map_err(boxed)?;
            let logs_dir = resolve_logs_dir().map_err(boxed)?;
            let yaml_path = reconciler::default_yaml_path().map_err(boxed)?;

            // Build the initial PC YAML from the registry so PC boots
            // against the real config rather than the deleted bootstrap
            // placeholder. Empty registry → empty `processes: {}`, PC
            // starts and waits.
            let initial_registry =
                store::load_or_default(&registry_path, DEFAULT_DOMAIN_SUFFIX).map_err(boxed)?;
            let initial_yaml =
                reconciler::build_initial_yaml(&initial_registry, &logs_dir).map_err(boxed)?;
            std::fs::write(&yaml_path, &initial_yaml).map_err(boxed)?;

            // Resolve mkcert; None is a tolerable degraded state.
            let mkcert_binary = resolve_mkcert_binary(&app.handle());
            let mkcert = mkcert_binary
                .as_ref()
                .and_then(|p| Mkcert::default_in_data_dir(p.clone()));

            let reconciler = Reconciler::new(logs_dir.clone(), yaml_path.clone());

            app.manage(AppState::new(
                registry_path,
                DEFAULT_DOMAIN_SUFFIX,
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
            state.boot_pc(&app.handle(), &yaml_path).map_err(boxed)?;

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
            if let Err(e) = state.boot_dnsmasq(&app.handle()) {
                tracing::warn!(error = %e, "dnsmasq sidecar did not start");
            }

            // Best-effort Mailpit boot. Same degraded-mode story as
            // dnsmasq: useful for catching outgoing SMTP from local
            // projects, but not on the critical path of any other
            // sidecar.
            if let Err(e) = state.boot_mailpit(&app.handle()) {
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
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state: tauri::State<AppState> = window.state();
                state.shutdown_pc();
                state.shutdown_caddy();
                state.shutdown_dnsmasq();
                state.shutdown_mailpit();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::list_projects,
            commands::projects::get_project,
            commands::projects::add_project,
            commands::projects::update_project,
            commands::projects::remove_project,
            commands::projects::detect_project,
            commands::projects::validate_project_folder,
            commands::lifecycle::start_project,
            commands::lifecycle::stop_project,
            commands::lifecycle::restart_project,
            commands::lifecycle::stop_all,
            commands::lifecycle::open_project,
            commands::integrations::installed_dev_tools,
            commands::integrations::open_in_ide,
            commands::sidecars::sidecar_status,
            commands::sidecars::pc_alive,
            commands::sidecars::restart_pc,
            commands::sidecars::restart_caddy,
            commands::sidecars::reconcile_hosts,
            commands::certs::install_mkcert_ca,
            commands::certs::cert_info,
            commands::certs::reissue_cert,
            commands::system::doctor,
            commands::system::tail_logs,
            commands::log_stream::subscribe_logs,
            commands::import::detect_sources,
            commands::import::preview_import,
            commands::import::import_projects,
            commands::dnsmasq::dnsmasq_resolver_status,
            commands::dnsmasq::dnsmasq_install_resolver,
            commands::dnsmasq::dnsmasq_uninstall_resolver,
            commands::dnsmasq::restart_dnsmasq,
            commands::metrics::system_metrics,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
