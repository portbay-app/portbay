//! Process Compose sub-reconciler.
//!
//! Generates the YAML the registry implies, hashes it, and restarts PC
//! against the new file only when the hash differs from the last
//! applied. PC's REST API does not support adding processes to a
//! running daemon — every YAML mutation costs one restart. This is a
//! known upstream constraint, called out in the kanban Outcome.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::AppHandle;

use crate::process_compose;
use crate::reconciler::report::StepOutcome;
use crate::registry::Registry;
use crate::state::AppState;

/// How long to wait for a freshly-rebooted process-compose to report live
/// before giving up and letting the tick continue. PC parses its YAML in a
/// few hundred ms; 5 s is generous headroom on a busy machine. Mirrors
/// `CADDY_READINESS_TIMEOUT`.
const PC_READINESS_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll cadence while waiting for the rebooted daemon's `/live` to answer 200.
const PC_READINESS_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub(super) struct PcCache {
    /// Hash of the last YAML successfully written + booted against.
    last_applied: Option<u64>,
    /// Database skip-warnings already emitted, keyed `"<id>:<reason>"`, so a
    /// persistent bad state (missing binary, unprovisioned data dir) logs once
    /// instead of on every safety tick. Rebuilt each pass: an instance that
    /// heals or is removed drops out, re-arming its warning.
    db_skips_warned: std::collections::HashSet<String>,
}

impl PcCache {
    pub(super) fn prime(&mut self, yaml_hash: u64) {
        self.last_applied = Some(yaml_hash);
    }
}

/// Apply the registry-derived YAML to PC. On first call (cache empty),
/// the YAML is always written + PC restarted so the in-process state
/// matches whatever's on disk. Subsequent calls short-circuit if the
/// hash is unchanged.
pub(super) async fn reconcile(
    reg: &Registry,
    logs_dir: &Path,
    yaml_path: &Path,
    state: &AppState,
    app: &AppHandle,
    cache: &mut PcCache,
) -> StepOutcome {
    let mail_env = mail_env_from_state(state);
    let data_dir = data_dir_from_logs(logs_dir);
    let php_fpm_specs = php_fpm_specs_for(reg, data_dir);
    let db_specs = db_daemon_specs_for(reg, data_dir, &mut cache.db_skips_warned);
    let web_specs = crate::webservers::specs_for(reg, data_dir, logs_dir);
    let yaml = match process_compose::config::to_yaml(
        reg,
        logs_dir,
        mail_env.as_ref(),
        &php_fpm_specs,
        &db_specs,
        &web_specs,
        state.preferences_snapshot().store_logs_locally,
    ) {
        Ok(y) => y,
        Err(e) => return StepOutcome::failed(format!("yaml generation: {e}")),
    };

    let hash = hash_string(&yaml);
    if cache.last_applied == Some(hash) {
        return StepOutcome::skipped("yaml unchanged");
    }

    if let Err(e) = std::fs::write(yaml_path, &yaml) {
        return StepOutcome::failed(format!("write {}: {}", yaml_path.display(), e));
    }

    // `shutdown_pc` sleeps ~800 ms (graceful drain) and `boot_pc` blocks until
    // the daemon is live (readiness poll) — both synchronous. `reconcile` runs
    // ON the async reconciler worker (`tick().await`), so running this inline
    // parks that worker for ~1 s+ on every YAML change, starving unrelated async
    // commands scheduled on the same thread. `block_in_place` preserves the
    // exact ordering and the "PC is serving the new YAML" postcondition (the
    // readiness poll still completes before we return) while letting the runtime
    // relocate other tasks off this thread for the duration.
    let boot = tokio::task::block_in_place(|| {
        state.shutdown_pc();
        state.boot_pc(app, yaml_path)
    });
    if let Err(e) = boot {
        return StepOutcome::failed(format!("boot pc: {e}"));
    }

    // `boot_pc` returns the moment the child is spawned, but process-compose
    // needs a beat to parse the YAML and register its processes before the REST
    // API will accept `/process/start/{name}` — until then it answers those with
    // `400 no such process`. Every start path (`start_project`,
    // `start_project_sandboxed`, `restart_project`, `force_start_project`) awaits
    // this tick *before* issuing the start, so blocking here until the daemon is
    // live makes "PC is serving the new YAML" the tick's postcondition and closes
    // the reboot→start race. Mirrors the readiness poll `boot_caddy` already does
    // for the identical reason. On timeout we proceed anyway: a genuinely failed
    // PC will surface a precise error on the subsequent start call rather than
    // wedging the whole reconcile (which also drives Caddy, hosts, and dnsmasq).
    if let Ok(client) = state.pc_client() {
        let deadline = std::time::Instant::now() + PC_READINESS_TIMEOUT;
        while !client.live().await.unwrap_or(false) {
            if std::time::Instant::now() >= deadline {
                tracing::warn!(
                    target: "reconciler",
                    "process-compose did not report live within {PC_READINESS_TIMEOUT:?} after reboot"
                );
                break;
            }
            tokio::time::sleep(PC_READINESS_POLL).await;
        }
    }

    cache.last_applied = Some(hash);
    StepOutcome::applied(format!("{} process(es)", count_processes(reg)))
}

/// Public helper exposed to `lib.rs::setup` so the cold-boot path can
/// produce the initial YAML before `boot_pc` is called the first time.
/// Mailpit hasn't booted yet at this point in setup, so the initial
/// YAML always emits without Mail* env vars; the first reconcile tick
/// after Mailpit comes up will regenerate the YAML with the
/// injections and restart PC once.
pub fn build_initial_yaml(reg: &Registry, logs_dir: &Path) -> Result<String, String> {
    // Cold boot: default to writing logs (true). The first reconcile tick applies
    // the user's actual `store_logs_locally` setting.
    process_compose::config::to_yaml(reg, logs_dir, None, &[], &[], &[], true)
        .map_err(|e| e.to_string())
}

/// The PortBay app-data directory — `<logs_dir>/..`. Used to derive
/// per-PHP-version pool/socket paths so Caddy and PC agree on a
/// single location.
fn data_dir_from_logs(logs_dir: &Path) -> &Path {
    logs_dir.parent().unwrap_or(logs_dir)
}

/// Build a [`PhpFpmSpec`] for every PHP version any project uses, and
/// materialise the pool config files those specs point at.
///
/// Managed PortBay PHP builds win before neutral host installs. We still probe
/// `crate::php::detect_all()` once per tick for the fallback path; detection
/// runs `php --ini` / `php -m` per candidate which costs ~10–30 ms total on a
/// typical Homebrew install — fine for the reconcile cadence.
///
/// Versions whose probe doesn't yield a `php-fpm` binary are silently
/// skipped (the user already sees a warning on `/php`). Pool-config
/// write failures are logged but don't abort the tick — PC will
/// surface the FPM-start failure with a useful error.
fn php_fpm_specs_for(reg: &Registry, data_dir: &Path) -> Vec<process_compose::config::PhpFpmSpec> {
    use std::collections::HashSet;

    // Resolve each project's PHP version through `php_version_effective`
    // (runtime pin first, legacy `php_version` fallback) — the same source the
    // Caddy FastCGI route dials, so every spawned pool has a matching upstream.
    let used_versions: HashSet<String> = reg
        .list_projects()
        .iter()
        .filter_map(|p| p.php_version_effective().map(str::to_owned))
        .collect();
    if used_versions.is_empty() {
        return Vec::new();
    }

    let host_installs = crate::php::detect_all();
    let mut specs = Vec::with_capacity(used_versions.len());
    for ver in &used_versions {
        let Some(install) = managed_php_install(reg, ver).or_else(|| {
            host_installs
                .iter()
                .find(|i| crate::runtimes::version_matches(&i.version, ver))
                .cloned()
        }) else {
            tracing::warn!(
                target: "reconciler",
                "PHP {ver} requested by a project but not installed — \
                 PC entry skipped. Install a PortBay PHP runtime from the \
                 Languages panel, or install a neutral host PHP."
            );
            continue;
        };
        let Some(fpm_bin) = install.php_fpm_bin.clone() else {
            tracing::warn!(
                target: "reconciler",
                "PHP {ver} is installed but php-fpm is missing — \
                 PC entry skipped. Reinstall with `brew reinstall php@{ver}`."
            );
            continue;
        };

        let pool_path = crate::php::lifecycle::fpm_pool_path(data_dir, ver);
        let socket_path = crate::php::lifecycle::fpm_socket_path(data_dir, ver);
        let working_dir = pool_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| data_dir.to_path_buf());

        // Write the pool config every tick so manual edits inside the
        // file get overwritten (the user is supposed to drop tweaks
        // into the extension-dir as separate .ini files; the pool
        // config itself is PortBay-managed).
        if let Some(parent) = pool_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    target: "reconciler",
                    "couldn't create FPM dir {}: {e}",
                    parent.display()
                );
                continue;
            }
        }
        // Per-version PortBay tuning + php.ini overrides from the registry
        // (set via the /languages FPM and PHP tabs). Absent → defaults, which
        // render the same pool config as before any tuning was saved.
        let php_cfg = reg.runtimes.php.get(ver).cloned().unwrap_or_default();
        let slowlog_path = if php_cfg.fpm.slowlog.trim().is_empty() {
            crate::php::lifecycle::fpm_slowlog_path(data_dir, ver)
        } else {
            std::path::PathBuf::from(php_cfg.fpm.slowlog.trim())
        };
        if !php_cfg.fpm.request_slowlog_timeout.trim().is_empty()
            && php_cfg.fpm.request_slowlog_timeout.trim() != "0"
            && php_cfg.fpm.request_slowlog_timeout.trim() != "0s"
        {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&slowlog_path);
        }
        if php_cfg.fpm.access_log {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(crate::php::lifecycle::fpm_access_log_path(data_dir, ver));
        }
        let pool_body = crate::php::lifecycle::render_pool_config(
            &install,
            &socket_path,
            &php_cfg.fpm,
            &php_cfg.ini,
        );
        if let Err(e) = std::fs::write(&pool_path, &pool_body) {
            tracing::warn!(
                target: "reconciler",
                "couldn't write {}: {e}",
                pool_path.display()
            );
            continue;
        }

        specs.push(process_compose::config::PhpFpmSpec {
            process_id: crate::php::lifecycle::fpm_process_id(ver),
            version: ver.clone(),
            php_fpm_bin: fpm_bin,
            pool_config: pool_path,
            working_dir,
        });
    }
    specs.sort_by(|a, b| a.process_id.cmp(&b.process_id));
    specs
}

fn managed_php_install(reg: &Registry, requested: &str) -> Option<crate::php::PhpInstall> {
    let managed = reg.runtimes.managed.iter().find(|m| {
        m.lang == "php"
            && m.arch == crate::runtimes::download::manifest::current_arch()
            && crate::runtimes::version_matches(&m.version, requested)
    })?;
    let install_dir = managed.binary.parent()?.parent()?;
    let fpm_bin = install_dir.join("sbin/php-fpm");
    if !fpm_bin.is_file() {
        tracing::warn!(
            target: "reconciler",
            "PortBay PHP {} is registered at {}, but sbin/php-fpm is missing",
            managed.version,
            install_dir.display()
        );
        return None;
    }
    let mut install = crate::php::probe(
        &managed.binary,
        &managed.version,
        crate::php::PhpSource::PortBay,
    )?;
    install.php_fpm_bin = Some(fpm_bin);
    Some(install)
}

/// Build a [`DatabaseDaemonSpec`] for every database instance whose daemon
/// binary resolves and whose data dir is initialized. `app_data` is the
/// PortBay data dir (the parent of `logs_dir`), which is where instance
/// data directories live.
///
/// Instances whose engine binary is missing, or whose data dir hasn't been
/// provisioned, are skipped with a warning — PC would only fail to launch
/// them, and a missing daemon shouldn't poison the whole YAML. `warned`
/// (from [`PcCache`]) dedupes those warnings across ticks: each skip state
/// logs once, then stays quiet until the instance heals or regresses.
fn db_daemon_specs_for(
    reg: &Registry,
    app_data: &Path,
    warned: &mut std::collections::HashSet<String>,
) -> Vec<process_compose::config::DatabaseDaemonSpec> {
    let mut specs = Vec::new();
    let mut still_skipped = std::collections::HashSet::new();
    for inst in reg.list_databases() {
        // File-based engines (SQLite) have no daemon to supervise — they're
        // never a Process Compose process. Their connection env is still
        // injected into linked projects (see `db_connection_env_for`).
        if inst.engine.is_file_based() {
            continue;
        }
        // Prefer a PortBay-managed engine install, falling back to Homebrew/system.
        let managed_bin = reg
            .managed_engine(inst.engine)
            .map(|m| crate::databases::managed_bin_dir(&m.dir));
        let Some(daemon) =
            crate::databases::daemon_binary_resolved(inst.engine, managed_bin.as_deref())
        else {
            let key = format!("{}:binary", inst.id);
            if !warned.contains(&key) {
                tracing::warn!(
                    target: "reconciler",
                    "database `{}` ({}) skipped — daemon binary not found. Install the engine via the Databases panel.",
                    inst.id, inst.engine.label(),
                );
            }
            still_skipped.insert(key);
            continue;
        };
        let data = crate::databases::data_dir(app_data, inst.id.as_str());
        if !crate::databases::is_initialized(inst.engine, &data) {
            let key = format!("{}:init", inst.id);
            if !warned.contains(&key) {
                tracing::warn!(
                    target: "reconciler",
                    "database `{}` skipped — data dir {} not initialized. Start it from the Databases panel to re-provision.",
                    inst.id, data.display(),
                );
            }
            still_skipped.insert(key);
            continue;
        }
        let command = crate::databases::run_command(inst, &daemon, app_data);
        let working_dir = crate::databases::instance_dir(app_data, inst.id.as_str());
        specs.push(process_compose::config::DatabaseDaemonSpec {
            process_id: inst.process_id(),
            description: format!("{} {} — {}", inst.engine.label(), inst.version, inst.name),
            command,
            working_dir,
            port: inst.port,
            auto_start: inst.auto_start,
        });
    }
    *warned = still_skipped;
    specs.sort_by(|a, b| a.process_id.cmp(&b.process_id));
    specs
}

/// Read the Mailpit sidecar's status off `AppState` and translate it
/// into the optional env-injection passed through to the YAML
/// generator. Returns `None` when Mailpit isn't running (Mail* vars
/// are absent from the generated YAML).
fn mail_env_from_state(state: &AppState) -> Option<process_compose::config::MailpitEnv> {
    let guard = state.mailpit.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_running() {
        Some(process_compose::config::MailpitEnv::with_smtp_port(
            guard.smtp_port(),
        ))
    } else {
        None
    }
}

/// Default location for the persistent reconciled YAML. The bootstrap
/// placeholder this replaces (`process-compose.bootstrap.yaml`) is
/// deleted from `state.rs` in the same commit.
pub fn default_yaml_path() -> std::io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("process-compose.yaml"))
}

fn hash_string(s: &str) -> u64 {
    crate::util::stable_hash(s.as_bytes())
}

/// Crate-internal hash helper exposed so `Reconciler::prime_pc_cache_from_yaml`
/// can pre-populate the cache with whatever YAML setup just wrote.
pub(super) fn hash_yaml(s: &str) -> u64 {
    hash_string(s)
}

fn count_processes(reg: &Registry) -> usize {
    let project_processes = reg
        .list_projects()
        .iter()
        .filter(|p| p.start_command.is_some())
        .count();
    let generated_web_servers = reg
        .list_projects()
        .iter()
        .filter(|p| {
            p.kind == crate::registry::ProjectType::Php
                && p.start_command.is_none()
                && p.port.is_some()
                && !matches!(p.web_server_effective(), crate::registry::WebServer::Caddy)
        })
        .count();
    project_processes + generated_web_servers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn next_project(id: &str, port: u16) -> Project {
        Project {
            cors: None,
            sandbox: None,
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(port),
            extra_ports: vec![],
            hostname: format!("{id}.test"),
            https: true,
            services: vec!["caddy".into()],
            env: BTreeMap::new(),
            readiness: None,
            auto_start: false,
            pre_start: vec![],
            post_start: vec![],
            tags: vec![],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
            domain: None,
            tunnel: None,
            deploy: None,
        }
    }

    #[test]
    fn yaml_hash_stable_for_equivalent_registries() {
        let mut a = Registry::new("test");
        a.add_project(next_project("x", 3010)).unwrap();
        a.add_project(next_project("y", 3011)).unwrap();
        let mut b = Registry::new("test");
        b.add_project(next_project("y", 3011)).unwrap();
        b.add_project(next_project("x", 3010)).unwrap();
        let y_a =
            process_compose::config::to_yaml(&a, Path::new("/tmp"), None, &[], &[], &[], true)
                .unwrap();
        let y_b =
            process_compose::config::to_yaml(&b, Path::new("/tmp"), None, &[], &[], &[], true)
                .unwrap();
        // YAML emit may be ordering-stable already; we don't depend on
        // that. We do depend on the hash being deterministic given a
        // string input.
        assert_eq!(hash_string(&y_a), hash_string(&y_a.clone()));
        // And on the two YAMLs being equivalent post-ordering inside PC
        // config gen (which uses BTreeMap, so emit IS stable).
        assert_eq!(hash_string(&y_a), hash_string(&y_b));
    }

    #[test]
    fn yaml_hash_changes_when_project_added() {
        let mut r = Registry::new("test");
        r.add_project(next_project("a", 3010)).unwrap();
        let h1 = hash_string(
            &process_compose::config::to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[], true)
                .unwrap(),
        );
        r.add_project(next_project("b", 3011)).unwrap();
        let h2 = hash_string(
            &process_compose::config::to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[], true)
                .unwrap(),
        );
        assert_ne!(h1, h2);
    }

    #[test]
    fn process_count_counts_only_command_bearing_projects() {
        let mut r = Registry::new("test");
        r.add_project(next_project("with-cmd", 3010)).unwrap();
        let mut no_cmd = next_project("php-only", 3011);
        no_cmd.start_command = None;
        r.add_project(no_cmd).unwrap();
        assert_eq!(count_processes(&r), 1);
    }
}
