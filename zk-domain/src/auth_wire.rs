//! Browser-auth wire DTOs shared by clients and server handlers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Password login request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    /// Login username.
    pub username: String,
    /// Plaintext password supplied by the caller.
    pub password: String,
}

/// Password login response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginResponse {
    /// Plaintext CSRF token returned once for browser double-submit requests.
    pub csrf_token: String,
    /// Session expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

/// Browser session refresh response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshResponse {
    /// Plaintext CSRF token for the rotated browser session.
    pub csrf_token: String,
    /// New session expiry timestamp.
    pub expires_at: DateTime<Utc>,
}
