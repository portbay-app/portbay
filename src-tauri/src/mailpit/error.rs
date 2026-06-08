//! Errors surfaced by the Mailpit adapter.

use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum MailpitError {
    #[error(
        "mailpit binary not found — install with the OS package manager or bundle as a sidecar"
    )]
    BinaryMissing,

    #[error("failed to spawn mailpit: {0}")]
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

pub type Result<T> = std::result::Result<T, MailpitError>;
