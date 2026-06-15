//! SSH remote-forwarding support.
//!
//! Cloudflare tunnels expose local projects outward; this module does the
//! inverse by supervising system `ssh` processes that pull remote services onto
//! localhost. The manager owns only child processes and live status. Persistent
//! profiles live in the registry.

pub mod agent;
pub mod backend;
pub mod config_import;
pub mod exec;
pub mod exec_manager;
pub mod interaction;
pub mod known_hosts;
pub mod manager;
pub mod probe;
pub mod proxy;
pub mod pty;
pub mod secret;
pub mod session;
pub mod sftp;

pub use agent::AgentManager;
pub use backend::{
    build_tunnel_key, equivalent_ssh_command, resolve_tunnels, should_use_system_ssh,
    EffectiveSshTunnel,
};
pub use config_import::{parse_ssh_config, SshConfigCandidate};
pub use exec_manager::ExecManager;
pub use interaction::{EventInteractor, NoopInteractor, SshInteractor};
pub use manager::{
    read_state, state_file_path, write_state, SshManager, SshTunnelEventStatus,
    SshTunnelRuntimeStatus, SSH_STATE_CHANNEL,
};
pub use probe::{probe_connection, ProbeResult};
pub use pty::PtyManager;
pub use secret::{secret_str, SecretString};
pub use session::{connect_session, SshSession, SshSessionHandle};
pub use sftp::SftpManager;
