//! Sandboxed project runner support.
//!
//! Logs, readiness, stop/restart, and Caddy routing remain the same because PC
//! still owns the supervised process.

use std::fs;
use std::path::{Path, PathBuf};

use crate::registry::{Project, SandboxConfig, SandboxNetworkPolicy};

/// Legacy marker used by the first sandbox build. Kept as read compatibility
/// so existing registries don't silently run unrestricted after upgrade.
pub const SANDBOX_TAG: &str = "portbay:sandbox";

pub fn is_enabled(project: &Project) -> bool {
    project
        .sandbox
        .as_ref()
        .map(|cfg| cfg.enabled)
        .unwrap_or_else(|| project.tags.iter().any(|tag| tag == SANDBOX_TAG))
}

pub fn config(project: &Project) -> SandboxConfig {
    project.sandbox.clone().unwrap_or_else(|| SandboxConfig {
        enabled: project.tags.iter().any(|tag| tag == SANDBOX_TAG),
        ..SandboxConfig::default()
    })
}

pub fn enable(project: &mut Project, network: SandboxNetworkPolicy, ephemeral: bool) {
    project.sandbox = Some(SandboxConfig::enabled(network, ephemeral));
    project.tags.retain(|tag| tag != SANDBOX_TAG);
}

pub fn disable(project: &mut Project) {
    project.tags.retain(|tag| tag != SANDBOX_TAG);
    if let Some(cfg) = &mut project.sandbox {
        cfg.enabled = false;
    }
}

pub fn network_policy_key(policy: SandboxNetworkPolicy) -> &'static str {
    match policy {
        SandboxNetworkPolicy::LoopbackOnly => "loopback_only",
        SandboxNetworkPolicy::Outbound => "outbound",
        SandboxNetworkPolicy::Full => "full",
        SandboxNetworkPolicy::Blocked => "blocked",
    }
}

pub fn profile_path(data_dir: &Path, project: &Project) -> PathBuf {
    data_dir
        .join("sandbox")
        .join(format!("{}.sb", project.id.as_str()))
}

pub fn ensure_profile(data_dir: &Path, project: &Project) -> std::io::Result<PathBuf> {
    let path = profile_path(data_dir, project);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, profile(project))?;
    Ok(path)
}

pub fn wrap_command(data_dir: &Path, project: &Project, command: &str) -> String {
    match ensure_profile(data_dir, project) {
        Ok(profile) => format!(
            "sandbox-exec -f {} /bin/zsh -lc {}",
            shell_quote(&profile.to_string_lossy()),
            shell_quote(command)
        ),
        Err(err) => format!(
            "printf %s {} >&2; exit 126",
            shell_quote(&format!("PortBay sandbox profile error: {err}\n"))
        ),
    }
}

pub fn reset_ephemeral_state(data_dir: &Path, project: &Project) -> std::io::Result<()> {
    let cfg = config(project);
    if !cfg.enabled || !cfg.ephemeral {
        return Ok(());
    }
    let root = data_dir
        .join("sandbox")
        .join(project.id.as_str())
        .join("ephemeral");
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)
}

pub fn violation_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("deny(")
                || lower.contains("sandbox")
                    && (lower.contains("deny") || lower.contains("operation not permitted"))
        })
        .cloned()
        .collect()
}

fn profile(project: &Project) -> String {
    let project_path = project.path.to_string_lossy();
    let cfg = config(project);
    let network = match cfg.network {
        SandboxNetworkPolicy::Blocked => "",
        SandboxNetworkPolicy::LoopbackOnly => {
            r#"; Loopback-only networking: local dev-server bind/connect.
(allow network* (local ip "localhost:*"))
(allow network* (remote ip "localhost:*"))
"#
        }
        SandboxNetworkPolicy::Outbound => {
            r#"; Outbound package-manager access plus local dev-server bind.
(allow network-outbound)
(allow network* (local ip "localhost:*"))
"#
        }
        SandboxNetworkPolicy::Full => "(allow network*)\n",
    };
    format!(
        r#"(version 1)
(deny default)

; Launch shells, package managers, interpreters, and their children.
(allow process*)
(allow signal (target same-sandbox))
(allow sysctl-read)
(allow mach-lookup)

; Runtimes need to read toolchains, frameworks, lockfiles, and package caches.
(allow file-read*)

; Writes are constrained to the project plus OS temp/cache locations commonly
; used by package managers and dev servers.
(allow file-write*
  (subpath {project_path_q})
  (subpath "/tmp")
  (subpath "/private/tmp")
  (literal "/dev/null")
  (regex #"^/private/var/folders/"))

{network}
"#,
        project_path_q = scheme_string(&project_path),
        network = network,
    )
}

fn scheme_string(value: &str) -> String {
    format!("{value:?}")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ProjectId, ProjectType};
    use std::collections::BTreeMap;

    fn project() -> Project {
        Project {
            id: ProjectId::new("demo"),
            name: "Demo".into(),
            path: PathBuf::from("/tmp/demo"),
            kind: ProjectType::Node,
            start_command: Some("pnpm dev".into()),
            port: Some(3000),
            extra_ports: vec![],
            hostname: "demo.test".into(),
            https: false,
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
            cors: None,
            sandbox: None,
        }
    }

    #[test]
    fn config_helpers_are_idempotent() {
        let mut p = project();
        enable(&mut p, SandboxNetworkPolicy::Outbound, false);
        enable(&mut p, SandboxNetworkPolicy::Outbound, false);
        assert!(is_enabled(&p));
        assert_eq!(config(&p).network, SandboxNetworkPolicy::Outbound);
        disable(&mut p);
        assert!(!is_enabled(&p));
    }

    #[test]
    fn network_policy_key_matches_api_wire_values() {
        assert_eq!(
            network_policy_key(SandboxNetworkPolicy::LoopbackOnly),
            "loopback_only"
        );
        assert_eq!(
            network_policy_key(SandboxNetworkPolicy::Outbound),
            "outbound"
        );
        assert_eq!(network_policy_key(SandboxNetworkPolicy::Full), "full");
        assert_eq!(network_policy_key(SandboxNetworkPolicy::Blocked), "blocked");
    }

    #[test]
    fn wrapper_uses_profile_and_original_command() {
        let p = project();
        let cmd = wrap_command(Path::new("/tmp/portbay"), &p, "pnpm dev");
        assert!(cmd.contains("sandbox-exec -f"));
        assert!(cmd.contains("/bin/zsh -lc 'pnpm dev'"));
    }
}
