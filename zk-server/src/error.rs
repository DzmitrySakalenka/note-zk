//! Shared error types returned by server bootstrap and HTTP workflows.

use thiserror::Error;

/// Result alias used by `zk-server` public APIs.
pub type ServerResult<T> = Result<T, ServerError>;

/// Error type for server configuration, storage, and request handling.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Server configuration failed to load or validate.
    #[error("config error")]
    Config(#[source] ConfigError),
    /// `SQLite` returned an error while serving a request or bootstrapping.
    #[error("database error")]
    Database(#[source] rusqlite::Error),
    /// Schema migration failed.
    #[error("migration error: {0}")]
    Migration(String),
    /// Client supplied an invalid request payload or parameter.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// Requested resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// Requested mutation conflicts with current server state.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Request lacks valid authentication credentials.
    #[error("unauthorized")]
    Unauthorized,
    /// Internal error wrapper for opaque server failures.
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Placeholder configuration error until TG12-002 defines the real config model.
#[derive(Debug, Error)]
#[error("config error")]
pub struct ConfigError;
