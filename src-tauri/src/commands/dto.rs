//! Public IPC data shapes.
//!
//! These types are the contract between the Rust core and the Svelte
//! frontend. They are *not* the internal types — `Project`, `Process`,
//! `Registry` are private to the core. Anything crossing Tauri's invoke /
//! event boundary goes through this module.
//!
//! Field naming is `camelCase` because that's the convention the frontend
//! consumes. Serde renames at the type level keep Rust idiomatic on this
//! side without leaking snake_case across the wire.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::process_compose::{Process, ProjectStatus};
use crate::registry::{Project, ProjectType, Readiness};

/// A merged registry + runtime view of one project.
///
/// `status` is always present — defaults to `Stopped` when the daemon is
/// unreachable. `runtime` is `Some` only when PC has live data for this
/// project (i.e. it's been started at least once this session).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: ProjectType,
    pub start_command: Option<String>,
    pub port: Option<u16>,
    pub extra_ports: Vec<u16>,
    pub hostname: String,
    pub url: String,
    pub https: bool,
    pub services: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub readiness: Option<Readiness>,
    pub auto_start: bool,
    pub tags: Vec<String>,
    pub document_root: Option<String>,
    pub php_version: Option<String>,

    /// PortBay status taxonomy (`docs/UX_DESIGN.md` §5.3).
    pub status: ProjectStatus,

    /// Live runtime details. `None` when PC has no record for this project.
    pub runtime: Option<RuntimeInfo>,
}

impl ProjectView {
    pub fn from_project(project: &Project, proc: Option<&Process>) -> Self {
        let scheme = if project.https { "https" } else { "http" };
        let url = format!("{scheme}://{}", project.hostname);
        Self {
            id: project.id.as_str().into(),
            name: project.name.clone(),
            path: project.path.to_string_lossy().into_owned(),
            kind: project.kind,
            start_command: project.start_command.clone(),
            port: project.port,
            extra_ports: project.extra_ports.clone(),
            hostname: project.hostname.clone(),
            url,
            https: project.https,
            services: project.services.clone(),
            env: project.env.clone(),
            readiness: project.readiness.clone(),
            auto_start: project.auto_start,
            tags: project.tags.clone(),
            document_root: project.document_root.clone(),
            php_version: project.php_version.clone(),
            status: proc
                .map(|p| p.portbay_status())
                .unwrap_or(ProjectStatus::Stopped),
            runtime: proc.map(RuntimeInfo::from_process),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub pid: u32,
    pub restarts: u32,
    /// PC's last-observed readiness string. Stale after the process exits —
    /// trust `status` for current truth (see `Process::is_serving`).
    pub is_ready: String,
    pub has_ready_probe: bool,
    pub exit_code: i32,
    /// Process age in nanoseconds (PC's native unit).
    pub age: u64,
    pub mem_bytes: u64,
    pub cpu_percent: f64,
}

impl RuntimeInfo {
    pub fn from_process(p: &Process) -> Self {
        Self {
            pid: p.pid,
            restarts: p.restarts,
            is_ready: p.is_ready.clone(),
            has_ready_probe: p.has_ready_probe,
            exit_code: p.exit_code,
            age: p.age,
            mem_bytes: p.mem,
            cpu_percent: p.cpu,
        }
    }
}

/// Emitted on `portbay://status` whenever the reconcile loop observes a
/// project's status (or runtime) transition.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatusEvent {
    pub id: String,
    pub status: ProjectStatus,
    pub runtime: Option<RuntimeInfo>,
    /// Last error observed on this project, if any. Carries through to the
    /// detail panel's "last error" line.
    pub last_error: Option<String>,
    /// Unix milliseconds since epoch.
    pub ts: u64,
}

/// Per-project outcome from a `stop_all` invocation. The frontend renders
/// this as a toast + per-row inline errors for failures.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopAllReport {
    pub stopped: u32,
    pub failed: u32,
    pub results: Vec<StopAllResultEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopAllResultEntry {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Health of one sidecar PortBay manages.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarStatus {
    pub name: &'static str,
    pub status: SidecarState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarState {
    Running,
    Stopped,
    NotInstalled,
    Unreachable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarHealth {
    pub process_compose: SidecarStatus,
    pub caddy: SidecarStatus,
    pub mkcert_ca: SidecarStatus,
    pub dnsmasq: SidecarStatus,
    pub mailpit: SidecarStatus,
    pub hosts_helper: SidecarStatus,
}

/// One row in `doctor`'s output. Matches the CLI's `--json` shape so the
/// two surfaces stay consistent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorFinding {
    pub check: String,
    pub verdict: DoctorVerdict,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorVerdict {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub findings: Vec<DoctorFinding>,
}

/// Input for `add_project`. Mirrors the CLI's `AddArgs` minus the
/// CLI-output flags. Optional fields are filled in from the path /
/// registry-defaults on the Rust side.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddProjectInput {
    pub path: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub hostname: Option<String>,
    #[serde(default = "default_kind")]
    pub kind: ProjectType,
    pub port: Option<u16>,
    pub start_command: Option<String>,
    #[serde(default = "default_https")]
    pub https: bool,
    #[serde(default)]
    pub auto_start: bool,
}

fn default_kind() -> ProjectType {
    ProjectType::Custom
}
fn default_https() -> bool {
    true
}

/// Output of `detect_project` — what the Add Project wizard's L1 step
/// fills the L2 fields with. Heuristics live in
/// `src-tauri/src/commands/projects.rs::detect`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedProject {
    pub kind: ProjectType,
    pub suggested_id: String,
    pub suggested_name: String,
    pub suggested_hostname: String,
    pub suggested_port: u16,
    pub suggested_start_command: Option<String>,
}

/// Input for `update_project` — partial patch over the registry entry.
/// Unset fields are left unchanged. `id` is the lookup key and isn't
/// itself mutable from this surface.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectPatch {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub extra_ports: Option<Vec<u16>>,
    pub start_command: Option<String>,
    pub https: Option<bool>,
    pub auto_start: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub services: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub document_root: Option<String>,
    pub php_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ProjectId;
    use std::path::PathBuf;

    fn sample_project() -> Project {
        Project {
            id: ProjectId::new("marketing-site"),
            name: "Marketing Site".into(),
            path: PathBuf::from("/tmp/marketing-site"),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
            extra_ports: vec![],
            hostname: "marketing-site.test".into(),
            https: true,
            services: vec!["caddy".into()],
            env: BTreeMap::new(),
            readiness: None,
            auto_start: false,
            tags: vec![],
            document_root: None,
            php_version: None,
            runtime: None,
        }
    }

    #[test]
    fn project_view_computes_https_url() {
        let v = ProjectView::from_project(&sample_project(), None);
        assert_eq!(v.url, "https://marketing-site.test");
    }

    #[test]
    fn project_view_serialises_camel_case() {
        let v = ProjectView::from_project(&sample_project(), None);
        let json = serde_json::to_value(&v).unwrap();
        assert!(json.get("startCommand").is_some());
        assert!(json.get("start_command").is_none());
        assert!(json.get("extraPorts").is_some());
    }

    #[test]
    fn project_view_defaults_status_to_stopped_when_no_runtime() {
        let v = ProjectView::from_project(&sample_project(), None);
        assert_eq!(v.status, ProjectStatus::Stopped);
        assert!(v.runtime.is_none());
    }

    #[test]
    fn add_project_input_accepts_minimal_json() {
        let json = r#"{ "path": "/tmp/x" }"#;
        let input: AddProjectInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.path, "/tmp/x");
        assert!(matches!(input.kind, ProjectType::Custom));
        assert!(input.https);
        assert!(!input.auto_start);
    }

    #[test]
    fn update_project_patch_accepts_empty_object() {
        let p: UpdateProjectPatch = serde_json::from_str("{}").unwrap();
        assert!(p.name.is_none());
        assert!(p.port.is_none());
    }

    #[test]
    fn http_only_project_uses_http_scheme() {
        let mut p = sample_project();
        p.https = false;
        let v = ProjectView::from_project(&p, None);
        assert_eq!(v.url, "http://marketing-site.test");
    }
}
