use std::path::PathBuf;

use crate::registry::types::{
    DatabaseInstanceId, ProjectId, SshConnectionId, SshIdentityId, SshTunnelId,
};

/// Errors surfaced by the registry layer.
///
/// These are intentionally specific so the GUI and CLI can map them to the
/// structured error envelope described in `claudedocs/ASSESSMENT_AND_PLAN.md`
/// §5.4 (what happened / why it matters / what to do).
#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("registry file not found at {path}")]
    NotFound { path: PathBuf },

    #[error("registry file is malformed: {0}")]
    Malformed(#[from] serde_json::Error),

    #[error("registry version {found} is unsupported (this build supports up to v{supported})")]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("registry migration from v{from} failed: {reason}")]
    Migration { from: u32, reason: String },

    #[error("project id `{0}` not found")]
    ProjectNotFound(ProjectId),

    #[error("project id `{0}` already exists")]
    DuplicateProjectId(ProjectId),

    #[error("hostname `{0}` is already used by another project")]
    DuplicateHostname(String),

    #[error("port {0} is already used by another project")]
    DuplicatePort(u16),

    #[error("group id `{0}` not found")]
    GroupNotFound(String),

    #[error("group id `{0}` already exists")]
    DuplicateGroupId(String),

    #[error("database instance `{0}` not found")]
    DatabaseNotFound(DatabaseInstanceId),

    #[error("database instance `{0}` already exists")]
    DuplicateDatabaseId(DatabaseInstanceId),

    #[error("SSH tunnel `{0}` not found")]
    SshTunnelNotFound(SshTunnelId),

    #[error("SSH tunnel `{0}` already exists")]
    DuplicateSshTunnelId(SshTunnelId),

    #[error("SSH connection `{0}` not found")]
    SshConnectionNotFound(SshConnectionId),

    #[error("SSH connection `{0}` already exists")]
    DuplicateSshConnectionId(SshConnectionId),

    #[error("SSH identity `{0}` not found")]
    SshIdentityNotFound(SshIdentityId),

    #[error("SSH identity `{0}` already exists")]
    DuplicateSshIdentityId(SshIdentityId),

    #[error("no data directory available on this OS — cannot resolve the default registry path")]
    NoDataDir,

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl RegistryError {
    /// Wrap an `io::Error` together with the path it concerned, so error
    /// messages tell the user exactly which file failed.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, RegistryError>;
