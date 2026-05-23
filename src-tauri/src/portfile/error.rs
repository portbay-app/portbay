//! Errors surfaced by the `.portbay.json` import/export path.

#[derive(thiserror::Error, Debug)]
pub enum PortfileError {
    #[error("could not serialise project descriptor: {0}")]
    Serialise(#[source] serde_json::Error),

    #[error("could not parse `.portbay.json`: {0}")]
    Parse(#[source] serde_json::Error),

    #[error(
        "this `.portbay.json` was written by a newer PortBay (schema v{found}); \
         this build understands up to v{supported}. Update PortBay to import it."
    )]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("the file lists secret `{0}` but no value was provided on import")]
    SecretMissing(String),
}

pub type Result<T> = std::result::Result<T, PortfileError>;
