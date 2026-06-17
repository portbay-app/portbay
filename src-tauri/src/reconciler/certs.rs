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
use crate::registry::{AcmeDnsProvider, Registry, SslMode};

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

/// Reissue a cert this many days before it expires, when auto-renew is on.
/// mkcert leaf certs are ~825 days; 30 days of runway is ample and avoids
/// reissuing on every tick.
const CERT_RENEW_THRESHOLD_DAYS: i64 = 30;

/// The suffixes a locally-trusted cert is allowed to cover: always `.test`, plus
/// the registry's configured domain suffix when the user has changed it away from
/// the default. Keeps the mkcert issuance guard ([`Mkcert::issue_cert`]) in
/// lock-step with the suffix the rest of the app actually serves on.
fn local_cert_suffixes(domain_suffix: &str) -> Vec<String> {
    let mut out = vec!["test".to_string()];
    let configured = domain_suffix
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if !configured.is_empty() && configured != "test" {
        out.push(configured);
    }
    out
}

pub(super) fn reconcile(
    reg: &Registry,
    mkcert: Option<&Mkcert>,
    auto_renew: bool,
    cache: &mut CertsCache,
) -> CertsTickResult {
    let mkcert_ready = mkcert.is_some_and(|m| m.is_ca_installed());

    // Only HTTPS projects PortBay is asked to manage get a minted cert.
    // `auto_manage_cert()` defaults true, so this matches the historical
    // "every HTTPS project" behaviour unless a project opts out.
    let https_projects: Vec<_> = reg
        .list_projects()
        .iter()
        .filter(|p| p.https && p.auto_manage_cert())
        .collect();

    // 1) Issue (or reissue) certs so each project's cert covers exactly the
    //    hostnames it needs. A project that opts into wildcard subdomains gets
    //    `*.hostname` added to the SAN list (mkcert accepts wildcard hosts).
    //    We skip only when a cert already covers every desired name, so
    //    toggling wildcard on — or renaming a hostname — is picked up here on
    //    the next tick without the UI having to force a reissue.
    let mut issued: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for p in &https_projects {
        if p.ssl_mode() != SslMode::AutomaticLocal {
            continue;
        }
        let Some(mkcert) = mkcert else {
            errors.push(format!(
                "{}: mkcert binary not available; skipping local cert issuance",
                p.id.as_str()
            ));
            continue;
        };
        if !mkcert_ready {
            errors.push(format!(
                "{}: mkcert CA not installed or not trusted",
                p.id.as_str()
            ));
            continue;
        }
        let desired_owned =
            desired_cert_names(p.hostname.as_str(), p.include_wildcard_subdomains());
        let desired: Vec<&str> = desired_owned.iter().map(String::as_str).collect();
        if let Some(paths) = mkcert.cert_paths(p.id.as_str()) {
            let have = crate::commands::certs::cert_all_sans(&paths.certificate);
            if crate::commands::certs::cert_covers_names(&have, &desired) {
                // SANs already cover this project. Reissue only when auto-renew
                // is enabled AND the cert is within the renewal window — so the
                // `auto_renew_certificates` setting is actually load-bearing
                // instead of a dead toggle.
                let near_expiry = auto_renew
                    && crate::commands::certs::cert_days_until_expiry(&paths.certificate)
                        .is_some_and(|days| days < CERT_RENEW_THRESHOLD_DAYS);
                if !near_expiry {
                    continue;
                }
            }
        }
        let allowed_suffixes = local_cert_suffixes(reg.domain_suffix.as_str());
        let allowed_refs: Vec<&str> = allowed_suffixes.iter().map(String::as_str).collect();
        match mkcert.issue_cert(p.id.as_str(), &desired, &allowed_refs) {
            Ok(_) => issued.push(p.id.to_string()),
            Err(e) => errors.push(format!("{}: {}", p.id.as_str(), e)),
        }
    }

    for p in reg.list_projects().iter().filter(|p| p.https) {
        match p.ssl_mode() {
            SslMode::AutomaticLocal => {}
            SslMode::CustomCertificate => {
                let Some((cert, key)) = p.custom_cert_paths() else {
                    errors.push(format!(
                        "{}: custom certificate mode requires certificate and key paths",
                        p.id.as_str()
                    ));
                    continue;
                };
                let desired_owned =
                    desired_cert_names(p.hostname.as_str(), p.include_wildcard_subdomains());
                let desired: Vec<&str> = desired_owned.iter().map(String::as_str).collect();
                if let Err(e) = crate::commands::certs::validate_custom_cert_pair(
                    &std::path::PathBuf::from(cert),
                    &std::path::PathBuf::from(key),
                    &desired,
                ) {
                    errors.push(format!(
                        "{}: custom certificate invalid: {e}",
                        p.id.as_str()
                    ));
                }
            }
            SslMode::SelfSigned => {
                errors.push(format!(
                    "{}: self-signed certificate mode is not implemented yet",
                    p.id.as_str()
                ));
            }
            SslMode::PublicAcme => {
                if let Err(e) = validate_public_acme_project(p) {
                    errors.push(format!("{}: {e}", p.id.as_str()));
                }
            }
        }
    }

    // 2) Reap cert dirs for project ids no longer in the registry.
    let active: HashSet<String> = reg
        .list_projects()
        .iter()
        .map(|p| p.id.to_string())
        .collect();
    let mut reaped: Vec<String> = Vec::new();
    if let Some(mkcert) = mkcert {
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

fn lookup_only(reg: &Registry, mkcert: Option<&Mkcert>) -> HashMap<String, CertPaths> {
    let mut out = HashMap::new();
    for p in reg.list_projects() {
        if !p.https {
            continue;
        }
        match p.ssl_mode() {
            SslMode::AutomaticLocal => {
                if let Some(paths) = mkcert.and_then(|m| m.cert_paths(p.id.as_str())) {
                    out.insert(p.id.to_string(), paths);
                }
            }
            SslMode::CustomCertificate => {
                let Some((cert, key)) = p.custom_cert_paths() else {
                    continue;
                };
                let desired_owned =
                    desired_cert_names(p.hostname.as_str(), p.include_wildcard_subdomains());
                let desired: Vec<&str> = desired_owned.iter().map(String::as_str).collect();
                let paths = CertPaths {
                    certificate: std::path::PathBuf::from(cert),
                    key: std::path::PathBuf::from(key),
                };
                if crate::commands::certs::validate_custom_cert_pair(
                    &paths.certificate,
                    &paths.key,
                    &desired,
                )
                .is_ok()
                {
                    out.insert(p.id.to_string(), paths);
                }
            }
            SslMode::SelfSigned | SslMode::PublicAcme => {}
        }
    }
    out
}

fn validate_public_acme_project(p: &crate::registry::Project) -> Result<(), String> {
    if is_local_only_name(&p.hostname) {
        return Err(
            "public ACME requires a publicly resolvable domain, not a local-only hostname".into(),
        );
    }
    let acme = p
        .domain
        .as_ref()
        .and_then(|d| d.acme.as_ref())
        .cloned()
        .unwrap_or_default();
    match acme.issuer {
        crate::registry::AcmeIssuer::LetsEncrypt => {}
        crate::registry::AcmeIssuer::ZeroSsl => {
            let has_api_key = acme
                .zerossl_api_key
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty());
            let has_eab = acme
                .eab_key_id
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
                && acme
                    .eab_hmac_key
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty());
            if !has_api_key && !has_eab {
                return Err(
                    "ZeroSSL ACME requires a ZeroSSL API key or EAB key id + HMAC key".into(),
                );
            }
        }
        crate::registry::AcmeIssuer::GoogleTrustServices => {
            let has_eab = acme
                .eab_key_id
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
                && acme
                    .eab_hmac_key
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty());
            if !has_eab {
                return Err("Google Trust Services ACME requires EAB key id + HMAC key".into());
            }
        }
    }
    if p.include_wildcard_subdomains() && acme.dns_provider == AcmeDnsProvider::None {
        return Err("wildcard public ACME requires DNS-01; select a DNS API provider".into());
    }
    if acme.dns_provider == AcmeDnsProvider::Cloudflare
        && acme
            .dns_api_token
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err("Cloudflare DNS-01 requires a Cloudflare API token".into());
    }
    Ok(())
}

fn is_local_only_name(hostname: &str) -> bool {
    let host = hostname.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "test"
        || host.ends_with(".test")
        || host == "local"
        || host.ends_with(".local")
        || host.parse::<std::net::IpAddr>().is_ok()
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

fn desired_cert_names(hostname: &str, include_wildcard: bool) -> Vec<String> {
    let mut names = vec![
        hostname.to_string(),
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if include_wildcard {
        names.push(format!("*.{hostname}"));
    }
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn cert_covers_requires_every_desired_name() {
        let single = vec!["app.test".to_string()];
        let both = vec!["app.test".to_string(), "*.app.test".to_string()];
        // A plain cert covers the bare host but not a newly-requested wildcard.
        assert!(crate::commands::certs::cert_covers_names(
            &single,
            &["app.test"]
        ));
        assert!(!crate::commands::certs::cert_covers_names(
            &single,
            &["app.test", "*.app.test"]
        ));
        // A wildcard cert covers both.
        assert!(crate::commands::certs::cert_covers_names(
            &both,
            &["app.test", "*.app.test"]
        ));
        // Missing / unparseable cert (empty SANs) covers nothing → reissue.
        assert!(!crate::commands::certs::cert_covers_names(
            &[],
            &["app.test"]
        ));
        // A stale cert for a renamed host doesn't cover the new hostname.
        assert!(!crate::commands::certs::cert_covers_names(
            &single,
            &["renamed.test"]
        ));
    }

    #[test]
    fn desired_cert_names_include_local_loopback_sans() {
        let names = desired_cert_names("app.test", true);
        assert!(names.contains(&"app.test".to_string()));
        assert!(names.contains(&"*.app.test".to_string()));
        assert!(names.contains(&"localhost".to_string()));
        assert!(names.contains(&"127.0.0.1".to_string()));
        assert!(names.contains(&"::1".to_string()));
    }

    fn https_project(id: &str) -> Project {
        Project {
            cors: None,
            sandbox: None,
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            framework: None,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
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

    fn http_project(id: &str) -> Project {
        let mut p = https_project(id);
        p.https = false;
        p.services.clear();
        p
    }

    #[test]
    fn no_mkcert_binary_returns_failed_and_empty_lookup() {
        let mut reg = Registry::new("test");
        reg.add_project(https_project("a")).unwrap();
        let mut cache = CertsCache::default();
        let r = reconcile(&reg, None, false, &mut cache);
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

        let lookup = lookup_only(&reg, Some(&m));
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
        let h1 = lookup_set_hash(&lookup_only(&reg, Some(&m)));

        reg.add_project(https_project("b")).unwrap();
        let dir = tmp.path().join("b");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("cert.pem"), b"x").unwrap();
        std::fs::write(dir.join("key.pem"), b"x").unwrap();
        let h2 = lookup_set_hash(&lookup_only(&reg, Some(&m)));

        assert_ne!(h1, h2);
    }
}
