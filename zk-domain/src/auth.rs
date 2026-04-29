//! Authentication record types shared by repositories and services.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::id::DeviceId;

/// Password-authenticated user record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordUser {
    /// Stable user identifier.
    pub user_id: String,
    /// Login username.
    pub username: String,
    /// Argon2id PHC password hash.
    pub password_hash: String,
    /// User creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the user was disabled, if disabled.
    pub disabled_at: Option<DateTime<Utc>>,
}

/// Stored device-token record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTokenRecord {
    /// Stable device-token identifier.
    pub token_id: String,
    /// Optional owning sync device identifier.
    pub device_id: Option<DeviceId>,
    /// Operator-visible token label.
    pub label: String,
    /// Lowercase `SHA-256(plaintext_token)` hex digest.
    pub token_sha256_hex: String,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
    /// Token creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent successful use timestamp, if any.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Revocation timestamp, if revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Stored web-session record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSessionRecord {
    /// Lowercase `SHA-256(plaintext_session_id)` hex digest.
    pub session_id_sha256_hex: String,
    /// Owning user identifier.
    pub user_id: String,
    /// Lowercase `SHA-256(plaintext_csrf_token)` hex digest.
    pub csrf_token_sha256_hex: String,
    /// Authorization scopes granted to the session.
    pub scopes: Vec<AuthScope>,
    /// Session creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Session expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Most recent successful use timestamp.
    pub last_seen_at: DateTime<Utc>,
    /// Revocation timestamp, if revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Authorization scope granted to a session or device token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthScope {
    /// Full sync API access.
    Sync,
    /// Read-only access.
    Read,
    /// Administrative access.
    Admin,
}

impl AuthScope {
    /// Returns the stable lowercase scope string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Read => "read",
            Self::Admin => "admin",
        }
    }
}
