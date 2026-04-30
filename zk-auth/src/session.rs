//! Web-session id issuance and validation primitives.

use chrono::{DateTime, Duration, Utc};

use crate::{AuthError, AuthResult, secret};

/// Fresh web-session material returned at session creation time.
///
/// `plaintext_id` is a bearer secret and should only be serialized into the
/// secure session cookie. Persist `sha256_hex`, never the plaintext id.
pub struct IssuedSession {
    /// Plaintext session id for the browser cookie.
    pub plaintext_id: String,
    /// Lowercase `SHA-256(plaintext_id)` hex digest for storage and lookup.
    pub sha256_hex: String,
    /// UTC expiry time stored with the session row.
    pub expires_at: DateTime<Utc>,
}

/// Fresh CSRF token material returned at session creation time.
///
/// `plaintext` is echoed by the browser in `X-CSRF-Token`. Persist
/// `sha256_hex`, never the plaintext token.
pub struct IssuedCsrfToken {
    /// Plaintext CSRF token for the readable browser cookie or response body.
    pub plaintext: String,
    /// Lowercase `SHA-256(plaintext)` hex digest for storage and comparison.
    pub sha256_hex: String,
}

/// Maximum age for a newly issued browser session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLifetime {
    /// Duration added to the issue timestamp to compute `expires_at`.
    pub max_age: Duration,
}

impl Default for SessionLifetime {
    fn default() -> Self {
        Self {
            max_age: Duration::hours(24),
        }
    }
}

/// Issues a fresh web-session id, storage hash, and expiry timestamp.
///
/// # Errors
///
/// Returns [`AuthError::Internal`] if the operating system random source fails.
pub fn issue_session(now: DateTime<Utc>, lifetime: SessionLifetime) -> AuthResult<IssuedSession> {
    let plaintext_id = secret::issue_id()?;
    let sha256_hex = hash_session_id(&plaintext_id);
    let expires_at = now + lifetime.max_age;

    Ok(IssuedSession {
        plaintext_id,
        sha256_hex,
        expires_at,
    })
}

/// Issues a fresh CSRF token and storage hash.
///
/// # Errors
///
/// Returns [`AuthError::Internal`] if the operating system random source fails.
pub fn issue_csrf_token() -> AuthResult<IssuedCsrfToken> {
    let plaintext = secret::issue_id()?;
    let sha256_hex = hash_csrf_token(&plaintext);

    Ok(IssuedCsrfToken {
        plaintext,
        sha256_hex,
    })
}

/// Computes the lowercase `SHA-256(plaintext_id)` hex digest for a session id.
pub fn hash_session_id(plaintext_id: &str) -> String {
    secret::sha256_hex(plaintext_id)
}

/// Computes the lowercase `SHA-256(plaintext)` hex digest for a CSRF token.
pub fn hash_csrf_token(plaintext: &str) -> String {
    secret::sha256_hex(plaintext)
}

/// Validates that a plaintext session id matches the issuance format.
///
/// # Errors
///
/// Returns [`AuthError::InvalidSessionId`] when the id length or alphabet is invalid.
pub fn validate_session_id_format(plaintext_id: &str) -> AuthResult<()> {
    secret::validate_format(plaintext_id).map_err(|_| AuthError::InvalidSessionId)
}
