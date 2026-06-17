// PortBay — Tauri 2 + Rust core.

pub mod adb_pair;
pub mod agents;
pub mod auth;
pub mod avatar;
pub mod caddy;
pub mod cli_install;
pub mod commands;
// Proprietary task board. Source injected from `portbay-cloud/desktop-pro` for
// official builds; absent from the public OSS tree (no `tasks` feature).
#[cfg(feature = "tasks")]
pub mod context;
pub mod databases;
pub mod db_approval;
pub mod db_client;
pub mod dictation;
pub mod dictation_anywhere;
pub mod dictation_commands;
pub mod dictation_context;
pub mod dictation_entities;
pub mod dictation_history;
pub mod dictation_session;
pub mod dictation_vocab;
pub mod dnsmasq;
pub mod dock_icon;
pub mod doctor;
pub mod domain;
pub mod entitlements;
pub mod error;
pub mod favicon;
pub mod flags;
pub mod hosts;
pub mod hosts_helper;
pub mod imagegen;
pub mod imageplayground;
pub mod import;
// Visual Preview Editor (Pro) — embedded live-preview webview. Shipped only
// in official builds; absent from the public OSS tree (no `visual-editor`
// feature).
pub mod install_proxy;
#[cfg(feature = "visual-editor")]
pub mod live_preview;
pub mod mailpit;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod mkcert;
pub mod mobile;
pub mod mobile_phase;
pub mod mobile_targets;
pub mod notifications;
pub mod ollama;
pub mod overlay_window;
pub mod php;
pub mod port_holder;
pub mod portfile;
pub mod preferences;
pub mod process_compose;
pub mod project_icon;
pub mod project_runtime;
pub mod reconciler;
pub mod registry;
#[cfg(feature = "tasks")]
pub mod run_stream;
pub mod runtimes;
pub mod sandbox;
pub mod sidecar_probe;
pub mod sidecar_reclaim;
pub mod smoke;
pub mod ssh;
pub mod state;
pub mod stt;
pub mod sync;
pub mod telemetry;
pub mod tray;
pub mod tunnel;
pub mod typing;
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

/// How often the SSH reconnect supervisor scans for dropped tunnels. Cheap when
/// idle (a mutex + per-tunnel liveness probe); a dropped auto-reconnect tunnel
/// becomes eligible for its first backed-off retry within one scan.
const SSH_SUPERVISOR_PERIOD: Duration = Duration::from_secs(2);

/// Install the global `tracing` subscriber. Idempotent — repeated calls
/// (e.g. from tests) silently no-op via `try_init`. Filter follows the
/// standard `PORTBAY_LOG` env var with an `info` default; the
/// `tauri_plugin_shell` and `reqwest` crates are quieted to `warn` so the
/// per-tick reconcile log isn't drowned in dependency noise.
///
/// `log_internal_errors(false)` matters: when a log write fails, the fmt
/// layer's default is to report that failure via `eprintln!` — which PANICS
/// with "failed printing to stderr: Broken pipe" once the stderr reader is
/// gone (closed terminal, dead `tauri dev` wrapper mid-restart). A logging
/// failure must never take the app down; the event is simply dropped.
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("PORTBAY_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tauri_plugin_shell=warn,reqwest=warn,hyper=warn"));
    let _ = fmt()
        .with_env_filter(filter)
        .log_internal_errors(false)
        .try_init();
}

/// Resolve the mkcert binary the reconciler should use. Tries, in order:
///
/// 1. **Next to the running executable** as plain `mkcert` — Tauri strips
///    the target-triple suffix when bundling `externalBin` sidecars, so a
///    packaged .app has `Contents/MacOS/mkcert` (and `tauri dev` copies it
///    to `target/debug/mkcert`). This is the production path.
/// 2. **Next to the running executable** with the triple suffix
///    (`mkcert-<target-triple>`) — covers a bare `cargo run` where the
///    Tauri CLI hasn't stripped the name.
/// 3. **Tauri resource directory** at `binaries/mkcert-<target-triple>` —
///    matches the `scripts/fetch-mkcert.sh` layout in `src-tauri/binaries`.
/// 4. **PATH** via `which::which("mkcert")` — final fallback for users
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

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Bundled sidecar: Tauri strips the triple suffix at bundle time.
            let candidate = dir.join("mkcert");
            if candidate.exists() {
                return Some(candidate);
            }
            if let Some(triple) = triple {
                let candidate = dir.join(format!("mkcert-{triple}"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    if let Some(triple) = triple {
        if let Ok(resource_dir) = app.path().resource_dir() {
            let candidate = resource_dir.join(format!("binaries/mkcert-{triple}"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    which::which("mkcert").ok()
}

/// Default location for per-process logs. `<data_dir>/PortBay/logs/`.
/// Created idempotently at setup; PC writes one file per project here.
/// Owner-only: project logs can carry request lines, hostnames, and env
/// echoes — other local users have no business reading them (0755 under
/// the default umask otherwise).
fn resolve_logs_dir() -> std::io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    dir.push("logs");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// Harden the `PortBay/` data dir at startup: owner-only on the directory
/// itself, and a sweep that fixes the mode of state files written by builds
/// that predate per-write 0600 (the registry + its migration backups carry
/// key paths, hosts/users, and proxy config; the tunnel state mirror carries
/// the full equivalent ssh command line). Best-effort — a failed chmod must
/// never block boot, and the per-write hardening still applies on next save.
#[cfg(unix)]
fn harden_data_dir(data_dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700));
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("registry.json") || name == "ssh-tunnels-state.json" {
            let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Strip PORTBAY_SESSION_JSON from the process environment before any
    // async workers or child processes are spawned. The value is captured
    // into a `OnceCell` in `auth` so `load_session()` can still honour it;
    // after this call the env var is gone from the live environment and every
    // child (process-compose, mkcert, node, project scripts) can't read it.
    crate::auth::capture_and_strip_session_env();

    init_tracing();
    telemetry::install_panic_hook(env!("CARGO_PKG_VERSION"));

    // Enrich the process PATH from the user's login shell without
    // blocking the UI thread. GUI launches on macOS inherit a minimal
    // PATH (no shell rc files run), so brew/asdf/mise/nvm installs are
    // invisible until we ask the user's shell for its PATH.
    //
    // Strategy: if a cache file exists from the previous run, apply it
    // synchronously (microseconds). The live login-shell probe runs on a
    // background thread and refreshes the cache when it completes. On a
    // first-ever launch there is no cache, so the process PATH stays at
    // the GUI-inherited minimum until the probe finishes in the
    // background — features that need the enriched PATH (runtime
    // detection, project spawn) are triggered by user action, which
    // happens well after the probe completes.
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

    let builder = builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Native file-drag-out for the permission sheet's drag-to-grant gesture.
        .plugin(tauri_plugin_drag::init());

    // pb-agent:// — the live-preview inspection agent's event channel
    // (agent.js fetch → Rust). Registered app-wide; only the preview child
    // webview ever loads the agent, so the surface stays inert elsewhere.
    #[cfg(feature = "visual-editor")]
    let builder = builder.register_uri_scheme_protocol("pb-agent", |ctx, request| {
        let app = ctx.app_handle().clone();
        live_preview::handle_agent_batch(&app, request.body());
        tauri::http::Response::builder()
            .status(200)
            .header("access-control-allow-origin", "*")
            .header("access-control-allow-headers", "content-type")
            .header("access-control-allow-methods", "POST, OPTIONS")
            .body(Vec::new())
            .expect("static response")
    });

    builder
        .setup(|app| {
            // Box-the-error helper — most fallible setup steps just
            // need their error stringified for Tauri's setup signature.
            fn boxed(e: impl std::fmt::Display) -> Box<dyn std::error::Error> {
                Box::<dyn std::error::Error>::from(e.to_string())
            }

            // If the previous boot was a fresh update that never confirmed it
            // launched cleanly, restore the prior known-good version before
            // anything else (this may relaunch into the restored build).
            commands::updater::rollback_on_startup(app.handle());

            let registry_path = store::default_path().map_err(boxed)?;
            let logs_dir = resolve_logs_dir().map_err(boxed)?;
            let yaml_path = reconciler::default_yaml_path().map_err(boxed)?;

            // Owner-only data dir + fix modes of state files from older
            // builds (they were world-readable in a 0755 dir).
            #[cfg(unix)]
            if let Some(data_dir) = registry_path.parent() {
                harden_data_dir(data_dir);
            }

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
            #[cfg(feature = "visual-editor")]
            app.manage(live_preview::LivePreviewState::default());
            #[cfg(feature = "visual-editor")]
            app.manage(live_preview::edit_proxy::EditModeState::default());

            // Boot the sidecars. PC against the just-written registry-
            // derived YAML; Caddy against its admin-only bootstrap.
            // `boot_caddy` is async (it polls the admin endpoint for
            // readiness) so we drive it with a block_on; the wait is
            // bounded by `CADDY_READINESS_TIMEOUT`.
            // Infra boot runs OFF the main thread (P0-3). The reclaim sweep
            // (`kill_gracefully` + `wait_port_released`, up to ~9 s if a crash
            // left stale sidecars) plus the Caddy readiness poll (up to 5 s) are
            // all synchronous; run inline in `setup()` they park the main thread,
            // which on a contended cold boot delays the first webview paint into
            // seconds of grey. Hoisting the whole sequence into a background task
            // lets the window paint immediately (the frontend's paint-aware
            // reveal then fires on time) while infra comes up behind it.
            //
            // The INTERNAL ORDER is unchanged — every step still `.await`s the
            // previous one inside this single task, so every incident-prevention
            // invariant still holds: reclaim frees :443 before Caddy binds; the
            // php-fpm socket is freed before PC reboots; the PC cache is primed
            // before the reconciler's first tick. Nothing after this in `setup()`
            // depends on boot having finished (the pollers poll over time, and
            // reopen-previous-projects already waits up to 15 s for the daemon).
            let boot_handle = app.handle().clone();
            let boot_yaml_path = yaml_path.clone();
            let boot_initial_yaml = initial_yaml.clone();
            tauri::async_runtime::spawn(async move {
                let state: tauri::State<AppState> = boot_handle.state();

                // Reap any process-compose left over from a previous run before
                // we boot our own. A crash or force-quit can orphan PC (and the
                // dev servers it supervised) to launchd; on a clean boot none of
                // ours should be running yet, so anything carrying our config
                // path is stale. SIGTERM-shutdown on quit handles the
                // prevent-leak half; this is the recover-on-boot half.
                let reaped = process_compose::lifecycle::sweep_stale(
                    &boot_yaml_path,
                    None,
                    process_compose::lifecycle::SweepMode::All,
                );
                if reaped > 0 {
                    tracing::info!(count = reaped, "reaped stale process-compose instances at boot");
                }

                // Same recover-on-boot half for cloudflared: a crash / SIGKILL
                // runs no `Drop`, so a quick tunnel can outlive the app. Reap any
                // cloudflared on our `--config` marker before a fresh share.
                let tunnels_reaped = tunnel::sweep_stale_cloudflared();
                if tunnels_reaped > 0 {
                    tracing::info!(count = tunnels_reaped, "reaped stale cloudflared tunnels at boot");
                }
                // Clear the cross-process tunnel mirror: nothing of ours is
                // tunneling at boot, so a stale file from a crashed prior run
                // must not make the CLI / MCP server report phantom tunnels.
                state.persist_tunnel_state();
                state.persist_ssh_tunnel_state();

                // Recover-on-boot for the sidecars spawned *directly* as app
                // children — caddy, dnsmasq, mailpit (+ php-fpm, a PC child the
                // stale-PC sweep above orphans). A crash / `tauri dev` rebuild
                // leaves them on launchd holding :443 / the DNS port / the SMTP
                // port / the FPM socket; the fresh stack then can't bind and
                // silently degrades (the ERR_SSL_PROTOCOL_ERROR / "php-fpm
                // already listening" incidents). Reaping them here — before we
                // boot our own — frees the canonical ports. For Caddy the
                // reclaim also waits for :443 to release before returning,
                // closing the kill→rebind race. Foreign instances (ServBay,
                // Homebrew) never match — the signature keys on our config paths.
                for kind in [
                    sidecar_reclaim::SidecarKind::Caddy,
                    sidecar_reclaim::SidecarKind::Dnsmasq,
                    sidecar_reclaim::SidecarKind::Mailpit,
                    sidecar_reclaim::SidecarKind::PhpFpm,
                ] {
                    sidecar_reclaim::reclaim_stale(kind, sidecar_reclaim::SweepMode::All);
                }

                // Advisory pidfile so the CLI / `portbay doctor` can tell the app
                // is live. The single-instance plugin is the real double-spawn
                // guard; this is just out-of-process visibility.
                sidecar_reclaim::write_pidfile();

                // Boot failures must not be fatal — log and run degraded. The
                // reconcile loop spawned below retries, and the frontend surfaces
                // a not-running sidecar from its status polling.
                if let Err(e) = state.boot_pc(&boot_handle, &boot_yaml_path) {
                    tracing::error!(error = %e, "process-compose failed to boot — degraded mode (reconciler will retry)");
                }

                if let Err(e) = state.boot_caddy(&boot_handle).await {
                    tracing::error!(error = %e, "caddy failed to boot — degraded mode (reconciler will retry)");
                }

                // Best-effort dnsmasq / Mailpit boot — useful background services
                // but not on any other sidecar's critical path.
                if let Err(e) = state.boot_dnsmasq(&boot_handle) {
                    tracing::warn!(error = %e, "dnsmasq sidecar did not start");
                }
                if let Err(e) = state.boot_mailpit(&boot_handle) {
                    tracing::warn!(error = %e, "mailpit sidecar did not start");
                }

                // Prime the PC sub-cache with the hash of the YAML we just booted
                // against — without this the first tick re-restarts the daemon
                // boot_pc spawned moments ago.
                state
                    .reconciler
                    .prime_pc_cache_from_yaml(&boot_initial_yaml)
                    .await;

                // Spawn the reconcile loop with an immediate first tick so the
                // registry-driven Caddy config + hosts + certs land alongside the
                // cold boot.
                state.reconciler.mark_dirty();
                reconciler::spawn_reconcile_loop(boot_handle.clone(), RECONCILE_SAFETY_PERIOD);
            });

            // Background SSH reconnect supervisor — restores dropped auto-reconnect
            // tunnels with exponential backoff regardless of whether the SSH page
            // is open, and never blocks an async worker (the reconnect spawn runs
            // on the blocking pool).
            commands::ssh_tunnels::spawn_ssh_supervisor(
                app.handle().clone(),
                SSH_SUPERVISOR_PERIOD,
            );

            // Spawn the status poller + metrics poller. Both run for the
            // lifetime of the app.
            commands::events::spawn_status_poller(app.handle().clone());
            commands::metrics::spawn_metrics_poller(app.handle().clone());
            // Mirror the global macOS dictation session (DictationIM's
            // distributed notifications) so start/stop_dictation can act on
            // the real OS state instead of blind-toggling. Setup runs on the
            // main thread, which the observer registration expects.
            dictation_session::init(app.handle());
            // "Dictate anywhere": make the notch overlay window inert
            // (click-through, all Spaces, never key) and install the global
            // Fn monitor when Accessibility trust already exists. Both are
            // main-thread AppKit work — setup is the right place.
            #[cfg(target_os = "macos")]
            {
                overlay_window::configure(app.handle());
                // Tauri runs `setup()` on the main thread today, so this marker
                // is normally present. Degrade gracefully rather than panic if a
                // future runtime change ever violates that: dictate-anywhere just
                // stays uninstalled instead of taking the whole app down on boot.
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    dictation_anywhere::init(app.handle(), mtm);
                } else {
                    tracing::error!(
                        "dictation: setup() not on the main thread — anywhere monitors skipped"
                    );
                }

                // Proactive grant nudge. "Dictate anywhere" needs Accessibility
                // trust to install its global Fn monitor — but TCC is keyed per
                // bundle/signature, so a fresh production install starts
                // untrusted even though the pref may have synced on from another
                // machine. Untrusted, the monitor never installs and the feature
                // is silently dead. We can't fire the system TCC prompt at launch
                // (a surprise permission dialog is forbidden) and we can't observe
                // the Fn key to nudge on use (macOS withholds key events from an
                // untrusted process) — so a desktop notification is the one
                // proactive surface left. It points at AI → Speech-to-Text, where
                // the in-app drag-to-grant sheet now auto-opens. Respects the
                // user's desktop-notification preference.
                {
                    let prefs = app.state::<AppState>().preferences_snapshot();
                    if prefs.dictation.anywhere
                        && prefs.desktop_notifications
                        && !crate::typing::ax_trusted()
                    {
                        use tauri_plugin_notification::NotificationExt;
                        let _ = app
                            .notification()
                            .builder()
                            .title("Dictate Anywhere needs Accessibility access")
                            .body("Open PortBay → AI → Speech-to-Text to grant it, then hold Fn in any app.")
                            .show();
                    }
                }
            }
            // Watch every project's audit log for new agent activity (comments,
            // blocks, warnings) and surface it to the topbar bell + desktop.
            // Board-only: the bell shell ships in every build, but the scanner
            // that feeds it is injected with the `tasks` feature.
            #[cfg(feature = "tasks")]
            notifications::spawn_scanner(app.handle().clone());

            // Tail every live run's transcript → `portbay://task-run-log`
            // events, for the agent panel's live stream and the board's
            // "latest action" line on running cards. Board-only.
            #[cfg(feature = "tasks")]
            run_stream::spawn_run_log_tailer(app.handle().clone());

            // Tail Caddy's JSON access log → `portbay://request` events for the
            // HTTP request inspector. Idle until Caddy writes its first entry.
            commands::http_inspector::spawn_request_tailer(app.handle().clone());

            // Background task-board sweep: reconcile per-project leases on boot
            // AND on a Rust-side timer, independent of the board UI being open.
            // The boot pass reclaims runs the app (or laptop) died under; the
            // recurring pass reclaims an agent that crashes while the board is
            // closed (the 3.5s frontend poll only runs with the board open) and
            // re-dispatches the persisted queue after a restart. Each pass also
            // prunes stale never-merged worktrees of long-Done/archived cards.
            // All file/process/git work, so it runs on a blocking thread — sync
            // work on the async workers stalls unrelated async commands.
            // Best-effort; never blocks startup. Board-only.
            #[cfg(feature = "tasks")]
            {
                let app_h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut tick =
                        tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        tick.tick().await; // first tick fires immediately = boot sweep
                        let (registry_path, domain_suffix) = {
                            let st: tauri::State<AppState> = app_h.state();
                            (st.registry_path.clone(), st.domain_suffix.clone())
                        };
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            let Ok(reg) = store::load_or_default(&registry_path, &domain_suffix)
                            else {
                                return;
                            };
                            for project in reg.list_projects() {
                                if !crate::context::automation::sweep_needed(&project.path) {
                                    continue;
                                }
                                let _ = crate::context::automation::reconcile(&reg, project);
                                let _ = crate::context::automation::drain_queue(project);
                                crate::context::automation::prune_stale_worktrees(&project.path);
                            }
                        })
                        .await;
                    }
                });
            }

            // Background build-artifact auto-clean. No-op unless the user opted
            // into a weekly/monthly cadence in Settings; the cadence gate lives
            // inside the scheduler.
            commands::artifacts::spawn_auto_clean_scheduler(app.handle().clone());

            // Reap idle SSH sessions. Cached exec/SFTP/agent sessions keep a
            // host authenticated so navigating the workspace doesn't re-prompt;
            // this drops any idle longer than 15 minutes (next action re-auths)
            // and any whose handle has already closed, so a host never holds an
            // open connection forever.
            {
                let app_h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    const MAX_IDLE: std::time::Duration = std::time::Duration::from_secs(15 * 60);
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
                    loop {
                        tick.tick().await;
                        let st: tauri::State<AppState> = app_h.state();
                        st.exec.lock().await.reap_idle(MAX_IDLE);
                        st.sftp.lock().await.reap_idle(MAX_IDLE);
                        st.agent.lock().await.reap_idle(MAX_IDLE);
                    }
                });
            }

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

                // Appearance-aware Dock icon: match the current Light/Dark
                // appearance now (read from NSApplication.effectiveAppearance),
                // and keep it in sync on the ThemeChanged window event below.
                // See `crate::dock_icon`.
                crate::dock_icon::apply();

                // Window reveal is driven primarily by the frontend
                // (`+layout.svelte`), which calls `.show()` only after the themed
                // UI has painted (two rAFs). The window is created hidden +
                // transparent, so revealing it before first paint shows macOS's
                // white/grey webview backing — the classic launch flash. On a
                // contended cold-boot autostart that "frame" can stretch into
                // seconds of grey, which is exactly what made autostart look
                // broken. So we do NOT show from Rust eagerly any more.
                //
                // Rust keeps only a *fallback*: after a grace period, if the
                // window is somehow still hidden (webview failed to load, or JS
                // errored before onMount), force it visible so it's never
                // stranded off-screen and reachable only via the tray. The grace
                // period lets the paint-aware frontend path win on every healthy
                // launch.
                let main_win_fallback = main_win.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
                    if !main_win_fallback.is_visible().unwrap_or(false) {
                        tracing::warn!(
                            "main window still hidden after grace period; forcing reveal"
                        );
                        let _ = main_win_fallback.show();
                        let _ = main_win_fallback.set_focus();
                    }
                });
            } else {
                tracing::error!(
                    "main webview window not found at setup — the app would appear \
                     with no visible window; check the window label in tauri.conf.json"
                );
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
                // Native OS file drop → record each path in the SFTP approved
                // set before the webview sees it. Drops are host-mediated (the
                // OS handed them to us, not the renderer), so recording them
                // here keeps drag-upload working under the dialog-approval
                // policy without giving the renderer a way to approve arbitrary
                // paths. The FileBrowserPane webview listener receives the same
                // paths and calls sftp_transfer / upload, which will pass the
                // ensure_local_path_approved check because we pre-inserted them.
                tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                    let state: tauri::State<AppState> = window.state();
                    let mut approved = state
                        .sftp_approved_paths
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    for path in paths {
                        if let Ok(canon) = std::fs::canonicalize(path) {
                            approved.insert(canon);
                        }
                    }
                }
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
            commands::projects::provision_python_env,
            commands::projects::clone_git_project_sandboxed,
            commands::projects::update_project,
            commands::projects::remove_project,
            commands::projects::detect_project,
            commands::projects::probe_readiness,
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
            commands::mobile::list_mobile_run_targets,
            commands::mobile::mobile_preflight,
            commands::mobile::get_mobile_phases,
            commands::mobile::mobile_hot_reload,
            commands::mobile::mobile_hot_restart,
            commands::mobile::open_mobile_simulator,
            commands::mobile::android_wifi_pair_start,
            commands::mobile::android_wifi_pair_manual,
            // Live-preview handlers — injected with the `visual-editor`
            // feature; absent from the public OSS build. `generate_handler!`
            // honours the per-entry #[cfg].
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_available,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_open,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_close,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_set_bounds,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_navigate,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_back,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_forward,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_reload,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_set_color_scheme,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_set_visible,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_set_select_mode,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_start_region_select,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_cancel_region_select,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_highlight,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_apply_style,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_set_text,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_refresh_context,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_get_console,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_clear_console,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_list_captures,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_read_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_delete_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_copy_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_export_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_save_annotated,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_create_card,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_preview_patch,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_apply_edits,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_load_session,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_save_session,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_history_list,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_history_preview,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_history_restore,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_detect_breakpoints,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_set_csp_proxy,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_watch_card,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_verify_edits,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_card_outcome,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_drag_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_pin_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_close_pin,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_ocr_capture,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_edit_mode_set,
            #[cfg(feature = "visual-editor")]
            commands::live_preview::live_preview_edit_mode_ids,
            commands::integrations::installed_dev_tools,
            commands::integrations::open_in_ide,
            commands::integrations::open_privacy_settings,
            commands::integrations::permission_drag_payload,
            commands::integrations::relaunch_app,
            commands::integrations::resolve_mcp_binary_path,
            commands::sidecars::sidecar_status,
            commands::sidecars::pc_alive,
            commands::sidecars::restart_pc,
            commands::sidecars::restart_caddy,
            commands::sidecars::reconcile_hosts,
            commands::certs::get_ca_status,
            commands::certs::install_mkcert_ca,
            commands::certs::cert_info,
            commands::certs::reissue_cert,
            commands::certs::export_cert_bundle,
            commands::webservers::webserver_overview,
            commands::system::doctor,
            commands::system::legal_notices,
            commands::system::tail_logs,
            commands::events::proc_log_history,
            commands::system::read_dotenv,
            commands::dbconn::project_db_connections,
            commands::artifacts::scan_artifacts,
            commands::artifacts::clean_artifact,
            commands::artifacts::clean_all_artifacts,
            commands::system::quit_app,
            commands::system::open_main_window,
            commands::system::start_dictation,
            commands::system::stop_dictation,
            commands::system::dictation_diagnostics,
            commands::dictation::dictation_rewrite,
            commands::dictation::dictation_edit,
            commands::dictation::dictation_rewrite_cancel,
            commands::dictation::dictation_trace,
            commands::dictation::dictation_provider_status,
            commands::dictation::dictation_prewarm,
            commands::dictation::dictation_unlearn,
            commands::dictation::dictation_reset_vocabulary,
            commands::dictation::dictation_list_apps,
            commands::stt::stt_status,
            commands::stt::stt_overview,
            commands::stt::stt_request_mic_access,
            commands::stt::stt_download_model,
            commands::stt::stt_cancel_download,
            commands::stt::stt_delete_model,
            commands::stt::stt_start_capture,
            commands::stt::stt_stop_capture,
            commands::stt::stt_cancel_capture,
            commands::stt::stt_prewarm,
            commands::tts::tts_overview,
            commands::tts::tts_download_model,
            commands::tts::tts_speak,
            commands::tts::tts_delete_model,
            commands::imagegen::imagegen_overview,
            commands::imagegen::imagegen_download_model,
            commands::imagegen::imagegen_cancel_download,
            commands::imagegen::imagegen_generate,
            commands::imagegen::imagegen_cancel_generate,
            commands::imagegen::imagegen_delete_model,
            commands::imageplayground::imageplayground_check,
            commands::imageplayground::imageplayground_generate,
            commands::imageplayground::imageplayground_open_app,
            commands::cli::cli_status,
            commands::cli::cli_install_tool,
            commands::cli::cli_uninstall_tool,
            commands::dictation_anywhere::dictation_anywhere_status,
            commands::dictation_anywhere::dictation_anywhere_arm,
            commands::dictation_anywhere::dictation_preview_cue,
            commands::dictation_anywhere::dictation_favicon_consent,
            commands::dictation_anywhere::dictation_favicon_consent_request,
            commands::dictation_anywhere::dictation_history_list,
            commands::dictation_anywhere::dictation_history_clear,
            commands::dictation_anywhere::dictation_overlay_stop,
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
            commands::ssh_tunnels::ssh_tunnel_list,
            commands::ssh_tunnels::ssh_tunnel_save,
            commands::ssh_tunnels::ssh_tunnel_delete,
            commands::ssh_tunnels::ssh_tunnel_start,
            commands::ssh_tunnels::ssh_tunnel_stop,
            commands::ssh_tunnels::ssh_tunnel_test,
            commands::ssh_tunnels::ssh_tunnel_open_database,
            commands::sftp::sftp_connect,
            commands::sftp::sftp_home_dir,
            commands::sftp::sftp_list_dir,
            commands::sftp::sftp_stat,
            commands::sftp::sftp_mkdir,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_remove_file,
            commands::sftp::sftp_remove_dir,
            commands::sftp::sftp_chmod,
            commands::sftp::sftp_read_text,
            commands::sftp::sftp_read_preview,
            commands::sftp::sftp_write_text,
            commands::sftp::sftp_upload,
            commands::sftp::sftp_download,
            commands::sftp::sftp_transfer,
            commands::sftp::sftp_transfer_cancel,
            commands::sftp::sftp_disconnect,
            commands::sftp::sftp_search,
            commands::sftp::sftp_search_cancel,
            // Host-side dialog pickers: the renderer calls these instead of the
            // plugin-dialog JS API so local paths are chosen by the host and
            // recorded in AppState::sftp_approved_paths before any transfer.
            commands::sftp::sftp_pick_upload_files,
            commands::sftp::sftp_pick_upload_dir,
            commands::sftp::sftp_pick_download_dir,
            commands::sftp::sftp_pick_save_path,
            commands::sftp::sftp_request_local_access,
            commands::ssh_exec::ssh_exec_run,
            commands::ssh_exec::ssh_deploy_run,
            commands::ssh_exec::ssh_deploy_cancel,
            commands::ssh_exec::ssh_deploy_snippets_get,
            commands::ssh_exec::ssh_deploy_snippets_set,
            commands::ssh_pty::ssh_pty_open,
            commands::ssh_pty::ssh_pty_input,
            commands::ssh_pty::ssh_pty_resize,
            commands::ssh_pty::ssh_pty_close,
            commands::ssh_agent::ssh_agent_open,
            commands::ssh_agent::ssh_agent_chat,
            commands::ssh_agent::ssh_agent_cli_chat,
            commands::ssh_agent::ssh_agent_run,
            commands::ssh_agent::ssh_ollama_complete,
            commands::ssh_agent::ssh_agent_upload_bytes,
            commands::ssh_agent::ssh_agent_upload_path,
            commands::ssh_agent::ssh_agent_cleanup_attachments,
            commands::ssh_agent::ssh_agent_forward_start,
            commands::ssh_agent::ssh_agent_forward_stop,
            commands::ssh_agent::ssh_agent_abort,
            commands::ssh_agent::ssh_agent_close,
            commands::ssh_agent::ssh_agent_threads_get,
            commands::ssh_agent::ssh_agent_threads_set,
            commands::ssh_connections::ssh_connections_list,
            commands::ssh_connections::ssh_connection_save,
            commands::ssh_connections::ssh_connection_delete,
            commands::ssh_connections::ssh_connection_detect_os,
            commands::ssh_connections::ssh_connection_probe,
            commands::ssh_connections::ssh_known_host_remove,
            crate::ssh::interaction::ssh_interaction_respond,
            crate::ssh::interaction::ssh_interaction_cancel,
            commands::ssh_connections::ssh_connection_touch,
            commands::ssh_connections::ssh_host_disconnect,
            commands::ssh_connections::ssh_host_connected,
            commands::ssh_connections::ssh_set_credential,
            commands::ssh_connections::ssh_clear_credential,
            commands::ssh_connections::ssh_has_stored_credential,
            commands::ssh_connections::ssh_forget_credentials,
            commands::ssh_connections::ssh_config_import,
            commands::ssh_identities::ssh_identities_list,
            commands::ssh_identities::ssh_identity_save,
            commands::ssh_identities::ssh_identity_delete,
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
            commands::projects::project_get_deploy,
            commands::projects::project_set_deploy,
            commands::deploy::project_deploy_run,
            commands::localfs::local_list_dir,
            commands::localfs::local_stat,
            commands::localfs::local_walk_files,
            commands::localfs::local_search,
            commands::metrics::system_metrics,
            commands::preferences::get_preferences,
            commands::preferences::set_preferences,
            commands::preferences::get_notification_prefs,
            commands::preferences::set_notification_prefs,
            commands::preferences::get_domain_settings,
            commands::preferences::update_domain_suffix,
            commands::preferences::mark_close_toast_seen,
            commands::ollama::ollama_overview,
            commands::ollama::ollama_running,
            commands::ollama::ollama_loaded_models,
            commands::ollama::ollama_start,
            commands::ollama::ollama_stop,
            commands::ollama::ollama_restart,
            commands::ollama::ollama_show_model,
            commands::ollama::ollama_delete_model,
            commands::ollama::ollama_unload_model,
            commands::ollama::ollama_smoke_test,
            commands::ollama::ollama_embed,
            commands::ollama::ollama_test_stream,
            commands::ollama::ollama_cancel_generate,
            commands::ollama::ollama_pull_model,
            commands::ollama::ollama_cancel_pull,
            commands::ollama::ollama_dismiss_pull,
            commands::ollama::ollama_install,
            commands::ollama::ollama_update_check,
            commands::ollama_library::ollama_library,
            commands::ollama_library::ollama_library_tags,
            commands::hwfit::hardware_profile,
            commands::entitlements::get_entitlement,
            commands::entitlements::refresh_entitlement,
            commands::entitlements::clear_entitlement,
            commands::entitlements::pro_checkout_url,
            commands::entitlements::subscription_status,
            commands::entitlements::billing_portal_url,
            commands::auth::begin_login,
            commands::auth::poll_login,
            commands::auth::cancel_login,
            commands::auth::logout,
            commands::auth::delete_account,
            commands::auth::account_status,
            commands::auth::cancel_account_deletion,
            commands::auth::export_account_data,
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
            commands::databases::database_client_schema,
            commands::databases::database_client_table_rows,
            commands::databases::database_client_query,
            commands::databases::database_client_explain,
            commands::databases::database_client_preview_writes,
            commands::databases::database_client_apply_writes,
            commands::databases::list_pending_db_writes,
            commands::databases::resolve_db_write,
            commands::telemetry::telemetry_settings,
            commands::telemetry::list_crash_reports,
            commands::telemetry::read_crash_report,
            commands::telemetry::discard_crash_report,
            commands::telemetry::send_crash_report,
            commands::telemetry::record_js_error,
            commands::telemetry::record_telemetry_event,
            commands::updater::check_for_update,
            commands::updater::install_update,
            commands::updater::confirm_update_health,
            // Per-project task board (Project Context & Task Authority).
            // Board-only handlers — injected with the `tasks` feature; absent
            // from the public OSS build. `generate_handler!` honours the per-
            // entry `#[cfg]`, so the public build registers none of these.
            #[cfg(feature = "tasks")]
            commands::tasks::tasks_list,
            #[cfg(feature = "tasks")]
            commands::tasks::task_get,
            #[cfg(feature = "tasks")]
            commands::tasks::task_card_path,
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
            commands::tasks::fetch_link_metadata,
            #[cfg(feature = "tasks")]
            commands::tasks::task_update,
            #[cfg(feature = "tasks")]
            commands::tasks::task_move,
            #[cfg(feature = "tasks")]
            commands::tasks::task_reorder,
            #[cfg(feature = "tasks")]
            commands::tasks::task_reorder_many,
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
            commands::tasks::task_archive_many,
            #[cfg(feature = "tasks")]
            commands::tasks::tasks_running_all,
            #[cfg(feature = "tasks")]
            commands::tasks::task_subscribe,
            #[cfg(feature = "tasks")]
            commands::tasks::task_attach,
            #[cfg(feature = "tasks")]
            commands::tasks::task_detach,
            #[cfg(feature = "tasks")]
            commands::tasks::task_attachment_path,
            #[cfg(feature = "tasks")]
            commands::tasks::open_attachment,
            #[cfg(feature = "tasks")]
            commands::tasks::task_attachment_data_url,
            #[cfg(feature = "tasks")]
            commands::tasks::ollama_local_models,
            #[cfg(feature = "tasks")]
            commands::tasks::agent_model_catalog,
            #[cfg(feature = "tasks")]
            commands::tasks::card_activity,
            #[cfg(feature = "tasks")]
            commands::tasks::task_run_log,
            #[cfg(feature = "tasks")]
            commands::tasks::task_branch,
            #[cfg(feature = "tasks")]
            commands::tasks::task_diff,
            #[cfg(feature = "tasks")]
            commands::tasks::task_commit,
            #[cfg(feature = "tasks")]
            commands::tasks::task_merge,
            #[cfg(feature = "tasks")]
            commands::tasks::task_duplicate,
            #[cfg(feature = "tasks")]
            commands::tasks::task_attempts_start,
            #[cfg(feature = "tasks")]
            commands::tasks::task_attempt_pick,
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
            commands::tasks::handoff_set_max_chars,
            #[cfg(feature = "tasks")]
            commands::tasks::learnings_show,
            #[cfg(feature = "tasks")]
            commands::tasks::learnings_replace,
            #[cfg(feature = "tasks")]
            commands::tasks::learnings_set_max_chars,
            #[cfg(feature = "tasks")]
            commands::tasks::tasks_watch,
            #[cfg(feature = "tasks")]
            commands::tasks::tasks_unwatch,
            #[cfg(feature = "tasks")]
            commands::tasks::project_mcp_status,
            #[cfg(feature = "tasks")]
            commands::tasks::setup_project_mcp,
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
