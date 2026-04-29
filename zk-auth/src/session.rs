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
    let mut random_bytes = [0_u8; SESSION_ID_BYTES];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut random_bytes)
        .map_err(internal_error)?;

    let plaintext_id = BASE32_NOPAD.encode(&random_bytes);
    debug_assert_eq!(plaintext_id.len(), SESSION_ID_PLAINTEXT_LEN);
    let sha256_hex = hash_session_id(&plaintext_id);
    let expires_at = now + lifetime.max_age;

    Ok(IssuedSession {
        plaintext_id,
        sha256_hex,
        expires_at,
    })
}

/// Computes the lowercase `SHA-256(plaintext_id)` hex digest for a session id.
pub fn hash_session_id(plaintext_id: &str) -> String {
    let digest = Sha256::digest(plaintext_id.as_bytes());
    HEXLOWER.encode(&digest)
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

fn internal_error(error: impl std::fmt::Display) -> AuthError {
    AuthError::Internal(error.to_string())
}
