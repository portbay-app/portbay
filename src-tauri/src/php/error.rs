//! Errors surfaced by the PHP detection + lifecycle module.

#[derive(thiserror::Error, Debug)]
pub enum PhpError {
    #[error("PHP version {0} is not installed — run `brew install php@{0}`")]
    VersionNotInstalled(String),

    #[error("php-fpm not found for PHP {0} — install the FPM variant of the Homebrew formula")]
    FpmMissing(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, PhpError>;
