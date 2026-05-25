//! Process Compose sub-reconciler.
//!
//! Generates the YAML the registry implies, hashes it, and restarts PC
//! against the new file only when the hash differs from the last
//! applied. PC's REST API does not support adding processes to a
//! running daemon — every YAML mutation costs one restart. This is a
//! known upstream constraint, called out in the kanban Outcome.

use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::process_compose;
use crate::reconciler::report::StepOutcome;
use crate::registry::Registry;
use crate::state::AppState;

#[derive(Debug, Default)]
pub(super) struct PcCache {
    /// Hash of the last YAML successfully written + booted against.
    last_applied: Option<u64>,
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
    let db_specs = db_daemon_specs_for(reg, data_dir);
    let web_specs = crate::webservers::specs_for(reg, data_dir, logs_dir);
    let yaml = match process_compose::config::to_yaml(
        reg,
        logs_dir,
        mail_env.as_ref(),
        &php_fpm_specs,
        &db_specs,
        &web_specs,
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

    state.shutdown_pc();
    if let Err(e) = state.boot_pc(app, yaml_path) {
        return StepOutcome::failed(format!("boot pc: {e}"));
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
    process_compose::config::to_yaml(reg, logs_dir, None, &[], &[], &[]).map_err(|e| e.to_string())
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
/// We probe `crate::php::detect_all()` once per tick. Detection runs
/// `php --ini` / `php -m` per candidate which costs ~10–30 ms total
/// on a typical Homebrew install — fine for the reconcile cadence.
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

    let installs = crate::php::detect_all();
    let mut specs = Vec::with_capacity(used_versions.len());
    for ver in &used_versions {
        let Some(install) = installs.iter().find(|i| &i.version == ver) else {
            tracing::warn!(
                target: "reconciler",
                "PHP {ver} requested by a project but not installed — \
                 PC entry skipped. Run `brew install php@{ver}` then \
                 re-detect from the /php panel."
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
        let pool_body = crate::php::lifecycle::render_pool_config(
            install,
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

/// Build a [`DatabaseDaemonSpec`] for every database instance whose daemon
/// binary resolves and whose data dir is initialized. `app_data` is the
/// PortBay data dir (the parent of `logs_dir`), which is where instance
/// data directories live.
///
/// Instances whose engine binary is missing, or whose data dir hasn't been
/// provisioned, are skipped with a warning — PC would only fail to launch
/// them, and a missing daemon shouldn't poison the whole YAML.
fn db_daemon_specs_for(
    reg: &Registry,
    app_data: &Path,
) -> Vec<process_compose::config::DatabaseDaemonSpec> {
    let mut specs = Vec::new();
    for inst in reg.list_databases() {
        let Some(daemon) = crate::databases::daemon_binary(inst.engine) else {
            tracing::warn!(
                target: "reconciler",
                "database `{}` ({}) skipped — daemon binary not found. Install the engine via the Databases panel.",
                inst.id, inst.engine.label(),
            );
            continue;
        };
        let data = crate::databases::data_dir(app_data, inst.id.as_str());
        if !crate::databases::is_initialized(inst.engine, &data) {
            tracing::warn!(
                target: "reconciler",
                "database `{}` skipped — data dir {} not initialized yet.",
                inst.id, data.display(),
            );
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
    specs.sort_by(|a, b| a.process_id.cmp(&b.process_id));
    specs
}

/// Read the Mailpit sidecar's status off `AppState` and translate it
/// into the optional env-injection passed through to the YAML
/// generator. Returns `None` when Mailpit isn't running (Mail* vars
/// are absent from the generated YAML).
fn mail_env_from_state(state: &AppState) -> Option<process_compose::config::MailpitEnv> {
    let guard = state.mailpit.lock().expect("mailpit mutex poisoned");
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
            tags: vec![],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
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
            process_compose::config::to_yaml(&a, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        let y_b =
            process_compose::config::to_yaml(&b, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
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
            &process_compose::config::to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap(),
        );
        r.add_project(next_project("b", 3011)).unwrap();
        let h2 = hash_string(
            &process_compose::config::to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap(),
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
