//! Errors surfaced by the dnsmasq adapter.

use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum DnsmasqError {
    #[error(
        "dnsmasq binary not found — install with the OS package manager or bundle as a sidecar"
    )]
    BinaryMissing,

    #[error("failed to spawn dnsmasq: {0}")]
    SpawnFailed(String),

    #[error("no free port could be found near {start}")]
    NoFreePort { start: u16 },

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, DnsmasqError>;
