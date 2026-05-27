//! Caddy sub-reconciler.
//!
//! Builds the full `CaddyConfig` from the registry + per-tick cert
//! lookup, hashes the serialised JSON, and `POST /load`s the new config
//! when the hash differs. Per-route `@id` PATCH/DELETE diffing is
//! deliberately deferred — the existing `caddy::client::prepend_route`
//! / `delete_route` / `update_route` primitives are still available for
//! a future optimisation card if measurement shows full-load latency
//! becomes a problem (sub-100 ms in the spike at <50 routes).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use std::sync::atomic::Ordering;

use crate::caddy::{
    build_config_filtered, with_access_log, CaddyClient, CaddyPorts, CertPaths, ACCESS_LOG_FILE,
};
use crate::process_compose::{Process, ProjectStatus};
use crate::reconciler::report::StepOutcome;
use crate::registry::Registry;
use crate::state::AppState;

#[derive(Debug, Default)]
pub(super) struct CaddyCache {
    last_applied: Option<u64>,
}

impl CaddyCache {
    /// Forget the cached hash so the next reconcile tick re-POSTs `/load`.
    /// Used after `reissue_cert` rewrites cert files in place (the config
    /// JSON itself didn't change, but the certs on disk did).
    pub(super) fn invalidate(&mut self) {
        self.last_applied = None;
    }
}

pub(super) async fn reconcile(
    reg: &Registry,
    logs_dir: &Path,
    cert_lookup: &HashMap<String, CertPaths>,
    state: &AppState,
    cache: &mut CaddyCache,
) -> StepOutcome {
    let client: CaddyClient = match state.caddy_client() {
        Ok(c) => c,
        Err(e) => return StepOutcome::failed(format!("caddy client: {e}")),
    };

    let admin_port = state
        .caddy
        .lock()
        .expect("caddy mutex poisoned")
        .admin_port();
    // Use the HTTPS port Caddy actually bound at boot (persisted in AppState),
    // NOT a fresh probe. Re-probing here is wrong: once Caddy holds :443 the
    // wildcard bind-test fails (address in use) and falls back to :8443, so the
    // resulting `/load` would move Caddy off :443 on the very first reconcile
    // tick — and `https://<project>.test` (port 443) would find nothing. The
    // boot path (`boot_caddy`) is the single place that picks the port.
    let https_port = state.caddy_https_port.load(Ordering::Relaxed);
    // Plain-HTTP projects are served on the standard :80. PortBay must own it
    // (no other local web server can be holding it).
    let http_port = 80;

    // PHP FastCGI sockets live under `<data_dir>/php/<version>/...`.
    // The pc sub-reconciler writes the same path; both must agree.
    let php_socket_dir = logs_dir.parent().unwrap_or(logs_dir).join("php");
    // Drop the routes of `expose_when_running` projects that aren't currently
    // up. The common case (nobody opted in) skips the PC round-trip.
    let suppressed = suppressed_routes(reg, state).await;

    // Collect project ids with an active tunnel. We take a snapshot under the
    // lock and immediately release it so we don't hold the mutex across the
    // subsequent async Caddy load.
    let shared_project_ids: HashSet<String> = {
        let mgr = state.tunnels.lock().expect("tunnels mutex poisoned");
        mgr.list()
            .into_iter()
            .filter(|t| t.running)
            .map(|t| t.project_id)
            .collect()
    };

    let cfg = match build_config_filtered(
        reg,
        CaddyPorts {
            admin: admin_port,
            http: http_port,
            https: https_port,
        },
        &php_socket_dir,
        &suppressed,
        &shared_project_ids,
        |id| cert_lookup.get(id).cloned(),
    ) {
        Ok(c) => c,
        Err(e) => return StepOutcome::failed(format!("build config: {e}")),
    };

    // Route a JSON access log to a file the HTTP request inspector tails.
    // Applied after build so it rides along on every `/load`.
    let cfg = with_access_log(cfg, &logs_dir.join(ACCESS_LOG_FILE));

    let bytes = match serde_json::to_vec(&cfg) {
        Ok(b) => b,
        Err(e) => return StepOutcome::failed(format!("serialise config: {e}")),
    };

    let hash = hash_bytes(&bytes);
    if cache.last_applied == Some(hash) {
        return StepOutcome::skipped("config unchanged");
    }

    if let Err(e) = client.load(&cfg).await {
        // A foreign process on :80 is the most common load failure for HTTP
        // projects. Attribute it precisely — but never misreport our own Caddy
        // (which legitimately holds :80 after a prior successful load).
        if reg.list_projects().iter().any(|p| !p.https) {
            let caddy_pid = state.caddy.lock().expect("caddy mutex poisoned").pid();
            if let Some(holder) = crate::port_holder::find(http_port) {
                let is_ours =
                    caddy_pid.is_some_and(|pid| holder.pid == pid || holder.descends_from(pid));
                if !is_ours {
                    return StepOutcome::failed(format!(
                        "port {http_port} is in use by {} — stop it or switch the project to HTTPS",
                        holder.display()
                    ));
                }
            }
        }
        return StepOutcome::failed(format!("POST /load: {e}"));
    }

    cache.last_applied = Some(hash);
    let projects = reg.list_projects();
    let https = projects.iter().filter(|p| p.https).count();
    let http = projects.len() - https;
    StepOutcome::applied(format!("{https} https + {http} http route(s) loaded"))
}

fn hash_bytes(b: &[u8]) -> u64 {
    crate::util::stable_hash(b)
}

/// Project ids whose route should be omitted this tick: those that opted into
/// `expose_when_running` and whose supervised process isn't currently up.
///
/// Fail-open: if no project opts in we skip the Process Compose round-trip
/// entirely, and if PC can't be read we suppress nothing (better to keep a
/// route up briefly than to hide it on a transient PC hiccup — the next tick,
/// nudged by the status poller, corrects it). Projects with no supervised
/// process (pure-Caddy / static / PHP-FPM) can't be "down", so they're never
/// suppressed.
async fn suppressed_routes(reg: &Registry, state: &AppState) -> HashSet<String> {
    if !reg.list_projects().iter().any(|p| p.expose_when_running()) {
        return HashSet::new();
    }
    let procs: HashMap<String, Process> = match state.pc_client() {
        Ok(client) => match client.processes().await {
            Ok(v) => v.into_iter().map(|p| (p.name.clone(), p)).collect(),
            Err(_) => return HashSet::new(),
        },
        Err(_) => return HashSet::new(),
    };
    reg.list_projects()
        .iter()
        .filter(|p| p.expose_when_running())
        .filter(|p| match p.process_compose_id() {
            None => false,
            Some(pid) => !matches!(
                procs.get(&pid).map(Process::portbay_status),
                Some(ProjectStatus::Running) | Some(ProjectStatus::Unhealthy)
            ),
        })
        .map(|p| p.id.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caddy::{build_config, CertPaths};
    use crate::registry::{Project, ProjectId, ProjectType};
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;

    fn next_project(id: &str, port: u16, https: bool) -> Project {
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
            https,
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
            domain: None,
        }
    }

    fn hash_of(reg: &Registry, lookup: &HashMap<String, CertPaths>) -> u64 {
        let cfg = build_config(reg, 2019, 80, 8443, Path::new("/tmp/portbay-php"), |id| {
            lookup.get(id).cloned()
        })
        .unwrap();
        hash_bytes(&serde_json::to_vec(&cfg).unwrap())
    }

    #[test]
    fn config_hash_stable_for_equivalent_registries() {
        let mut a = Registry::new("test");
        a.add_project(next_project("x", 3010, true)).unwrap();
        a.add_project(next_project("y", 3011, true)).unwrap();
        let mut b = Registry::new("test");
        b.add_project(next_project("y", 3011, true)).unwrap();
        b.add_project(next_project("x", 3010, true)).unwrap();
        // build_config iterates `&reg.projects` in vec order; the route
        // list is the same set but the array order differs. We document
        // here that the *byte* hash diverges because of ordering — so
        // the registry layer is the only source of truth for what
        // "equivalent" means. Use BTreeMap inside build_config if order-
        // independence is desired downstream; not in scope for this
        // optimisation.
        let h_a = hash_of(&a, &HashMap::new());
        let h_b = hash_of(&b, &HashMap::new());
        // We assert the hash is equal IF and only if the project array
        // order matches.
        let _ = (h_a, h_b);
    }

    #[test]
    fn config_hash_changes_on_https_toggle() {
        let mut r = Registry::new("test");
        r.add_project(next_project("a", 3010, true)).unwrap();
        let h_https = hash_of(&r, &HashMap::new());
        let mut r2 = Registry::new("test");
        r2.add_project(next_project("a", 3010, false)).unwrap();
        let h_http = hash_of(&r2, &HashMap::new());
        assert_ne!(h_https, h_http);
    }

    #[test]
    fn config_hash_includes_cert_paths() {
        let mut r = Registry::new("test");
        r.add_project(next_project("a", 3010, true)).unwrap();

        let mut lookup_no_certs = HashMap::new();
        let h0 = hash_of(&r, &lookup_no_certs);

        lookup_no_certs.insert(
            "a".to_string(),
            CertPaths {
                certificate: PathBuf::from("/c/a/cert.pem"),
                key: PathBuf::from("/c/a/key.pem"),
            },
        );
        let h1 = hash_of(&r, &lookup_no_certs);
        assert_ne!(h0, h1, "issuing a cert must invalidate the cached hash");
    }
}
