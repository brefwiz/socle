//! Error type for `socle`.

use thiserror::Error;

/// Result alias used throughout `socle`.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur while bootstrapping or running a service.
#[derive(Debug, Error)]
pub enum Error {
    /// A required builder method was not called or a value was invalid.
    #[error("configuration error: {0}")]
    Config(String),

    /// Telemetry initialisation failed.
    #[error("telemetry init failed: {0}")]
    Telemetry(String),

    /// Database pool construction failed.
    #[error("database init failed: {0}")]
    Database(String),

    /// Binding the TCP listener failed.
    #[error("bind failed: {0}")]
    Bind(String),

    /// The HTTP server returned an error.
    #[error("serve failed: {0}")]
    Serve(String),

    /// Outbound HTTP client construction failed.
    #[error("http client build failed: {0}")]
    HttpClient(String),
}
