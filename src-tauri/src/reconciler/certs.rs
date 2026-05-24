//! Certs sub-reconciler.
//!
//! Walks the HTTPS-enabled projects, issues a cert for any whose
//! `cert.pem` / `key.pem` pair is missing on disk, and reaps cert
//! directories for project ids that no longer appear in the registry.
//! Returns a synchronous `HashMap<project_id, CertPaths>` lookup that
//! the Caddy sub-reconciler closes over when calling
//! `caddy::config::build_config`.
//!
//! Hard rule: never call `mkcert -install` from here. CA installation is
//! a user-driven flow with explicit consent — see the cert-lifecycle
//! card on the backlog. If the CA is not present, we record a non-fatal
//! `Failed` for the step; Caddy will still POST /load without certs and
//! the user sees a typed sidecar warning surface elsewhere.

use std::collections::{HashMap, HashSet};

use crate::caddy::CertPaths;
use crate::mkcert::Mkcert;
use crate::reconciler::report::StepOutcome;
use crate::registry::Registry;

#[derive(Debug, Default)]
pub(super) struct CertsCache {
    /// Hash of the sorted set of project ids that have certs on disk.
    /// Drives the "skip when unchanged" short-circuit.
    last_set_hash: Option<u64>,
}

#[derive(Debug)]
pub(super) struct CertsTickResult {
    pub outcome: StepOutcome,
    pub lookup: HashMap<String, CertPaths>,
}

pub(super) fn reconcile(
    reg: &Registry,
    mkcert: Option<&Mkcert>,
    cache: &mut CertsCache,
) -> CertsTickResult {
    let Some(mkcert) = mkcert else {
        // No binary resolved at boot — Caddy will continue, projects with
        // HTTPS just won't have certs. Surface as Failed so doctor can
        // see it; the user's existing mkcert sidebar slot already shows
        // the underlying installation state.
        return CertsTickResult {
            outcome: StepOutcome::failed("mkcert binary not available; skipping cert issuance"),
            lookup: HashMap::new(),
        };
    };

    if !mkcert.is_ca_installed() {
        return CertsTickResult {
            outcome: StepOutcome::failed(
                "mkcert CA not installed; run the cert-lifecycle install flow",
            ),
            lookup: lookup_only(reg, mkcert),
        };
    }

    let https_projects: Vec<_> = reg.list_projects().iter().filter(|p| p.https).collect();

    // 1) Issue missing certs.
    let mut issued: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for p in &https_projects {
        if mkcert.cert_paths(p.id.as_str()).is_some() {
            continue;
        }
        let hosts = [p.hostname.as_str()];
        match mkcert.issue_cert(p.id.as_str(), &hosts) {
            Ok(_) => issued.push(p.id.to_string()),
            Err(e) => errors.push(format!("{}: {}", p.id.as_str(), e)),
        }
    }

    // 2) Reap cert dirs for project ids no longer in the registry.
    let active: HashSet<String> = reg
        .list_projects()
        .iter()
        .map(|p| p.id.to_string())
        .collect();
    let mut reaped: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(mkcert.certs_root()) {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            if !active.contains(&name) {
                if let Err(e) = mkcert.remove_cert(&name) {
                    errors.push(format!("reap {name}: {e}"));
                } else {
                    reaped.push(name);
                }
            }
        }
    }

    // 3) Build the lookup snapshot for the Caddy step.
    let lookup = lookup_only(reg, mkcert);

    // 4) Decide the outcome.
    let set_hash = lookup_set_hash(&lookup);
    let outcome = if !errors.is_empty() {
        // Don't advance the cache so the next tick retries the failures.
        StepOutcome::failed(errors.join("; "))
    } else if issued.is_empty() && reaped.is_empty() && cache.last_set_hash == Some(set_hash) {
        StepOutcome::skipped("unchanged")
    } else {
        cache.last_set_hash = Some(set_hash);
        let mut summary = Vec::new();
        if !issued.is_empty() {
            summary.push(format!("issued {}", issued.len()));
        }
        if !reaped.is_empty() {
            summary.push(format!("reaped {}", reaped.len()));
        }
        if summary.is_empty() {
            summary.push(format!("{} cert(s) ready", lookup.len()));
        }
        StepOutcome::applied(summary.join(", "))
    };

    CertsTickResult { outcome, lookup }
}

fn lookup_only(reg: &Registry, mkcert: &Mkcert) -> HashMap<String, CertPaths> {
    let mut out = HashMap::new();
    for p in reg.list_projects() {
        if !p.https {
            continue;
        }
        if let Some(paths) = mkcert.cert_paths(p.id.as_str()) {
            out.insert(p.id.to_string(), paths);
        }
    }
    out
}

fn lookup_set_hash(lookup: &HashMap<String, CertPaths>) -> u64 {
    let mut ids: Vec<&String> = lookup.keys().collect();
    ids.sort();
    // Newline-joined sorted ids → stable across Rust toolchains.
    let joined = ids
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    crate::util::stable_hash(joined.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn https_project(id: &str) -> Project {
        Project {
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
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
            runtime: None,
            workspace: None,
        }
    }

    fn http_project(id: &str) -> Project {
        let mut p = https_project(id);
        p.https = false;
        p.services.clear();
        p
    }

    #[test]
    fn no_mkcert_binary_returns_failed_and_empty_lookup() {
        let reg = Registry::new("test");
        let mut cache = CertsCache::default();
        let r = reconcile(&reg, None, &mut cache);
        assert!(matches!(r.outcome, StepOutcome::Failed { .. }));
        assert!(r.lookup.is_empty());
    }

    #[test]
    fn lookup_only_skips_non_https_and_missing_certs() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Mkcert::new("/does/not/exist", tmp.path());
        let mut reg = Registry::new("test");
        reg.add_project(https_project("a")).unwrap();
        reg.add_project(http_project("b")).unwrap();

        // Materialise cert.pem + key.pem only for "a".
        let dir = tmp.path().join("a");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("cert.pem"), b"x").unwrap();
        std::fs::write(dir.join("key.pem"), b"x").unwrap();

        let lookup = lookup_only(&reg, &m);
        assert_eq!(lookup.len(), 1);
        assert!(lookup.contains_key("a"));
        assert!(!lookup.contains_key("b"));
    }

    #[test]
    fn set_hash_changes_when_project_added() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Mkcert::new("/does/not/exist", tmp.path());

        let mut reg = Registry::new("test");
        reg.add_project(https_project("a")).unwrap();
        let dir = tmp.path().join("a");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("cert.pem"), b"x").unwrap();
        std::fs::write(dir.join("key.pem"), b"x").unwrap();
        let h1 = lookup_set_hash(&lookup_only(&reg, &m));

        reg.add_project(https_project("b")).unwrap();
        let dir = tmp.path().join("b");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("cert.pem"), b"x").unwrap();
        std::fs::write(dir.join("key.pem"), b"x").unwrap();
        let h2 = lookup_set_hash(&lookup_only(&reg, &m));

        assert_ne!(h1, h2);
    }
}
