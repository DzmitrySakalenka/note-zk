#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
#![doc = "Backup primitives shared across the workspace. Owns archive build, encryption, manifest serialization, retention, and restore primitives consumed by `zk-server` and `zk-cli`."]

pub mod archive;
pub mod encryption;
pub mod error;
pub mod manifest;
pub mod restore;
pub mod retention;
pub mod schedule;
pub mod service;

pub use encryption::{
    AgeIdentityDecryptor, AgeRecipientEncryptor, BackupDecryptor, BackupEncryptor,
    PassthroughEncryptor,
};
pub use error::{BackupError, BackupResult};
pub use manifest::{
    BackupArchiveDescriptor, BackupManifest, BackupManifestEncryption, BackupSourceSummary,
    MANIFEST_SCHEMA_VERSION,
};
pub use restore::{RestoreService, RestoreSpec, RestoreSummary};
pub use retention::{RetentionPolicy, RetentionPruneSummary};
pub use schedule::CronSchedule;
pub use service::{BackupService, BackupServiceConfig};

// TG15-001 fixes the crate's agreed dependency surface before the primitive
// modules consume it in TG15-003 through TG15-009.
use chrono as _;
use data_encoding as _;
use secrecy as _;
use serde as _;
use serde_json as _;
use sha2 as _;
use tracing as _;
