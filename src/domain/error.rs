//! Domain error type shared across the crate.
//!
//! A single [`Error`] enum models every expected failure (bad input, missing
//! entities, config problems, I/O). Library code propagates it via [`Result`];
//! the binary edge adds human context with `anyhow`.

/// All errors the domain, storage and service layers can produce.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An operation was requested with invalid input or in a disallowed state.
    #[error("{0}")]
    Invalid(String),

    /// A repository could not be found by index, slug or path.
    #[error("repository not found: {0}")]
    NotFound(String),

    /// A slug was rejected (empty, reserved, malformed or already in use).
    #[error("invalid slug: {0}")]
    Slug(String),

    /// A configuration file could not be read or parsed.
    #[error("error in config {path}: {message}")]
    Config {
        /// Config file path.
        path: String,
        /// Reader or parser message.
        message: String,
    },

    /// An underlying I/O failure with the context that caused it.
    #[error("{context}: {source}")]
    Io {
        /// What was attempted (e.g. "write config.toml").
        context: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

impl Error {
    /// Builds an [`Error::Invalid`] from a message.
    pub fn invalid(message: impl Into<String>) -> Self {
        Error::Invalid(message.into())
    }

    /// Builds an [`Error::Config`] from a path and message.
    pub fn config(path: impl Into<String>, message: impl Into<String>) -> Self {
        Error::Config {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Builds an [`Error::Io`] tagging the underlying error with `context`.
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }
}

/// Crate-wide result alias over [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
