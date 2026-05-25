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
use crate::registry::{DatabaseInstance, Project, Readiness, Registry, RuntimeSettings};

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

    /// When true, Process Compose loads the process definition but
    /// does NOT auto-start it on boot — the user must explicitly call
    /// /processes/start. We set this to `!project.auto_start` so the
    /// PortBay app boots in a quiet state: projects appear in the
    /// list, nothing runs until the user clicks Play. Without this
    /// field, PC's default behaviour is to launch every process it
    /// sees, which surprised users on fresh boots.
    disabled: bool,

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

/// SMTP defaults injected into every process's environment when
/// Mailpit is running. Frameworks that read `MAIL_HOST` /
/// `MAIL_PORT` / `MAIL_FROM_ADDRESS` / `MAIL_MAILER` (Laravel,
/// Symfony) pick up the local catcher automatically. Project-level
/// `env` overrides win — if the user has an explicit `MAIL_HOST`,
/// these defaults stay out of the way.
#[derive(Debug, Clone)]
pub struct MailpitEnv {
    pub smtp_port: u16,
    pub from_address: String,
}

impl MailpitEnv {
    pub fn with_smtp_port(smtp_port: u16) -> Self {
        Self {
            smtp_port,
            from_address: "hello@example.local".into(),
        }
    }
}

/// One PHP-FPM pool the reconciler wants Process Compose to keep
/// alive. Emitted as a process entry alongside the registered
/// projects so PC owns the lifecycle (same start/stop/log surface as
/// every other process).
#[derive(Debug, Clone)]
pub struct PhpFpmSpec {
    /// Stable process id (e.g. `php-fpm-8-3`). Same value [`crate::php::lifecycle::fpm_process_id`] returns.
    pub process_id: String,
    /// PHP version label this pool serves — used in the description.
    pub version: String,
    /// Absolute path to the `php-fpm` binary for this version.
    pub php_fpm_bin: std::path::PathBuf,
    /// Pool config file the daemon should read.
    pub pool_config: std::path::PathBuf,
    /// Working directory PC should `cd` into before spawning. Usually
    /// the same directory the pool config lives in.
    pub working_dir: std::path::PathBuf,
}

/// One database daemon the reconciler wants Process Compose to supervise.
/// The `command` is fully built by `crate::databases::run_command` (binary
/// resolved, paths quoted) so this layer stays engine-agnostic — same
/// contract as [`PhpFpmSpec`].
#[derive(Debug, Clone)]
pub struct DatabaseDaemonSpec {
    /// Stable PC process name, e.g. `db-myapp-pg`.
    pub process_id: String,
    /// Human description, e.g. `PostgreSQL 16 — myapp-pg`.
    pub description: String,
    /// Fully-built launch command.
    pub command: String,
    /// Directory PC `cd`s into before launching.
    pub working_dir: std::path::PathBuf,
    /// Listening port — drives the TCP readiness probe.
    pub port: u16,
    /// Honour the instance's auto_start flag.
    pub auto_start: bool,
}

/// One generated web-server daemon (Nginx or Apache) PortBay supervises for a
/// PHP document-root project. Caddy still owns the public hostname and TLS; it
/// reverse-proxies to this loopback port.
#[derive(Debug, Clone)]
pub struct WebServerSpec {
    /// Stable PC process name, e.g. `web-nginx-myapp`.
    pub process_id: String,
    /// Human description, e.g. `Nginx - myapp`.
    pub description: String,
    /// Fully-built launch command.
    pub command: String,
    /// Directory PC `cd`s into before launching.
    pub working_dir: std::path::PathBuf,
    /// Loopback HTTP port Caddy reverse-proxies to.
    pub port: u16,
    /// Honour the project's auto_start flag.
    pub auto_start: bool,
}

/// Build a YAML string from the registry.
///
/// `logs_dir` is the directory each per-process log file is written to
/// (e.g. `~/Library/Application Support/PortBay/logs/`).
///
/// `mail_env` injects SMTP defaults pointing at the local Mailpit
/// catcher into every process. Pass `None` when Mailpit isn't
/// running; pass `Some(MailpitEnv::with_smtp_port(port))` otherwise.
///
/// `php_fpm_specs` adds one PC process entry per running PHP version.
///
/// `db_specs` adds one PC process per supervised database instance.
/// Database connection env vars are also injected into any project the
/// instance is linked to (read from `reg.databases`), the same way Mailpit
/// env is injected — project-level `env` always overrides.
pub fn to_yaml(
    reg: &Registry,
    logs_dir: &Path,
    mail_env: Option<&MailpitEnv>,
    php_fpm_specs: &[PhpFpmSpec],
    db_specs: &[DatabaseDaemonSpec],
    web_specs: &[WebServerSpec],
) -> Result<String> {
    let mut processes = BTreeMap::new();
    for p in &reg.projects {
        if let Some(entry) =
            project_to_pc_process(p, logs_dir, mail_env, &reg.databases, &reg.runtimes)
        {
            processes.insert(p.id.to_string(), entry);
        }
    }
    for spec in php_fpm_specs {
        processes.insert(
            spec.process_id.clone(),
            php_fpm_to_pc_process(spec, logs_dir),
        );
    }
    for spec in db_specs {
        processes.insert(
            spec.process_id.clone(),
            db_daemon_to_pc_process(spec, logs_dir),
        );
    }
    for spec in web_specs {
        processes.insert(
            spec.process_id.clone(),
            web_server_to_pc_process(spec, logs_dir),
        );
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

fn php_fpm_to_pc_process(spec: &PhpFpmSpec, logs_dir: &Path) -> PcProcess {
    let log_path = logs_dir.join(format!("{}.log", spec.process_id));
    // `php-fpm -F` keeps it foreground; `-y` points at the pool
    // config. Quoting via shell would risk newlines — Process
    // Compose accepts the command as a single string, so we glue
    // with spaces and trust the absolute paths.
    let command = format!(
        "{bin} -F -y {cfg}",
        bin = spec.php_fpm_bin.to_string_lossy(),
        cfg = spec.pool_config.to_string_lossy(),
    );
    PcProcess {
        description: Some(format!("PHP-FPM {}", spec.version)),
        working_dir: spec.working_dir.to_string_lossy().into_owned(),
        command,
        // PHP-FPM pools are infrastructure — they're only ever in the
        // PC list when at least one PHP project pins their version,
        // and they must be running for that project to serve. Auto-start.
        disabled: false,
        availability: PcAvailability { restart: "no" },
        readiness_probe: None,
        log_location: log_path.to_string_lossy().into_owned(),
        environment: BTreeMap::new(),
        shutdown: PcShutdown {
            signal: 15,
            timeout_seconds: 5,
        },
    }
}

/// Emit a PC process for a supervised database daemon. The daemon gets a
/// TCP readiness probe on its port so the project's readiness gate (and
/// the GUI status pill) reflect "actually accepting connections", not
/// merely "process spawned".
fn db_daemon_to_pc_process(spec: &DatabaseDaemonSpec, logs_dir: &Path) -> PcProcess {
    let log_path = logs_dir.join(format!("{}.log", spec.process_id));
    PcProcess {
        description: Some(spec.description.clone()),
        working_dir: spec.working_dir.to_string_lossy().into_owned(),
        command: spec.command.clone(),
        disabled: !spec.auto_start,
        availability: PcAvailability { restart: "no" },
        readiness_probe: Some(PcReadinessProbe {
            http_get: None,
            tcp_socket: Some(PcTcpSocket {
                host: "127.0.0.1",
                port: spec.port,
            }),
            initial_delay_seconds: 1,
            period_seconds: 2,
            timeout_seconds: 5,
            success_threshold: 1,
            failure_threshold: 30,
        }),
        log_location: log_path.to_string_lossy().into_owned(),
        environment: BTreeMap::new(),
        shutdown: PcShutdown {
            signal: 15,
            timeout_seconds: 10,
        },
    }
}

fn web_server_to_pc_process(spec: &WebServerSpec, logs_dir: &Path) -> PcProcess {
    let log_path = logs_dir.join(format!("{}.log", spec.process_id));
    PcProcess {
        description: Some(spec.description.clone()),
        working_dir: spec.working_dir.to_string_lossy().into_owned(),
        command: spec.command.clone(),
        disabled: !spec.auto_start,
        availability: PcAvailability { restart: "no" },
        readiness_probe: Some(PcReadinessProbe {
            http_get: Some(PcHttpGet {
                host: "127.0.0.1",
                scheme: "http",
                path: "/".into(),
                port: spec.port,
            }),
            tcp_socket: None,
            initial_delay_seconds: 1,
            period_seconds: 2,
            timeout_seconds: 5,
            success_threshold: 1,
            failure_threshold: 30,
        }),
        log_location: log_path.to_string_lossy().into_owned(),
        environment: BTreeMap::new(),
        shutdown: PcShutdown {
            signal: 15,
            timeout_seconds: 10,
        },
    }
}

fn project_to_pc_process(
    p: &Project,
    logs_dir: &Path,
    mail_env: Option<&MailpitEnv>,
    databases: &[DatabaseInstance],
    runtimes: &RuntimeSettings,
) -> Option<PcProcess> {
    // The command to launch. An explicit `start_command` always wins. Failing
    // that, a monorepo project derives a workspace-filtered command from its
    // `workspace` binding (e.g. `pnpm --filter @app/web dev`) so it runs one
    // app from the repo root instead of fanning out. A project with neither
    // (a pure Caddy-served site) produces no PC entry.
    let command = match &p.start_command {
        Some(cmd) => cmd.clone(),
        None => p.workspace.as_ref().map(|ws| ws.derive_dev_command())?,
    };

    let log_path = logs_dir.join(format!("{}.log", p.id));
    let readiness_probe = p
        .readiness
        .as_ref()
        .and_then(|r| readiness_to_pc_probe(r, p.port));

    let mut environment = BTreeMap::new();
    // Inject Mailpit defaults first; the per-project env below
    // overrides any key the user has set explicitly.
    if let Some(mail) = mail_env {
        environment.insert("MAIL_HOST".into(), "127.0.0.1".into());
        environment.insert("MAIL_PORT".into(), mail.smtp_port.to_string());
        environment.insert("MAIL_FROM_ADDRESS".into(), mail.from_address.clone());
        environment.insert("MAIL_MAILER".into(), "smtp".into());
        environment.insert("MAIL_ENCRYPTION".into(), "null".into());
    }
    // Inject connection env for any database instance linked to this
    // project. If multiple instances link the same project, the last one
    // wins for the shared keys (DATABASE_URL, DB_*) — the per-instance
    // values are still reachable, but a project should normally bind one
    // primary database. Project-level env (below) overrides everything.
    for db in databases {
        if db.linked_projects.iter().any(|pid| pid == &p.id) {
            for (k, v) in db.connection_env() {
                environment.insert(k, v);
            }
        }
    }
    for (k, v) in &p.env {
        environment.insert(k.clone(), v.clone());
    }
    inject_runtime_path(p, runtimes, &mut environment);

    Some(PcProcess {
        description: Some(p.name.clone()),
        working_dir: p.path.to_string_lossy().into_owned(),
        command,
        // Honour the registry's `auto_start` flag literally: when the
        // user hasn't opted in, the process exists in PC's list but
        // is dormant until they click Play. Defaults to false so a
        // fresh app boot is quiet — projects show up but nothing
        // runs.
        disabled: !p.auto_start,
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

fn inject_runtime_path(
    p: &Project,
    runtimes: &RuntimeSettings,
    environment: &mut BTreeMap<String, String>,
) {
    let Some(runtime) = &p.runtime else {
        return;
    };
    let Some(binary) = crate::runtimes::resolve_binary(runtime, runtimes) else {
        return;
    };
    let Some(bin_dir) = binary.parent() else {
        return;
    };
    let current = environment
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    environment.insert(
        "PATH".into(),
        format!("{}:{current}", bin_dir.to_string_lossy()),
    );
    environment.insert("PORTBAY_RUNTIME_LANG".into(), runtime.lang.clone());
    environment.insert("PORTBAY_RUNTIME_VERSION".into(), runtime.version.clone());
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
                failure_threshold: (timeout_seconds / 2).clamp(5, 120),
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
                failure_threshold: (timeout_seconds / 2).clamp(5, 120),
            })
        }
        Readiness::Process => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ManualRuntime, Project, ProjectId, ProjectType, Readiness, Runtime};
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
            env: Default::default(),
            readiness: Some(Readiness::Http {
                path: "/".into(),
                timeout_seconds: 75,
            }),
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

    fn php_project(id: &str) -> Project {
        Project {
            cors: None,
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
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
        }
    }

    #[test]
    fn empty_registry_produces_minimal_yaml() {
        let r = Registry::new("test");
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        assert!(yaml.contains("version: '0.5'") || yaml.contains("version: \"0.5\""));
        assert!(yaml.contains("processes: {}"));
    }

    #[test]
    fn next_project_produces_process_with_http_probe() {
        let mut r = Registry::new("test");
        r.add_project(next_project("marketing-site", 3010)).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp/logs"), None, &[], &[], &[]).unwrap();
        assert!(
            yaml.contains("marketing-site"),
            "process name missing: {yaml}"
        );
        assert!(yaml.contains("pnpm dev"));
        assert!(yaml.contains("port: 3010"));
        assert!(yaml.contains("path: /"));
        assert!(yaml.contains("scheme: http"));
        assert!(yaml.contains("/tmp/logs/marketing-site.log"));
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
        r.add_project(php_project("api-gateway")).unwrap();
        r.add_project(next_project("marketing-site", 3010)).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        assert!(!yaml.contains("api-gateway"));
        assert!(yaml.contains("marketing-site"));
    }

    #[test]
    fn workspace_project_without_start_command_derives_filtered_command() {
        use crate::registry::{Workspace, WorkspaceTool};
        // A monorepo app pinned by workspace filter, no explicit start_command:
        // the PC entry should run the filtered dev command from the repo root.
        let mut p = next_project("bookslash-web", 3000);
        p.start_command = None;
        p.path = PathBuf::from("/repos/BookSlash");
        p.workspace = Some(Workspace {
            package: "@bookslash/web".into(),
            rel_dir: "apps/web".into(),
            tool: WorkspaceTool::Pnpm,
        });
        let mut r = Registry::new("test");
        r.add_project(p).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        assert!(
            yaml.contains("pnpm --filter @bookslash/web dev"),
            "expected the derived filter command in:\n{yaml}"
        );
        // working_dir stays the monorepo root, not the sub-app dir.
        assert!(yaml.contains("/repos/BookSlash"));
    }

    #[test]
    fn project_env_vars_appear_in_yaml() {
        let mut p = next_project("env-test", 3010);
        p.env.insert("DATABASE_URL".into(), "postgres://x".into());
        p.env.insert("NODE_ENV".into(), "development".into());
        let mut r = Registry::new("test");
        r.add_project(p).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        assert!(yaml.contains("DATABASE_URL"));
        assert!(yaml.contains("postgres://x"));
        assert!(yaml.contains("NODE_ENV"));
    }

    #[test]
    fn project_runtime_binary_dir_is_prepended_to_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("node-22").join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let node = bin_dir.join("node");
        std::fs::write(&node, "#!/bin/sh\necho v22.1.0\n").unwrap();

        let mut p = next_project("runtime-path", 3010);
        p.runtime = Some(Runtime {
            lang: "node".into(),
            version: "22.1.0".into(),
        });
        let mut r = Registry::new("test");
        r.runtimes.manual.push(ManualRuntime {
            lang: "node".into(),
            version: "22.1.0".into(),
            binary: node,
        });
        r.add_project(p).unwrap();

        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let env = &doc["processes"]["runtime-path"]["environment"];
        let path = env["PATH"].as_str().unwrap();
        assert!(
            path.starts_with(&bin_dir.to_string_lossy().to_string()),
            "expected runtime bin dir first in PATH, got {path}"
        );
        assert_eq!(env["PORTBAY_RUNTIME_LANG"].as_str(), Some("node"));
        assert_eq!(env["PORTBAY_RUNTIME_VERSION"].as_str(), Some("22.1.0"));
    }

    #[test]
    fn mail_env_injects_smtp_defaults_when_present() {
        let mut r = Registry::new("test");
        r.add_project(next_project("withmail", 3010)).unwrap();
        let mail = MailpitEnv::with_smtp_port(1025);
        let yaml = to_yaml(&r, Path::new("/tmp"), Some(&mail), &[], &[], &[]).unwrap();
        assert!(yaml.contains("MAIL_HOST: 127.0.0.1"));
        assert!(yaml.contains("MAIL_PORT: '1025'") || yaml.contains("MAIL_PORT: \"1025\""));
        assert!(yaml.contains("MAIL_MAILER: smtp"));
        assert!(yaml.contains("hello@example.local"));
    }

    #[test]
    fn project_env_overrides_mail_defaults() {
        let mut p = next_project("override", 3010);
        p.env
            .insert("MAIL_HOST".into(), "mail.production.test".into());
        let mut r = Registry::new("test");
        r.add_project(p).unwrap();
        let mail = MailpitEnv::with_smtp_port(1025);
        let yaml = to_yaml(&r, Path::new("/tmp"), Some(&mail), &[], &[], &[]).unwrap();
        // Project's explicit override wins; the default loopback is gone.
        assert!(yaml.contains("MAIL_HOST: mail.production.test"));
        assert!(!yaml.contains("MAIL_HOST: 127.0.0.1"));
    }

    #[test]
    fn no_mail_env_leaves_yaml_without_mail_vars() {
        let mut r = Registry::new("test");
        r.add_project(next_project("nomail", 3010)).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        assert!(!yaml.contains("MAIL_HOST"));
        assert!(!yaml.contains("MAIL_PORT"));
    }

    #[test]
    fn process_readiness_variant_produces_no_probe() {
        let mut p = next_project("no-probe", 3010);
        p.readiness = Some(Readiness::Process);
        let mut r = Registry::new("test");
        r.add_project(p).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        assert!(!yaml.contains("readiness_probe"));
    }

    #[test]
    fn yaml_is_parseable_by_serde_yaml_round_trip() {
        let mut r = Registry::new("test");
        r.add_project(next_project("a", 3010)).unwrap();
        r.add_project(next_project("b", 3011)).unwrap();
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[], &[], &[]).unwrap();
        let back: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        // Just confirm structural validity — content was checked above.
        assert!(back.get("processes").is_some());
        assert_eq!(back["processes"]["a"]["command"].as_str(), Some("pnpm dev"));
        assert_eq!(back["processes"]["b"]["command"].as_str(), Some("pnpm dev"));
    }

    #[test]
    fn php_fpm_specs_emit_one_process_each() {
        let r = Registry::new("test");
        let specs = vec![
            PhpFpmSpec {
                process_id: "php-fpm-8-3".into(),
                version: "8.3".into(),
                php_fpm_bin: PathBuf::from("/opt/homebrew/opt/php@8.3/sbin/php-fpm"),
                pool_config: PathBuf::from("/tmp/portbay/php/8.3/php-fpm.conf"),
                working_dir: PathBuf::from("/tmp/portbay/php/8.3"),
            },
            PhpFpmSpec {
                process_id: "php-fpm-7-4".into(),
                version: "7.4".into(),
                php_fpm_bin: PathBuf::from("/opt/homebrew/opt/php@7.4/sbin/php-fpm"),
                pool_config: PathBuf::from("/tmp/portbay/php/7.4/php-fpm.conf"),
                working_dir: PathBuf::from("/tmp/portbay/php/7.4"),
            },
        ];
        let yaml = to_yaml(&r, Path::new("/tmp/logs"), None, &specs, &[], &[]).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let procs = &doc["processes"];

        for id in ["php-fpm-8-3", "php-fpm-7-4"] {
            let p = &procs[id];
            assert!(p.is_mapping(), "process `{id}` missing");
            let cmd = p["command"].as_str().unwrap();
            assert!(cmd.contains("-F -y"), "command missing FPM flags: {cmd}");
            assert!(cmd.contains("/php-fpm"), "command missing binary: {cmd}");
            // Log location is per-process_id, not per-project name.
            let log = p["log_location"].as_str().unwrap();
            assert!(
                log.ends_with(&format!("{id}.log")),
                "log path mismatch: {log}"
            );
            // Description carries the human-readable version label.
            let desc = p["description"].as_str().unwrap();
            assert!(desc.starts_with("PHP-FPM "), "description shape: {desc}");
        }
    }

    #[test]
    fn php_fpm_specs_do_not_collide_with_project_ids() {
        // A project literally named `php-fpm-8-3` should still be
        // emitted; the spec entry uses the same key but only one
        // wins. Ensure project entries take precedence (since the
        // user's registry choices outrank our derived process ids).
        let mut r = Registry::new("test");
        r.add_project(next_project("php-fpm-8-3", 3000)).unwrap();
        let spec = PhpFpmSpec {
            process_id: "php-fpm-8-3".into(),
            version: "8.3".into(),
            php_fpm_bin: PathBuf::from("/x/bin/php-fpm"),
            pool_config: PathBuf::from("/x/conf"),
            working_dir: PathBuf::from("/x"),
        };
        let yaml = to_yaml(&r, Path::new("/tmp"), None, &[spec], &[], &[]).unwrap();
        // The spec inserts AFTER the project loop, so it overwrites —
        // documenting that here. If we later want the project to win,
        // swap the iteration order.
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert!(doc["processes"]["php-fpm-8-3"].is_mapping());
    }

    #[test]
    fn web_server_specs_emit_http_readiness_probe() {
        let r = Registry::new("test");
        let spec = WebServerSpec {
            process_id: "web-nginx-cms".into(),
            description: "Nginx - CMS".into(),
            command: "nginx -c /tmp/nginx.conf -g 'daemon off;'".into(),
            working_dir: PathBuf::from("/tmp/portbay/webservers/nginx/cms"),
            port: 9080,
            auto_start: true,
        };
        let yaml = to_yaml(&r, Path::new("/tmp/logs"), None, &[], &[], &[spec]).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let proc = &doc["processes"]["web-nginx-cms"];
        assert_eq!(
            proc["command"].as_str(),
            Some("nginx -c /tmp/nginx.conf -g 'daemon off;'")
        );
        assert_eq!(
            proc["readiness_probe"]["http_get"]["port"].as_u64(),
            Some(9080)
        );
        assert_eq!(
            proc["readiness_probe"]["http_get"]["path"].as_str(),
            Some("/")
        );
        assert_eq!(proc["disabled"].as_bool(), Some(false));
    }
}
