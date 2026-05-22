//! Errors surfaced by the migration-import modules.

use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum ImportError {
    #[error("source tool not installed at {0}")]
    SourceMissing(PathBuf),

    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("config at {path} is malformed: {detail}")]
    Malformed { path: PathBuf, detail: String },
}

impl ImportError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn malformed(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Self::Malformed {
            path: path.into(),
            detail: detail.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, ImportError>;
