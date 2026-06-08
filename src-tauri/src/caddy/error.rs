use std::path::PathBuf;

/// Errors surfaced by the Caddy adapter.
#[derive(thiserror::Error, Debug)]
pub enum CaddyError {
    #[error("could not reach Caddy admin API at {url}: {source}")]
    Unreachable {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Caddy returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },

    #[error("Caddy returned a body we couldn't parse: {0}")]
    BodyDecode(#[source] reqwest::Error),

    #[error("failed to spawn Caddy sidecar: {0}")]
    SpawnFailed(String),

    #[error("no free port could be found near {start}")]
    NoFreePort { start: u16 },

    #[error("route id `{0}` not found in Caddy's running config")]
    RouteNotFound(String),

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON serialisation failed: {0}")]
    JsonSerialize(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CaddyError>;
