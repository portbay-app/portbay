//! Errors surfaced by the Cloudflare Tunnel adapter.

#[derive(thiserror::Error, Debug)]
pub enum TunnelError {
    #[error(
        "cloudflared binary not found — bundle a sidecar or install with the OS package manager"
    )]
    BinaryMissing,

    #[error("failed to spawn cloudflared: {0}")]
    SpawnFailed(String),

    #[error("tunnel for project `{0}` is already running")]
    AlreadyRunning(String),

    #[error("no tunnel running for project `{0}`")]
    NotRunning(String),

    #[error("cloudflared did not announce a public URL within the timeout")]
    UrlTimeout,
}

pub type Result<T> = std::result::Result<T, TunnelError>;
