//! Error types returned by sync planning and apply workflows.

use thiserror::Error;
use zk_index::IndexError;
use zk_vault::error::VaultError;

/// Result alias used by `zk-sync` public APIs.
pub type SyncResult<T> = Result<T, SyncError>;

/// Error type for sync planning, transport, and local apply operations.
#[derive(Debug, Error)]
pub enum SyncError {
    /// Local sync state does not contain the requested item.
    #[error("item {0} missing from local state")]
    ItemMissing(String),
    /// The caller's base revision no longer matches the server revision.
    #[error("base revision {base:?} does not match server revision {server:?}")]
    RevisionMismatch {
        /// Revision the client believed it was updating from.
        base: Option<i64>,
        /// Current server revision returned by the remote side.
        server: i64,
    },
    /// A pulled event sequence number is not strictly greater than the last one seen.
    #[error("unexpected event sequence: expected > {last}, got {next}")]
    OutOfOrderEvent {
        /// Last successfully applied event sequence number.
        last: i64,
        /// Next event sequence number presented by the remote side.
        next: i64,
    },
    /// Initial sync cannot proceed automatically because both sides already contain data.
    #[error("initial sync is ambiguous: both local and remote have content")]
    AmbiguousBootstrap,
    /// A bounded collision-probing loop could not find a free destination.
    #[error("collision retry exhausted after {attempts} attempts")]
    CollisionRetryExhausted {
        /// Number of destination candidates inspected before giving up.
        attempts: u32,
    },
    /// The selected transport does not support this operation.
    #[error("unsupported sync operation: {0}")]
    Unsupported(&'static str),
    /// Remote payload violated the sync protocol after decoding.
    #[error("protocol failure: {0}")]
    Protocol(String),
    /// Writing bytes or metadata to the local vault failed.
    #[error("vault write failed")]
    VaultWrite(#[from] VaultError),
    /// Reading or mutating the local sync index failed.
    #[error("index operation failed")]
    Index(#[from] IndexError),
    /// Remote transport I/O or protocol machinery failed.
    #[error("transport failure")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
}
