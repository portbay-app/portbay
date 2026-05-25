//! Domain suffix validation and registry migration.

use std::path::PathBuf;

use crate::registry::Registry;

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainMigration {
    pub old_suffix: String,
    pub new_suffix: String,
    pub changed_projects: usize,
    pub cert_dirs_removed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("domain suffix is required")]
    Empty,

    #[error("domain suffix `{0}` has an invalid label (use letters, digits and hyphens)")]
    InvalidLabel(String),

    #[error("`{0}` is a public TLD — pick a local-only suffix like `test` or `portbay.test`")]
    Reserved(String),

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, DomainError>;

const RESERVED_SUFFIXES: &[&str] = &[
    "com", "net", "org", "edu", "gov", "mil", "int", "io", "co", "ai", "app", "cloud", "site",
    "online",
];

pub fn normalise_domain_suffix(input: &str) -> Result<String> {
    let suffix = input
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if suffix.is_empty() {
        return Err(DomainError::Empty);
    }
    if suffix.len() > 253 {
        return Err(DomainError::InvalidLabel(suffix));
    }

    // Multi-label suffixes are allowed (e.g. `portbay.test`), so validate
    // each dot-separated label independently. Each must be a well-formed DNS
    // label: 1–63 chars of [a-z0-9-], no leading/trailing hyphen.
    let labels: Vec<&str> = suffix.split('.').collect();
    for label in &labels {
        let ok = !label.is_empty()
            && label.len() <= 63
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !ok {
            return Err(DomainError::InvalidLabel(suffix));
        }
    }

    // Guard only the *final* label against public TLDs. This keeps the
    // "don't hijack real DNS" protection (`app.com` → rejected) while
    // permitting local-safe two-label suffixes like `portbay.test`, whose
    // final label `test` is RFC 6761-reserved for local use.
    let final_label = *labels.last().expect("non-empty after split");
    if RESERVED_SUFFIXES.contains(&final_label) {
        return Err(DomainError::Reserved(suffix));
    }
    Ok(suffix)
}

pub fn migrate_registry_suffix(
    reg: &mut Registry,
    input: &str,
    certs_root: Option<PathBuf>,
) -> Result<DomainMigration> {
    let new_suffix = normalise_domain_suffix(input)?;
    let old_suffix = normalise_domain_suffix(&reg.domain_suffix)?;
    if old_suffix == new_suffix {
        reg.domain_suffix = new_suffix.clone();
        return Ok(DomainMigration {
            old_suffix,
            new_suffix,
            changed_projects: 0,
            cert_dirs_removed: 0,
        });
    }

    let old_tail = format!(".{old_suffix}");
    let mut changed_projects = 0usize;
    let mut cert_dirs_removed = 0usize;

    for project in &mut reg.projects {
        if let Some(stem) = project.hostname.strip_suffix(&old_tail) {
            project.hostname = format!("{stem}.{new_suffix}");
        } else {
            project.hostname = format!("{}.{}", project.id.as_str(), new_suffix);
        }
        changed_projects += 1;

        if project.https {
            if let Some(root) = certs_root.as_ref() {
                let dir = root.join(project.id.as_str());
                if dir.exists() {
                    std::fs::remove_dir_all(&dir).map_err(|source| DomainError::Io {
                        path: dir.clone(),
                        source,
                    })?;
                    cert_dirs_removed += 1;
                }
            }
        }
    }

    reg.domain_suffix = new_suffix.clone();
    Ok(DomainMigration {
        old_suffix,
        new_suffix,
        changed_projects,
        cert_dirs_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType, Registry};
    use std::collections::BTreeMap;

    fn project(id: &str, hostname: &str, https: bool) -> Project {
        Project {
            cors: None,
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3000),
            extra_ports: vec![],
            hostname: hostname.into(),
            https,
            services: vec![],
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
    fn normalise_accepts_local_suffixes() {
        assert_eq!(normalise_domain_suffix(".test").unwrap(), "test");
        assert_eq!(normalise_domain_suffix("localhost").unwrap(), "localhost");
        assert_eq!(normalise_domain_suffix("dev").unwrap(), "dev");
    }

    #[test]
    fn normalise_accepts_multi_label_local_suffixes() {
        // The branding suffix: two labels, final label `test` is local-safe.
        assert_eq!(
            normalise_domain_suffix("portbay.test").unwrap(),
            "portbay.test"
        );
        assert_eq!(
            normalise_domain_suffix(".PortBay.Test.").unwrap(),
            "portbay.test"
        );
        assert_eq!(
            normalise_domain_suffix("app.local.dev").unwrap(),
            "app.local.dev"
        );
    }

    #[test]
    fn normalise_rejects_public_or_malformed_suffixes() {
        // Single public TLD.
        assert!(matches!(
            normalise_domain_suffix("com"),
            Err(DomainError::Reserved(_))
        ));
        // Multi-label whose *final* label is a public TLD must still be
        // rejected so we never shadow real DNS.
        assert!(matches!(
            normalise_domain_suffix("portbay.com"),
            Err(DomainError::Reserved(_))
        ));
        assert!(matches!(
            normalise_domain_suffix("-bad"),
            Err(DomainError::InvalidLabel(_))
        ));
        // Empty labels (double dot) are malformed.
        assert!(matches!(
            normalise_domain_suffix("portbay..test"),
            Err(DomainError::InvalidLabel(_))
        ));
        // Trailing-hyphen label is malformed.
        assert!(matches!(
            normalise_domain_suffix("bad-.test"),
            Err(DomainError::InvalidLabel(_))
        ));
    }

    #[test]
    fn migration_rewrites_to_multi_label_suffix() {
        // The rebrand path: an existing `.test` install migrates to
        // `.portbay.test`, renaming every project hostname.
        let mut reg = Registry::new("test");
        reg.add_project(project("app", "app.test", false)).unwrap();
        reg.add_project(project("api", "api.test", false)).unwrap();
        let migration = migrate_registry_suffix(&mut reg, "portbay.test", None).unwrap();
        assert_eq!(migration.changed_projects, 2);
        assert_eq!(reg.domain_suffix, "portbay.test");
        assert_eq!(reg.projects[0].hostname, "app.portbay.test");
        assert_eq!(reg.projects[1].hostname, "api.portbay.test");
    }

    #[test]
    fn migration_rewrites_matching_and_custom_hosts() {
        let mut reg = Registry::new("test");
        reg.add_project(project("app", "app.test", true)).unwrap();
        reg.add_project(project("api", "custom.internal", false))
            .unwrap();
        let migration = migrate_registry_suffix(&mut reg, "localhost", None).unwrap();
        assert_eq!(migration.changed_projects, 2);
        assert_eq!(reg.domain_suffix, "localhost");
        assert_eq!(reg.projects[0].hostname, "app.localhost");
        assert_eq!(reg.projects[1].hostname, "api.localhost");
    }

    #[test]
    fn migration_removes_https_cert_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut reg = Registry::new("test");
        reg.add_project(project("app", "app.test", true)).unwrap();
        let dir = tmp.path().join("app");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("cert.pem"), "cert").unwrap();
        let migration =
            migrate_registry_suffix(&mut reg, "localhost", Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(migration.cert_dirs_removed, 1);
        assert!(!dir.exists());
    }
}
