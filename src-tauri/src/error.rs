//! Structured error envelope for Tauri commands.
//!
//! Every command returns `AppResult<T>`. Failures serialise into the §5.4
//! envelope shape the frontend's error component expects:
//!
//! ```json
//! {
//!   "code": "SIDECAR_DOWN",
//!   "whatHappened": "Process Compose isn't reachable.",
//!   "whyItMatters": "Projects can't start until process-compose is running.",
//!   "whoCausedIt": "system",
//!   "actions": [{ "label": "Restart process-compose", "command": "lifecycle.restart_sidecars" }]
//! }
//! ```
//!
//! The frontend's `safeInvoke` (card #4) catches the rejected promise and
//! routes the envelope into the error component. Don't `.to_string()` here —
//! the manual `Serialize` impl preserves every field.

use serde::ser::{Serialize, SerializeStruct, Serializer};

use crate::caddy::CaddyError;
use crate::dnsmasq::DnsmasqError;
use crate::hosts::HostsError;
use crate::mailpit::MailpitError;
use crate::process_compose::PcError;
use crate::registry::RegistryError;
use crate::tunnel::TunnelError;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorAction {
    pub label: String,
    /// Frontend command id the action button invokes when clicked.
    /// `None` means the action is a passive hint (e.g. "Show details").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl ErrorAction {
    pub fn command(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: Some(command.into()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Registry(#[from] RegistryError),

    #[error("{0}")]
    Pc(#[from] PcError),

    #[error("{0}")]
    Caddy(#[from] CaddyError),

    #[error("{0}")]
    Dnsmasq(#[from] DnsmasqError),

    #[error("{0}")]
    Mailpit(#[from] MailpitError),

    #[error("{0}")]
    Tunnel(#[from] TunnelError),

    #[error("{0}")]
    Hosts(#[from] HostsError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A sidecar (process-compose, caddy, …) is not running or not reachable.
    #[error("{0} is not running")]
    SidecarDown(&'static str),

    /// A project id was referenced that isn't in the registry.
    #[error("project '{0}' not found")]
    NotFound(String),

    /// User input was malformed (bad path, bad id, missing required field).
    #[error("bad input: {0}")]
    BadInput(String),

    /// A failure that doesn't fit the other variants. Kept narrow on purpose.
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    fn code(&self) -> &'static str {
        match self {
            Self::Registry(_) => "REGISTRY_FAILURE",
            Self::Pc(_) => "PROCESS_COMPOSE_FAILURE",
            Self::Caddy(_) => "CADDY_FAILURE",
            Self::Dnsmasq(_) => "DNSMASQ_FAILURE",
            Self::Mailpit(_) => "MAILPIT_FAILURE",
            Self::Tunnel(_) => "TUNNEL_FAILURE",
            Self::Hosts(_) => "HOSTS_FAILURE",
            Self::Io(_) => "IO_FAILURE",
            Self::SidecarDown(_) => "SIDECAR_DOWN",
            Self::NotFound(_) => "PROJECT_NOT_FOUND",
            Self::BadInput(_) => "BAD_INPUT",
            Self::Internal(_) => "INTERNAL",
        }
    }

    fn why_it_matters(&self) -> String {
        match self {
            Self::SidecarDown(name) => {
                format!("Projects can't start until {name} is running again.")
            }
            Self::NotFound(_) => "Nothing was changed.".into(),
            Self::Hosts(HostsError::PermissionDenied { .. }) => {
                "Your hostnames won't resolve to localhost until /etc/hosts is updated.".into()
            }
            Self::Registry(_) => "PortBay can't read or write its project list.".into(),
            Self::Pc(_) => "The action didn't reach the daemon.".into(),
            Self::Caddy(_) => "Caddy didn't apply the change — routes may be out of sync.".into(),
            Self::Dnsmasq(_) => {
                "dnsmasq didn't start — wildcard DNS for .test won't resolve until it's running."
                    .into()
            }
            Self::Mailpit(_) => {
                "Mailpit didn't start — outgoing mail from local projects won't be caught.".into()
            }
            Self::Tunnel(_) => {
                "The Cloudflare tunnel didn't come up — the project isn't reachable from the public URL.".into()
            }
            Self::BadInput(_) => "Fix the input and try again.".into(),
            Self::Hosts(_) | Self::Io(_) | Self::Internal(_) => {
                "The action did not complete.".into()
            }
        }
    }

    /// Who the user should blame — `user` for input mistakes, `system` for
    /// anything PortBay or the OS got wrong. Drives the UI's tone.
    fn who(&self) -> &'static str {
        match self {
            Self::BadInput(_) | Self::NotFound(_) => "user",
            _ => "system",
        }
    }

    fn actions(&self) -> Vec<ErrorAction> {
        match self {
            Self::SidecarDown("process-compose") => vec![ErrorAction::command(
                "Restart process-compose",
                "sidecars.restart_pc",
            )],
            Self::SidecarDown("caddy") => vec![ErrorAction::command(
                "Restart Caddy",
                "sidecars.restart_caddy",
            )],
            Self::Hosts(HostsError::PermissionDenied { .. }) => vec![ErrorAction::command(
                "Open Terminal with sudo command",
                "system.open_sudo_hosts_hint",
            )],
            _ => vec![],
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let actions = self.actions();
        let mut st = s.serialize_struct("CommandError", 5)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("whatHappened", &self.to_string())?;
        st.serialize_field("whyItMatters", &self.why_it_matters())?;
        st.serialize_field("whoCausedIt", self.who())?;
        st.serialize_field("actions", &actions)?;
        st.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ProjectId;

    fn parse(err: &AppError) -> serde_json::Value {
        serde_json::to_value(err).expect("AppError must serialise")
    }

    #[test]
    fn envelope_has_all_five_fields() {
        let v = parse(&AppError::SidecarDown("process-compose"));
        assert!(v.get("code").is_some());
        assert!(v.get("whatHappened").is_some());
        assert!(v.get("whyItMatters").is_some());
        assert!(v.get("whoCausedIt").is_some());
        assert!(v.get("actions").is_some());
    }

    #[test]
    fn field_names_are_camel_case_not_snake_case() {
        let v = parse(&AppError::BadInput("test".into()));
        assert!(v.get("what_happened").is_none(), "must be camelCase");
        assert!(v.get("whatHappened").is_some());
    }

    #[test]
    fn sidecar_down_has_a_command_action() {
        let v = parse(&AppError::SidecarDown("process-compose"));
        let actions = v["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["label"], "Restart process-compose");
        assert_eq!(actions[0]["command"], "sidecars.restart_pc");
    }

    #[test]
    fn bad_input_is_blamed_on_user_not_system() {
        let v = parse(&AppError::BadInput("nope".into()));
        assert_eq!(v["whoCausedIt"], "user");
    }

    #[test]
    fn registry_failure_is_blamed_on_system() {
        let err = AppError::Registry(RegistryError::ProjectNotFound(ProjectId::new("x")));
        let v = parse(&err);
        assert_eq!(v["whoCausedIt"], "system");
    }

    #[test]
    fn code_distinguishes_variants() {
        assert_eq!(parse(&AppError::BadInput("".into()))["code"], "BAD_INPUT");
        assert_eq!(
            parse(&AppError::NotFound("x".into()))["code"],
            "PROJECT_NOT_FOUND"
        );
        assert_eq!(
            parse(&AppError::SidecarDown("caddy"))["code"],
            "SIDECAR_DOWN"
        );
    }

    #[test]
    fn what_happened_matches_display_impl() {
        let err = AppError::NotFound("nour-beiruti".into());
        let v = parse(&err);
        assert_eq!(v["whatHappened"], err.to_string());
        assert_eq!(v["whatHappened"], "project 'nour-beiruti' not found");
    }
}
