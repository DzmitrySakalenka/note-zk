//! Device-token issuance and validation primitives.

use crate::{AuthError, AuthResult, secret};

/// Fresh device token material returned at issuance time.
///
/// `plaintext` is a bearer secret and should only be shown to the operator once.
/// Persist `sha256_hex`, never the plaintext token.
pub struct IssuedDeviceToken {
    /// One-time plaintext bearer token.
    pub plaintext: String,
    /// Lowercase `SHA-256(plaintext)` hex digest for storage and lookup.
    pub sha256_hex: String,
}

/// Issues a fresh device bearer token and its storage hash.
///
/// # Errors
///
/// Returns [`AuthError::Internal`] if the operating system random source fails.
pub fn issue_device_token() -> AuthResult<IssuedDeviceToken> {
    let plaintext = secret::issue_id()?;
    let sha256_hex = hash_device_token(&plaintext);

    Ok(IssuedDeviceToken {
        plaintext,
        sha256_hex,
    })
}

/// Computes the lowercase `SHA-256(plaintext)` hex digest for a device token.
pub fn hash_device_token(plaintext: &str) -> String {
    secret::sha256_hex(plaintext)
}

/// Validates that a plaintext device token matches the issuance format.
///
/// # Errors
///
/// Returns [`AuthError::InvalidToken`] when the token length or alphabet is invalid.
pub fn validate_device_token_format(plaintext: &str) -> AuthResult<()> {
    secret::validate_format(plaintext).map_err(|_| AuthError::InvalidToken)
}
