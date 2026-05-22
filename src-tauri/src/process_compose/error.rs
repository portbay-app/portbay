use std::path::PathBuf;

/// Errors surfaced by the Process Compose adapter.
///
/// Maps directly to the error envelope in `ASSESSMENT_AND_PLAN.md` §5.4
/// — every variant has a clear "what happened" message so the GUI / CLI
/// can render it without further interpretation.
#[derive(thiserror::Error, Debug)]
pub enum PcError {
    #[error("could not reach the Process Compose daemon at {url}: {source}")]
    Unreachable {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Process Compose returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },

    #[error("Process Compose returned a body we couldn't parse: {0}")]
    BodyDecode(#[source] reqwest::Error),

    #[error("process `{name}` was not found in the running project")]
    ProcessNotFound { name: String },

    #[error("failed to spawn process-compose sidecar: {0}")]
    SpawnFailed(String),

    #[error("sidecar exited unexpectedly: {0}")]
    SidecarExited(String),

    #[error("no free port could be found near {start}")]
    NoFreePort { start: u16 },

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("YAML serialisation failed: {0}")]
    YamlSerialize(#[from] serde_yaml::Error),
}

impl PcError {
    /// Constructor used by file I/O sites that need to attach the offending
    /// path to the error. Reintroduced as soon as the CLI starts writing
    /// PC YAML files to disk.
    #[allow(dead_code)]
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, PcError>;
