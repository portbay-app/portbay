//! Cross-process sidecar status probe.
//!
//! Shared by the CLI `sidecar status` command and the MCP
//! `portbay_sidecar_status` tool so the two surfaces can't drift. It reports
//! only what's *honestly* observable from outside the app process:
//!
//! - **process-compose** — its HTTP admin API answers liveness directly.
//! - **dnsmasq** — the `/etc/resolver/<suffix>` file is the cross-process
//!   signal (the wildcard routing + port). The daemon's *liveness* is owned by
//!   the app, so we report the routing intent, not a live up/down.
//! - **/etc/hosts** — the managed-entry count is readable from disk.
//!
//! Caddy (dynamic admin port), the mkcert CA, and Mailpit are owned by the app
//! in-memory and can't be confirmed from here — reported `unknown` with an
//! honest pointer rather than a guess. Restarting any sidecar is app-only (the
//! app owns the child processes), so this module is status-only.

use crate::dnsmasq::resolver;
use crate::hosts::HostsManager;
use crate::process_compose::PcClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeState {
    /// Confirmed up from outside the app.
    Running,
    /// Confirmed down / not reachable.
    Stopped,
    /// Owned by the app; live state can't be seen from here.
    Unknown,
}

impl ProbeState {
    pub fn as_str(self) -> &'static str {
        match self {
            ProbeState::Running => "running",
            ProbeState::Stopped => "stopped",
            ProbeState::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SidecarProbe {
    pub name: &'static str,
    pub state: ProbeState,
    pub detail: String,
}

/// Probe every sidecar reachable from outside the app. `pc_port` is the
/// process-compose admin port; `suffix` is the active domain suffix (for the
/// resolver file). Async because the process-compose check is an HTTP request.
pub async fn probe(pc_port: u16, suffix: &str) -> Vec<SidecarProbe> {
    let mut out = Vec::with_capacity(6);

    // process-compose — HTTP liveness, the one sidecar we can confirm directly.
    let pc_live = PcClient::new(pc_port).live().await.unwrap_or(false);
    out.push(SidecarProbe {
        name: "process-compose",
        state: if pc_live {
            ProbeState::Running
        } else {
            ProbeState::Stopped
        },
        detail: if pc_live {
            format!("reachable on :{pc_port}")
        } else {
            "not reachable — open PortBay.app".into()
        },
    });

    // dnsmasq — the resolver file tells us the wildcard routing + port; the
    // daemon's liveness is the app's to know.
    let contents = resolver::read_installed(suffix);
    let installed = contents
        .as_deref()
        .is_some_and(|c| c.contains("nameserver 127.0.0.1"));
    let port = contents.as_deref().and_then(parse_resolver_port);
    out.push(SidecarProbe {
        name: "dnsmasq",
        state: ProbeState::Unknown,
        detail: match (installed, port) {
            (true, Some(p)) => format!("resolver routes *.{suffix} → 127.0.0.1:{p} (liveness in-app)"),
            (true, None) => format!("resolver installed for .{suffix} (liveness in-app)"),
            (false, _) => "no resolver file — names resolve via /etc/hosts".into(),
        },
    });

    // /etc/hosts — the managed-entry count is readable from disk.
    match HostsManager::system().list_managed() {
        Ok(entries) => out.push(SidecarProbe {
            name: "hosts",
            state: ProbeState::Running,
            detail: format!("{} managed entries", entries.len()),
        }),
        Err(e) => out.push(SidecarProbe {
            name: "hosts",
            state: ProbeState::Unknown,
            detail: e.to_string(),
        }),
    }

    // App-owned sidecars we can't confirm from outside the daemon. (Caddy's
    // admin port is allocated dynamically, so we can't even probe it reliably.)
    for tool in ["caddy", "mkcert", "mailpit"] {
        out.push(SidecarProbe {
            name: tool,
            state: ProbeState::Unknown,
            detail: "managed by PortBay.app — live state not visible from here".into(),
        });
    }

    out
}

/// Pull the `port <n>` line out of an `/etc/resolver/<suffix>` file body.
fn parse_resolver_port(contents: &str) -> Option<u16> {
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("port ")
            .and_then(|n| n.trim().parse().ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resolver_port_from_body() {
        assert_eq!(
            parse_resolver_port("nameserver 127.0.0.1\nport 53053\n"),
            Some(53053)
        );
        assert_eq!(parse_resolver_port("nameserver 127.0.0.1\n"), None);
    }

    #[tokio::test]
    async fn probe_reports_every_sidecar_with_pc_first() {
        // No daemon on this port → process-compose reads stopped, but the probe
        // still returns one honest row per sidecar (never panics / never empty).
        let probes = probe(1, "portbay.test").await;
        assert_eq!(probes[0].name, "process-compose");
        assert_eq!(probes[0].state, ProbeState::Stopped);
        let names: Vec<&str> = probes.iter().map(|p| p.name).collect();
        for expected in ["process-compose", "dnsmasq", "hosts", "caddy", "mkcert", "mailpit"] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }
}
