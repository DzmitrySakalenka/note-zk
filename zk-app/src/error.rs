use std::{io, path::PathBuf};

use thiserror::Error;
use zk_domain::id::{NoteId, RelativePath};
use zk_index::IndexError;
use zk_sync::SyncError;
use zk_vault::error::VaultError;

use crate::config::{ConfigValidationError, GlobalConfigError, VaultConfigError};

/// Error type for `zk-app` facade operations.
#[derive(Debug, Error)]
pub enum AppError {
    /// Vault scan/parse/write failure propagated from `zk-vault`.
    #[error("while attempting to access vault data: {0}")]
    Vault(#[from] VaultError),
    /// SQLite/indexing failure propagated from `zk-index`.
    #[error("while attempting to access local index data: {0}")]
    Index(#[from] IndexError),
    /// SQLite/indexing failure with extra app-layer operation context.
    #[error("while attempting to access local index data: {context}: {source}")]
    IndexContext {
        /// Operation-specific context added by the app layer.
        context: String,
        /// Original index-layer failure.
        #[source]
        source: IndexError,
    },
    /// Global config load/parse failure.
    #[error("while attempting to load global config: {0}")]
    GlobalConfig(#[from] GlobalConfigError),
    /// Per-vault config load/parse failure.
    #[error("while attempting to load vault config: {0}")]
    VaultConfig(#[from] VaultConfigError),
    /// Effective config merge/validation failure.
    #[error("while attempting to validate effective config: {0}")]
    ConfigValidation(#[from] ConfigValidationError),
    /// Sync transport or local sync-state workflow failure.
    #[error("while attempting to perform sync workflow: {0}")]
    Sync(#[from] SyncError),
    /// Selected app-layer AI provider or mode cannot satisfy the request.
    #[error("while attempting to select AI provider: {message}")]
    ProviderUnavailable {
        /// User-facing provider availability message.
        message: String,
    },
    /// Remote network or transport failure.
    #[error("while attempting to reach remote API: {message}")]
    Network {
        /// Redacted network or transport error message.
        message: String,
    },
    /// Remote API rejected authentication or authorization.
    #[error("while attempting to authenticate remote API request: {message}")]
    Auth {
        /// Redacted API error message.
        message: String,
    },
    /// Remote API did not find the requested resource.
    #[error("while attempting to access remote API resource: {message}")]
    NotFound {
        /// Redacted API error message.
        message: String,
    },
    /// Remote API returned a structured error response.
    #[error("while attempting to call remote API: HTTP {status} {code}: {message}")]
    ApiError {
        /// Numeric HTTP status. Zero means the error came from a streamed
        /// application-level frame rather than a final HTTP status.
        status: u16,
        /// Stable API error code.
        code: String,
        /// Redacted API error message.
        message: String,
    },
    /// Retention arithmetic overflowed the representable chrono range.
    #[error(
        "while attempting to compute retention timestamp from {base}: adding {days} days overflowed representable datetime range"
    )]
    RetentionArithmeticOverflow {
        /// Base timestamp used for retention arithmetic.
        base: String,
        /// Effective clamped retention window in days.
        days: u32,
    },
    /// Status summary count overflowed the public `u32` output contract.
    #[error("while attempting to compute status summary: {field} exceeds supported u32 range")]
    StatusCountOverflow {
        /// Summary field whose count did not fit in the status output type.
        field: &'static str,
    },
    /// Generic path-scoped I/O failure.
    #[error(
        "while attempting to access path {path_display}: {source}",
        path_display = .path.display()
    )]
    Io {
        /// Path involved in the failing I/O operation.
        path: PathBuf,
        /// Source I/O error.
        #[source]
        source: io::Error,
    },
    /// A bounded collision-probing loop could not find a free destination.
    #[error("while attempting to choose a collision-free path: exhausted {attempts} attempts")]
    CollisionRetryExhausted {
        /// Number of destination candidates inspected before giving up.
        attempts: u32,
    },
    /// App-layer invariant was violated after a local lifecycle write.
    #[error("while attempting to maintain local lifecycle invariants: {context}")]
    InvariantViolation {
        /// Human-readable description of the violated local invariant.
        context: String,
    },
    /// Write blocked by immutable folder policy.
    #[error("while attempting to write item at {path}: folder is immutable")]
    ImmutableFolder {
        /// Relative path that falls under an immutable folder policy.
        path: RelativePath,
    },
    /// Requested note or asset target was not found in the local index.
    #[error("while attempting to access item {target}: item not found")]
    ItemNotFound {
        /// User-supplied target string that could not be resolved.
        target: String,
    },
    /// Requested note ID was not found in the vault.
    #[error("while attempting to update note {note_id}: note not found")]
    NoteNotFound {
        /// Note ID that could not be resolved to exactly one file.
        note_id: NoteId,
    },
    /// Update request ID did not match the provided frontmatter ID.
    #[error(
        "while attempting to update note {request_note_id}: frontmatter id {frontmatter_note_id} does not match request id"
    )]
    NoteIdMismatch {
        /// Note ID from the update request.
        request_note_id: NoteId,
        /// Note ID present in supplied frontmatter.
        frontmatter_note_id: NoteId,
    },
    /// Create request targeted an already existing vault path.
    #[error("while attempting to create note at {path}: path already exists")]
    NotePathExists {
        /// Relative path that already exists on disk.
        path: RelativePath,
    },
    /// Note frontmatter could not be serialized for index input.
    #[error("while attempting to serialize note frontmatter to JSON: {source}")]
    FrontmatterSerialization {
        /// Source JSON serialization error.
        #[source]
        source: serde_json::Error,
    },
    /// Note frontmatter could not be rendered to YAML for a vault write.
    #[error("while attempting to render note frontmatter to YAML: {source}")]
    NoteRender {
        /// Source YAML serialization error.
        #[source]
        source: serde_yaml::Error,
    },
    /// Purge API was called with the wrong request scope for that entry point.
    #[error("while attempting to purge items: expected {expected} scope")]
    InvalidPurgeScope {
        /// Stable scope shape that the called API requires.
        expected: &'static str,
    },
    /// Restore was requested for an item that is not currently trashed.
    #[error("while attempting to restore item {target}: item is not trashed")]
    ItemNotTrashed {
        /// User-supplied target string for the failed restore request.
        target: String,
    },
    /// Requested trash lifecycle item has already been purged.
    #[error("while attempting to access trashed item {target}: item is already purged")]
    AlreadyPurged {
        /// User-supplied target string for the purged item.
        target: String,
    },
    /// Restore destination already exists on disk.
    #[error("while attempting to restore item to {path}: path already exists")]
    RestoreTargetExists {
        /// Vault-relative restore destination that is already occupied.
        path: RelativePath,
    },
    /// Hard purge was requested before the retention window expired.
    #[error(
        "while attempting to purge item {target}: retention has not expired until {retention_until}"
    )]
    RetentionNotExpired {
        /// User-supplied target string for the blocked purge request.
        target: String,
        /// Timestamp when the item becomes eligible for purge.
        retention_until: String,
    },
    /// Trashed note parse could not recover a stable ID for bridge/index use.
    #[error("while attempting to bridge trashed item at {path}: missing stable id")]
    TrashedItemMissingId {
        /// Vault-relative trashed path missing a recoverable ID.
        path: RelativePath,
    },
}

/// `Result` alias used by `zk-app` public APIs.
pub type Result<T> = std::result::Result<T, AppError>;
