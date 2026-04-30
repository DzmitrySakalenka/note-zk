//! Shared secret-generation and digest helpers for auth primitives.

use data_encoding::{BASE32_NOPAD, HEXLOWER};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

use crate::{AuthError, AuthResult};

/// Number of random bytes used for generated opaque ids.
pub(crate) const ID_BYTES: usize = 32;
/// Base32-no-padding plaintext length for `ID_BYTES`.
pub(crate) const ID_PLAINTEXT_LEN: usize = 52;

/// Issues one opaque base32-no-padding id.
///
/// # Errors
///
/// Returns [`AuthError::Internal`] if the operating system random source fails.
pub(crate) fn issue_id() -> AuthResult<String> {
    let mut random_bytes = [0_u8; ID_BYTES];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut random_bytes)
        .map_err(internal_error)?;

    let plaintext = BASE32_NOPAD.encode(&random_bytes);
    debug_assert_eq!(plaintext.len(), ID_PLAINTEXT_LEN);

    Ok(plaintext)
}

/// Validates one generated id against the shared plaintext format.
///
/// # Errors
///
/// Returns [`AuthError::InvalidToken`] when the length or alphabet is invalid.
/// Public wrappers remap this sentinel to their domain-specific error.
pub(crate) fn validate_format(plaintext: &str) -> AuthResult<()> {
    if plaintext.len() != ID_PLAINTEXT_LEN {
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

/// Computes a lowercase `SHA-256(value)` hex digest.
pub(crate) fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    HEXLOWER.encode(&digest)
}

/// Converts implementation errors to a stable auth internal error.
pub(crate) fn internal_error(error: impl std::fmt::Display) -> AuthError {
    AuthError::Internal(error.to_string())
}
