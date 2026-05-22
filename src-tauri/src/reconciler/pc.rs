//! Process Compose sub-reconciler.
//!
//! Generates the YAML the registry implies, hashes it, and restarts PC
//! against the new file only when the hash differs from the last
//! applied. PC's REST API does not support adding processes to a
//! running daemon — every YAML mutation costs one restart. This is a
//! known upstream constraint, called out in the kanban Outcome.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::process_compose;
use crate::registry::Registry;
use crate::reconciler::report::StepOutcome;
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
    let yaml = match process_compose::config::to_yaml(reg, logs_dir) {
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
pub fn build_initial_yaml(reg: &Registry, logs_dir: &Path) -> Result<String, String> {
    process_compose::config::to_yaml(reg, logs_dir).map_err(|e| e.to_string())
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
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Crate-internal hash helper exposed so `Reconciler::prime_pc_cache_from_yaml`
/// can pre-populate the cache with whatever YAML setup just wrote.
pub(super) fn hash_yaml(s: &str) -> u64 {
    hash_string(s)
}

fn count_processes(reg: &Registry) -> usize {
    reg.list_projects()
        .iter()
        .filter(|p| p.start_command.is_some())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn next_project(id: &str, port: u16) -> Project {
        Project {
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
        let y_a = process_compose::config::to_yaml(&a, Path::new("/tmp")).unwrap();
        let y_b = process_compose::config::to_yaml(&b, Path::new("/tmp")).unwrap();
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
        let h1 = hash_string(&process_compose::config::to_yaml(&r, Path::new("/tmp")).unwrap());
        r.add_project(next_project("b", 3011)).unwrap();
        let h2 = hash_string(&process_compose::config::to_yaml(&r, Path::new("/tmp")).unwrap());
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
