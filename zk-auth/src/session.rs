//! Web-session id issuance and validation primitives.

use chrono::{DateTime, Duration, Utc};
use data_encoding::{BASE32_NOPAD, HEXLOWER};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

use crate::{AuthError, AuthResult};

const SESSION_ID_BYTES: usize = 32;
const SESSION_ID_PLAINTEXT_LEN: usize = 52;

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
    let plaintext_id = issue_base32_secret()?;
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
    let plaintext = issue_base32_secret()?;
    let sha256_hex = hash_csrf_token(&plaintext);

    Ok(IssuedCsrfToken {
        plaintext,
        sha256_hex,
    })
}

/// Computes the lowercase `SHA-256(plaintext_id)` hex digest for a session id.
pub fn hash_session_id(plaintext_id: &str) -> String {
    sha256_hex(plaintext_id)
}

/// Computes the lowercase `SHA-256(plaintext)` hex digest for a CSRF token.
pub fn hash_csrf_token(plaintext: &str) -> String {
    sha256_hex(plaintext)
}

/// Validates that a plaintext session id matches the issuance format.
///
/// # Errors
///
/// Returns [`AuthError::InvalidSessionId`] when the id length or alphabet is invalid.
pub fn validate_session_id_format(plaintext_id: &str) -> AuthResult<()> {
    if plaintext_id.len() != SESSION_ID_PLAINTEXT_LEN {
        return Err(AuthError::InvalidSessionId);
    }

    if plaintext_id
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'2'..=b'7'))
    {
        return Ok(());
    }

    Err(AuthError::InvalidSessionId)
}

fn issue_base32_secret() -> AuthResult<String> {
    let mut random_bytes = [0_u8; SESSION_ID_BYTES];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut random_bytes)
        .map_err(internal_error)?;

    let plaintext = BASE32_NOPAD.encode(&random_bytes);
    debug_assert_eq!(plaintext.len(), SESSION_ID_PLAINTEXT_LEN);

    Ok(plaintext)
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    HEXLOWER.encode(&digest)
}

fn internal_error(error: impl std::fmt::Display) -> AuthError {
    AuthError::Internal(error.to_string())
}
