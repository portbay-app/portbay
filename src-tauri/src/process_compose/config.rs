//! Translates a `Registry` into a `process-compose.yaml` document.
//!
//! Only projects with a `start_command` produce a Process Compose entry —
//! a PHP / static project served entirely via Caddy + PHP-FPM has nothing
//! for PC to launch and is reconciled at the Caddy layer instead.
//!
//! The YAML shape follows Process Compose's v0.5 schema, validated by the
//! `--dry-run` step in the Phase 0 spike.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::process_compose::error::Result;
use crate::registry::{Project, Readiness, Registry};

/// Top-level YAML document for Process Compose.
#[derive(Debug, Serialize)]
struct PcDocument<'a> {
    version: &'static str,

    #[serde(skip_serializing_if = "Option::is_none")]
    log_location: Option<&'a str>,

    log_level: &'static str,

    /// We always start PC with `--keep-project` from the CLI (spike Quirk 3),
    /// so this field is informational; PC accepts it but `--keep-project`
    /// is the authoritative flag.
    keep_project: bool,

    processes: BTreeMap<String, PcProcess>,
}

#[derive(Debug, Serialize)]
struct PcProcess {
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,

    working_dir: String,

    command: String,

    availability: PcAvailability,

    #[serde(skip_serializing_if = "Option::is_none")]
    readiness_probe: Option<PcReadinessProbe>,

    log_location: String,

    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    environment: BTreeMap<String, String>,

    shutdown: PcShutdown,
}

#[derive(Debug, Serialize)]
struct PcAvailability {
    /// PortBay owns lifecycle — we restart processes when the user clicks,
    /// not on PC's own schedule. Always "no".
    restart: &'static str,
}

#[derive(Debug, Serialize)]
struct PcShutdown {
    signal: u8,
    timeout_seconds: u32,
}

#[derive(Debug, Serialize)]
struct PcReadinessProbe {
    #[serde(skip_serializing_if = "Option::is_none")]
    http_get: Option<PcHttpGet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tcp_socket: Option<PcTcpSocket>,
    initial_delay_seconds: u32,
    period_seconds: u32,
    timeout_seconds: u32,
    success_threshold: u32,
    failure_threshold: u32,
}

#[derive(Debug, Serialize)]
struct PcHttpGet {
    host: &'static str,
    scheme: &'static str,
    path: String,
    port: u16,
}

#[derive(Debug, Serialize)]
struct PcTcpSocket {
    host: &'static str,
    port: u16,
}

/// Build a YAML string from the registry.
///
/// `logs_dir` is the directory each per-process log file is written to
/// (e.g. `~/Library/Application Support/PortBay/logs/`).
pub fn to_yaml(reg: &Registry, logs_dir: &Path) -> Result<String> {
    let mut processes = BTreeMap::new();
    for p in &reg.projects {
        if let Some(entry) = project_to_pc_process(p, logs_dir) {
            processes.insert(p.id.to_string(), entry);
        }
    }

    let global_log = logs_dir.join("process-compose.log");
    let doc = PcDocument {
        version: "0.5",
        log_location: global_log.to_str(),
        log_level: "info",
        keep_project: true,
        processes,
    };

    Ok(serde_yaml::to_string(&doc)?)
}

fn project_to_pc_process(p: &Project, logs_dir: &Path) -> Option<PcProcess> {
    // Projects without a start_command (pure Caddy-served sites) don't
    // produce a PC entry.
    let command = p.start_command.clone()?;

    let log_path = logs_dir.join(format!("{}.log", p.id));
    let readiness_probe = p
        .readiness
        .as_ref()
        .and_then(|r| readiness_to_pc_probe(r, p.port));

    let mut environment = BTreeMap::new();
    for (k, v) in &p.env {
        environment.insert(k.clone(), v.clone());
    }

    Some(PcProcess {
        description: Some(p.name.clone()),
        working_dir: p.path.to_string_lossy().into_owned(),
        command,
        availability: PcAvailability { restart: "no" },
        readiness_probe,
        log_location: log_path.to_string_lossy().into_owned(),
        environment,
        shutdown: PcShutdown {
            signal: 15, // SIGTERM
            timeout_seconds: 10,
        },
    })
}

fn readiness_to_pc_probe(r: &Readiness, port: Option<u16>) -> Option<PcReadinessProbe> {
    match r {
        Readiness::Http {
            path,
            timeout_seconds,
        } => {
            let port = port?;
            Some(PcReadinessProbe {
                http_get: Some(PcHttpGet {
                    host: "127.0.0.1",
                    scheme: "http",
                    path: path.clone(),
                    port,
                }),
                tcp_socket: None,
                initial_delay_seconds: 2,
                period_seconds: 2,
                timeout_seconds: 5,
                success_threshold: 1,
                // Failure threshold expressed in probe periods, derived
                // from the user's timeout. 75s / 2s ≈ 38 attempts.
                failure_threshold: ((timeout_seconds / 2).max(5)).min(120),
            })
        }
        Readiness::Tcp { timeout_seconds } => {
            let port = port?;
            Some(PcReadinessProbe {
                http_get: None,
                tcp_socket: Some(PcTcpSocket {
                    host: "127.0.0.1",
                    port,
                }),
                initial_delay_seconds: 2,
                period_seconds: 2,
                timeout_seconds: 5,
                success_threshold: 1,
                failure_threshold: ((timeout_seconds / 2).max(5)).min(120),
            })
        }
        Readiness::Process => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType, Readiness};
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
            env: Default::default(),
            readiness: Some(Readiness::Http {
                path: "/".into(),
                timeout_seconds: 75,
            }),
            auto_start: false,
            tags: vec![],
            document_root: None,
            php_version: None,
        }
    }

    fn php_project(id: &str) -> Project {
        Project {
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Php,
            start_command: None, // pure Caddy-served — no PC entry
            port: None,
            extra_ports: vec![],
            hostname: format!("{id}.test"),
            https: true,
            services: vec!["caddy".into(), "php-fpm".into()],
            env: Default::default(),
            readiness: None,
            auto_start: false,
            tags: vec![],
            document_root: Some("public".into()),
            php_version: Some("8.3".into()),
        }
    }

    #[test]
    fn empty_registry_produces_minimal_yaml() {
        let r = Registry::new("test");
        let yaml = to_yaml(&r, Path::new("/tmp")).unwrap();
        assert!(yaml.contains("version: '0.5'") || yaml.contains("version: \"0.5\""));
        assert!(yaml.contains("processes: {}"));
    }

    #[test]
    fn next_project_produces_process_with_http_probe() {
        let mut r = Registry::new("test");
        r.add_project(next_project("nour-beiruti", 3010)).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp/logs")).unwrap();
        assert!(yaml.contains("nour-beiruti"), "process name missing: {yaml}");
        assert!(yaml.contains("pnpm dev"));
        assert!(yaml.contains("port: 3010"));
        assert!(yaml.contains("path: /"));
        assert!(yaml.contains("scheme: http"));
        assert!(yaml.contains("/tmp/logs/nour-beiruti.log"));
        // serde_yaml 0.9 emits bare `no` (a string in YAML 1.2); PC reads
        // it as the string "no", which is what its schema expects. Confirm
        // the field is present in any form.
        assert!(
            yaml.contains("restart: no")
                || yaml.contains("restart: 'no'")
                || yaml.contains("restart: \"no\""),
            "expected `restart: no` in YAML, got: {yaml}"
        );
    }

    #[test]
    fn php_only_project_is_skipped() {
        let mut r = Registry::new("test");
        r.add_project(php_project("tribal-house")).unwrap();
        r.add_project(next_project("nour-beiruti", 3010)).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp")).unwrap();
        assert!(!yaml.contains("tribal-house"));
        assert!(yaml.contains("nour-beiruti"));
    }

    #[test]
    fn project_env_vars_appear_in_yaml() {
        let mut p = next_project("env-test", 3010);
        p.env.insert("DATABASE_URL".into(), "postgres://x".into());
        p.env.insert("NODE_ENV".into(), "development".into());
        let mut r = Registry::new("test");
        r.add_project(p).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp")).unwrap();
        assert!(yaml.contains("DATABASE_URL"));
        assert!(yaml.contains("postgres://x"));
        assert!(yaml.contains("NODE_ENV"));
    }

    #[test]
    fn process_readiness_variant_produces_no_probe() {
        let mut p = next_project("no-probe", 3010);
        p.readiness = Some(Readiness::Process);
        let mut r = Registry::new("test");
        r.add_project(p).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp")).unwrap();
        assert!(!yaml.contains("readiness_probe"));
    }

    #[test]
    fn yaml_is_parseable_by_serde_yaml_round_trip() {
        let mut r = Registry::new("test");
        r.add_project(next_project("a", 3010)).unwrap();
        r.add_project(next_project("b", 3011)).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp")).unwrap();
        let back: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        // Just confirm structural validity — content was checked above.
        assert!(back.get("processes").is_some());
        assert_eq!(back["processes"]["a"]["command"].as_str(), Some("pnpm dev"));
        assert_eq!(back["processes"]["b"]["command"].as_str(), Some("pnpm dev"));
    }
}
