//! Hosts sub-reconciler.
//!
//! Reads the project list from the registry, derives a `(hostname, ip)`
//! pair for every project, hashes the sorted pair list, and only writes
//! `/etc/hosts` when the hash differs from the last successful apply.
//! Permission-denied is cached against the input hash so that, on a
//! system without the privileged helper installed, the 30 s safety tick
//! does not spam sudo prompts forever.

use std::net::Ipv4Addr;

use crate::hosts::{HostsError, HostsManager};
use crate::hosts_helper::HostsHelperClient;
use crate::reconciler::report::StepOutcome;
use crate::registry::{Registry, ResolverMode};

/// State kept across ticks. Lives inside `ReconcilerInner` (see `mod.rs`).
#[derive(Debug, Default)]
pub(super) struct HostsCache {
    /// Hash of the last *successful* apply. `None` until the first
    /// successful write.
    last_applied: Option<u64>,

    /// Hash of the last *attempted* apply that failed with
    /// `PermissionDenied`. Used to short-circuit re-attempts at the same
    /// hash so the periodic tick doesn't pile up permission prompts.
    last_perm_denied: Option<u64>,
}

pub(super) fn reconcile(
    reg: &Registry,
    resolver_installed: bool,
    cache: &mut HostsCache,
) -> StepOutcome {
    // Prefer PortBay's own privileged helper (a root LaunchDaemon) so the
    // GUI can write /etc/hosts without a per-write sudo prompt and without
    // depending on any third-party tool. Fall back to a direct write when
    // the helper isn't installed (e.g. the user is running as root, or
    // hasn't installed it yet) so behaviour degrades rather than breaks.
    let helper = HostsHelperClient::system();
    if helper.is_available() {
        if let Some(outcome) = reconcile_via_helper(reg, resolver_installed, cache, &helper) {
            return outcome;
        }
        // Helper present but the request failed — fall through to a direct
        // attempt so a transient helper error doesn't strand the hosts file.
    }
    reconcile_with(reg, resolver_installed, cache, &HostsManager::system())
}

/// Apply the expected pairs through the privileged helper. Returns `None` if
/// the helper call failed (so the caller can fall back to a direct write);
/// `Some(outcome)` when the helper handled it (applied or skipped-unchanged).
fn reconcile_via_helper(
    reg: &Registry,
    resolver_installed: bool,
    cache: &mut HostsCache,
    helper: &HostsHelperClient,
) -> Option<StepOutcome> {
    let pairs = expected_pairs(reg, resolver_installed);
    let hash = hash_pairs(&pairs);

    if cache.last_applied == Some(hash) {
        return Some(StepOutcome::skipped("unchanged"));
    }

    match helper.replace_all(pairs.iter().cloned(), &reg.domain_suffix) {
        Ok(()) => {
            cache.last_applied = Some(hash);
            cache.last_perm_denied = None;
            Some(StepOutcome::applied(format!(
                "{} hostname(s) via privileged helper",
                pairs.len()
            )))
        }
        Err(_) => None,
    }
}

/// Same as [`reconcile`] but with an injectable manager — used by tests
/// to point at a tempfile.
pub(super) fn reconcile_with(
    reg: &Registry,
    resolver_installed: bool,
    cache: &mut HostsCache,
    manager: &HostsManager,
) -> StepOutcome {
    let pairs = expected_pairs(reg, resolver_installed);
    let hash = hash_pairs(&pairs);

    if cache.last_applied == Some(hash) {
        return StepOutcome::skipped("unchanged");
    }

    if cache.last_perm_denied == Some(hash) {
        return StepOutcome::skipped(
            "hosts unwritable; last attempt at this hash failed with permission denied",
        );
    }

    match manager.replace_all(pairs.clone()) {
        Ok(()) => {
            cache.last_applied = Some(hash);
            cache.last_perm_denied = None;
            StepOutcome::applied(format!("{} hostname(s)", pairs.len()))
        }
        Err(HostsError::PermissionDenied { path }) => {
            cache.last_perm_denied = Some(hash);
            StepOutcome::failed(format!("permission denied writing {}", path.display()))
        }
        Err(e) => StepOutcome::failed(e.to_string()),
    }
}

/// Build the sorted, deduplicated list of `(hostname, ip)` pairs the
/// registry implies. Sorted so the hash is stable across registry
/// reorderings.
///
/// Per-project [`ResolverMode`] decides whether a hostname earns an
/// `/etc/hosts` row:
/// - `Hosts` — always written, even when the dnsmasq wildcard would cover it.
/// - `Dnsmasq` — never written; the host relies on the wildcard resolver.
/// - `Auto` — written only when the wildcard resolver isn't installed (the
///   historical fallback), so an installed resolver keeps `/etc/hosts` clean.
fn expected_pairs(reg: &Registry, resolver_installed: bool) -> Vec<(String, Ipv4Addr)> {
    let mut pairs: Vec<(String, Ipv4Addr)> = reg
        .list_projects()
        .iter()
        .filter(|p| match p.resolver_mode() {
            ResolverMode::Hosts => true,
            ResolverMode::Dnsmasq => false,
            ResolverMode::Auto => !resolver_installed,
        })
        .map(|p| (p.hostname.clone(), Ipv4Addr::LOCALHOST))
        .collect();
    pairs.sort();
    pairs.dedup();
    pairs
}

fn hash_pairs(pairs: &[(String, Ipv4Addr)]) -> u64 {
    // Canonical, order-preserving byte encoding so the cache key is stable
    // across Rust toolchains (DefaultHasher's algorithm is not guaranteed to be).
    let mut buf = String::new();
    for (host, ip) in pairs {
        buf.push_str(host);
        buf.push('=');
        buf.push_str(&ip.to_string());
        buf.push('\n');
    }
    crate::util::stable_hash(buf.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType, Registry};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn project(id: &str, host: &str) -> Project {
        Project {
            cors: None,
            sandbox: None,
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
            extra_ports: vec![],
            hostname: host.into(),
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
            domain: None,
        }
    }

    fn tmp_manager() -> (tempfile::TempDir, HostsManager) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hosts");
        // Seed the file with a non-PortBay line so the manager's block
        // logic has something realistic to leave alone.
        std::fs::write(&path, "127.0.0.1 localhost\n").unwrap();
        let m = HostsManager::new(path);
        (tmp, m)
    }

    #[test]
    fn emits_pairs_for_all_projects() {
        let mut reg = Registry::new("test");
        reg.add_project(project("a", "a.test")).unwrap();
        reg.add_project(project("b", "b.test")).unwrap();
        reg.add_project(project("c", "c.test")).unwrap();
        let pairs = expected_pairs(&reg, false);
        assert_eq!(pairs.len(), 3);
        assert!(pairs.contains(&("a.test".into(), Ipv4Addr::LOCALHOST)));
    }

    #[test]
    fn resolver_mode_controls_hosts_rows() {
        use crate::registry::DomainConfig;
        let mut reg = Registry::new("test");
        let mut force = project("force", "force.test");
        force.domain = Some(DomainConfig {
            resolver_mode: ResolverMode::Hosts,
            ..DomainConfig::default()
        });
        let mut skip = project("skip", "skip.test");
        skip.domain = Some(DomainConfig {
            resolver_mode: ResolverMode::Dnsmasq,
            ..DomainConfig::default()
        });
        reg.add_project(force).unwrap();
        reg.add_project(skip).unwrap();
        reg.add_project(project("auto", "auto.test")).unwrap();

        // Wildcard resolver installed: Auto relies on it (no row), Dnsmasq opts
        // out, only the forced Hosts host gets an /etc/hosts row.
        let with_resolver = expected_pairs(&reg, true);
        assert!(with_resolver.contains(&("force.test".into(), Ipv4Addr::LOCALHOST)));
        assert!(!with_resolver
            .iter()
            .any(|(h, _)| h == "auto.test" || h == "skip.test"));

        // No resolver: Auto + Hosts both get rows; Dnsmasq still opts out.
        let without = expected_pairs(&reg, false);
        assert!(without.contains(&("force.test".into(), Ipv4Addr::LOCALHOST)));
        assert!(without.contains(&("auto.test".into(), Ipv4Addr::LOCALHOST)));
        assert!(!without.iter().any(|(h, _)| h == "skip.test"));
    }

    #[test]
    fn hash_stable_for_equal_registries() {
        let mut a = Registry::new("test");
        a.add_project(project("x", "x.test")).unwrap();
        a.add_project(project("y", "y.test")).unwrap();
        let mut b = Registry::new("test");
        // Inserted in a different order.
        b.add_project(project("y", "y.test")).unwrap();
        b.add_project(project("x", "x.test")).unwrap();
        assert_eq!(
            hash_pairs(&expected_pairs(&a, false)),
            hash_pairs(&expected_pairs(&b, false))
        );
    }

    #[test]
    fn first_apply_writes_then_second_skips() {
        let (_tmp, m) = tmp_manager();
        let mut reg = Registry::new("test");
        reg.add_project(project("a", "a.test")).unwrap();
        let mut cache = HostsCache::default();
        let first = reconcile_with(&reg, false, &mut cache, &m);
        assert!(matches!(first, StepOutcome::Applied { .. }));
        let second = reconcile_with(&reg, false, &mut cache, &m);
        assert!(
            matches!(second, StepOutcome::Skipped { .. }),
            "expected Skipped, got {second:?}"
        );
    }

    #[test]
    fn change_in_registry_triggers_reapply() {
        let (_tmp, m) = tmp_manager();
        let mut reg = Registry::new("test");
        reg.add_project(project("a", "a.test")).unwrap();
        let mut cache = HostsCache::default();
        let _ = reconcile_with(&reg, false, &mut cache, &m);
        reg.add_project(project("b", "b.test")).unwrap();
        let second = reconcile_with(&reg, false, &mut cache, &m);
        assert!(matches!(second, StepOutcome::Applied { .. }));
        let entries = m.list_managed().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    #[cfg(unix)]
    fn permission_denied_caches_and_short_circuits() {
        // The hosts manager writes atomically via a sibling tempfile and
        // `rename`. Rename's permission check is on the *parent directory*,
        // not the target file — so we lock the parent to read+exec only.
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("locked");
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("hosts");
        std::fs::write(&path, "127.0.0.1 localhost\n").unwrap();

        let mut perms = std::fs::metadata(&parent).unwrap().permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(&parent, perms).unwrap();

        let m = HostsManager::new(&path);
        let mut reg = Registry::new("test");
        reg.add_project(project("a", "a.test")).unwrap();

        let mut cache = HostsCache::default();
        let first = reconcile_with(&reg, false, &mut cache, &m);

        // Restore parent perms before any assertions panic so the
        // tempdir cleanup at end-of-scope can succeed.
        let mut perms = std::fs::metadata(&parent).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&parent, perms).unwrap();

        assert!(matches!(first, StepOutcome::Failed { .. }), "got {first:?}");
        assert!(cache.last_perm_denied.is_some());

        // Subsequent calls with the same hash short-circuit without a
        // second open attempt — the whole point of the cache.
        let second = reconcile_with(&reg, false, &mut cache, &m);
        assert!(matches!(second, StepOutcome::Skipped { .. }));
    }
}
