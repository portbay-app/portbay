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
use crate::preferences::NotificationCategory;
use crate::process_compose::PcError;
use crate::registry::RegistryError;
use crate::ssh::backend::SshError;
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
    Ssh(#[from] SshError),

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

    /// The project's port is already bound by another process and
    /// PortBay didn't recognise it as something it could clean up
    /// (i.e. it's an external app — ServBay, MAMP, a manually-started
    /// dev server). Caller fills in the holder description.
    #[error("port {port} is already in use by {holder}")]
    PortConflict { port: u16, holder: String },

    /// User input was malformed (bad path, bad id, missing required field).
    #[error("bad input: {0}")]
    BadInput(String),

    /// Adding another project would exceed the current tier's cap. The GUI
    /// catches this code to open the sign-in / upgrade sheet; the CLI prints
    /// the next step. Carries the cap that was hit.
    #[error("You've reached your {cap}-project limit")]
    ProjectCapReached { cap: u32 },

    /// Enabling Sandboxed Run on another project would exceed the current
    /// tier's community allowance (anonymous/free get a small cap; Pro is
    /// uncapped, so it never sees this). Mirrors [`Self::ProjectCapReached`].
    /// Re-running an already-sandboxed project never trips it.
    #[error("You've reached your {cap}-project Sandboxed Run limit")]
    SandboxCapReached { cap: u32 },

    /// A Pro-gated configuration was set or changed by a non-Pro session. The
    /// GUI locks these controls proactively; this is the core-side safety net
    /// for the CLI and hand-edited registries. Carries a human feature label.
    /// An *existing* configured value is never stripped — only the act of
    /// introducing/changing one without Pro is rejected.
    #[error("{feature} is a PortBay Pro feature")]
    ProRequired { feature: &'static str },

    /// A feature was invoked on a platform that can't provide it — e.g. the
    /// macOS-only Seatbelt sandbox on Linux/Windows. Unlike [`Self::ProRequired`],
    /// no tier unlocks it on this OS.
    #[error("{reason}")]
    Unsupported {
        feature: &'static str,
        reason: &'static str,
    },

    /// This Pro license is already active on its device cap (2 devices). The GUI
    /// catches this code to point the user at Settings → Sync to deactivate a
    /// device before adding this one. The server is authoritative for the count.
    #[error("This Pro license is active on its limit of {max} devices")]
    DeviceLimitReached { max: u32 },

    /// A failure that doesn't fit the other variants. Kept narrow on purpose.
    #[error("{0}")]
    Internal(String),

    /// The per-project task board / agent-context layer (`crate::context`)
    /// failed — parsing a card's frontmatter, an atomic write, a dispatch, etc.
    /// Board-only; absent from the public OSS build (no `tasks` feature).
    #[cfg(feature = "tasks")]
    #[error("{0}")]
    Context(#[from] crate::context::ContextError),
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
            Self::Ssh(SshError::NeedsKeyPassphrase { .. }) => "SSH_NEEDS_PASSPHRASE",
            Self::Ssh(SshError::MissingPassword { .. }) => "SSH_NEEDS_PASSWORD",
            Self::Ssh(_) => "SSH_TUNNEL_FAILURE",
            Self::Hosts(_) => "HOSTS_FAILURE",
            Self::Io(_) => "IO_FAILURE",
            Self::SidecarDown(_) => "SIDECAR_DOWN",
            Self::NotFound(_) => "PROJECT_NOT_FOUND",
            Self::PortConflict { .. } => "PORT_CONFLICT",
            Self::BadInput(_) => "BAD_INPUT",
            Self::ProjectCapReached { .. } => "PROJECT_CAP_REACHED",
            Self::SandboxCapReached { .. } => "SANDBOX_CAP_REACHED",
            Self::ProRequired { .. } => "PRO_REQUIRED",
            Self::Unsupported { .. } => "UNSUPPORTED",
            Self::DeviceLimitReached { .. } => "DEVICE_LIMIT_REACHED",
            Self::Internal(_) => "INTERNAL",
            #[cfg(feature = "tasks")]
            Self::Context(_) => "CONTEXT_FAILURE",
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
            // Credential gaps are normally intercepted by the inline prompt; if
            // one surfaces, guide rather than talk about forwarding.
            Self::Ssh(SshError::MissingPassword { .. }) => {
                "Enter the host password to connect.".into()
            }
            Self::Ssh(SshError::NeedsKeyPassphrase { .. }) => {
                "Enter the SSH key's passphrase to connect.".into()
            }
            // Genuine port-forward failures: localhost reachability is the point.
            Self::Ssh(
                SshError::ReadinessTimeout(_)
                | SshError::ExitedEarly
                | SshError::PasswordForwardUnsupported,
            ) => "The remote service wasn't forwarded to localhost, so local tools cannot reach it yet.".into(),
            // Everything else (auth rejected, transport/connect failure, …) is a
            // plain connection problem — not necessarily a tunnel.
            Self::Ssh(_) => "PortBay couldn't establish the SSH connection.".into(),
            Self::PortConflict { holder, .. } => {
                format!("Stop {holder} (or change this project's port in its detail panel) and try again.")
            }
            Self::BadInput(_) => "Fix the input and try again.".into(),
            Self::ProjectCapReached { .. } => {
                "Sign in or create a free account to add more, or upgrade to Pro for unlimited projects.".into()
            }
            Self::SandboxCapReached { .. } => {
                "Upgrade to PortBay Pro to sandbox more projects — your existing sandboxed projects are unchanged.".into()
            }
            Self::ProRequired { .. } => {
                "Upgrade to PortBay Pro to use this — your existing settings are unchanged.".into()
            }
            Self::Unsupported { .. } => {
                "This feature isn't available on your operating system.".into()
            }
            Self::DeviceLimitReached { .. } => {
                "Deactivate another device under Settings → Sync to use Pro on this one.".into()
            }
            #[cfg(feature = "tasks")]
            Self::Context(_) => {
                "The project's task board or agent-context files weren't updated.".into()
            }
            Self::Hosts(_) | Self::Io(_) | Self::Internal(_) => {
                "The action did not complete.".into()
            }
        }
    }

    /// Who the user should blame — `user` for input mistakes, `system` for
    /// anything PortBay or the OS got wrong. Drives the UI's tone.
    fn who(&self) -> &'static str {
        match self {
            Self::BadInput(_)
            | Self::NotFound(_)
            | Self::PortConflict { .. }
            | Self::ProjectCapReached { .. }
            | Self::SandboxCapReached { .. }
            | Self::ProRequired { .. }
            | Self::DeviceLimitReached { .. } => "user",
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

    fn category(&self) -> NotificationCategory {
        match self {
            Self::Dnsmasq(_) | Self::Hosts(_) => NotificationCategory::Infrastructure,
            Self::Tunnel(_) | Self::Ssh(_) => NotificationCategory::Infrastructure,
            Self::DeviceLimitReached { .. } => NotificationCategory::AccountSync,
            Self::Io(_) | Self::Internal(_) => NotificationCategory::Crash,
            #[cfg(feature = "tasks")]
            Self::Context(_) => NotificationCategory::AgentBoard,
            _ => NotificationCategory::ProjectError,
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let actions = self.actions();
        let mut st = s.serialize_struct("CommandError", 6)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("whatHappened", &self.to_string())?;
        st.serialize_field("whyItMatters", &self.why_it_matters())?;
        st.serialize_field("whoCausedIt", self.who())?;
        st.serialize_field("category", &self.category())?;
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
        assert!(v.get("category").is_some());
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
        assert_eq!(v["category"], "project-error");
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
        let err = AppError::NotFound("marketing-site".into());
        let v = parse(&err);
        assert_eq!(v["whatHappened"], err.to_string());
        assert_eq!(v["whatHappened"], "project 'marketing-site' not found");
    }
}
