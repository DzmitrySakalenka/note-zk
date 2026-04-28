//! Device-token issuance and validation primitives.

use data_encoding::{BASE32_NOPAD, HEXLOWER};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

use crate::{AuthError, AuthResult};

const DEVICE_TOKEN_BYTES: usize = 32;
const DEVICE_TOKEN_PLAINTEXT_LEN: usize = 52;

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
    let mut random_bytes = [0_u8; DEVICE_TOKEN_BYTES];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut random_bytes)
        .map_err(internal_error)?;

    let plaintext = BASE32_NOPAD.encode(&random_bytes);
    debug_assert_eq!(plaintext.len(), DEVICE_TOKEN_PLAINTEXT_LEN);
    let sha256_hex = hash_device_token(&plaintext);

    Ok(IssuedDeviceToken {
        plaintext,
        sha256_hex,
    })
}

/// Computes the lowercase `SHA-256(plaintext)` hex digest for a device token.
pub fn hash_device_token(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.as_bytes());
    HEXLOWER.encode(&digest)
}

/// Validates that a plaintext device token matches the issuance format.
///
/// # Errors
///
/// Returns [`AuthError::InvalidToken`] when the token length or alphabet is invalid.
pub fn validate_device_token_format(plaintext: &str) -> AuthResult<()> {
    if plaintext.len() != DEVICE_TOKEN_PLAINTEXT_LEN {
        return Err(AuthError::InvalidToken);
    }

    if plaintext
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'2'..=b'7'))
    {
        return Ok(());
    }

    Err(AuthError::InvalidToken)
}

fn internal_error(error: impl std::fmt::Display) -> AuthError {
    AuthError::Internal(error.to_string())
}
