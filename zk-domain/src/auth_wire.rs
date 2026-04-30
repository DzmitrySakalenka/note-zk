//! Browser-auth wire DTOs shared by clients and server handlers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::AuthScope;

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

/// Password-user creation request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateUserRequest {
    /// Login username for the new user.
    pub username: String,
    /// Plaintext password supplied by the caller.
    pub password: String,
}

/// Password-user creation response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateUserResponse {
    /// Stable user identifier.
    pub user_id: String,
    /// Login username for the created user.
    pub username: String,
    /// Whether this request used unauthenticated bootstrap mode.
    pub bootstrap: bool,
}

/// Device-token issue request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueTokenRequest {
    /// Human-readable operator label.
    pub label: String,
    /// Optional owning sync device identifier.
    pub device_id: Option<String>,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
}

/// Device-token rotation request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RotateTokenRequest {
    /// Human-readable operator label for the replacement token.
    pub label: String,
    /// Optional owning sync device identifier for the replacement token.
    pub device_id: Option<String>,
    /// Authorization scopes granted to the replacement token.
    pub scopes: Vec<AuthScope>,
}

/// Device-token issue response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueTokenResponse {
    /// Stable device-token identifier.
    pub token_id: String,
    /// One-time plaintext bearer token.
    pub plaintext: String,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
    /// Token creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Device-token list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedToken {
    /// Stable device-token identifier.
    pub token_id: String,
    /// Human-readable operator label.
    pub label: String,
    /// Optional owning sync device identifier.
    pub device_id: Option<String>,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
    /// Token creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent successful use timestamp, if any.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Device-token list response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListTokensResponse {
    /// Active device tokens.
    pub tokens: Vec<ListedToken>,
}
