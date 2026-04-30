//! Password hashing and verification primitives.

use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use rand::rngs::OsRng;

use crate::{AuthError, AuthResult, secret};

/// Tunable Argon2id password hashing parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashParams {
    /// Memory cost in KiB.
    pub memory_kib: u32,
    /// Number of Argon2id iterations.
    pub iterations: u32,
    /// Number of parallel lanes.
    pub parallelism: u32,
}

impl Default for HashParams {
    /// Returns the OWASP minimum Argon2id baseline: 19 MiB, 2 iterations, 1 lane.
    fn default() -> Self {
        Self {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        }
    }
}

/// Hashes a plaintext password into a PHC-format Argon2id string.
///
/// # Errors
///
/// Returns [`AuthError::Internal`] if the supplied Argon2 parameters are
/// invalid or hashing fails.
pub fn hash_password(password: &str, params: HashParams) -> AuthResult<String> {
    let argon2 = argon2_with_params(params)?;
    let salt = generate_salt();

    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(secret::internal_error)
}

/// Verifies a plaintext password against a PHC-format Argon2id hash.
///
/// # Errors
///
/// Returns [`AuthError::InvalidPasswordHash`] when `phc_hash` is malformed.
/// Returns [`AuthError::PasswordVerification`] when verification fails.
pub fn verify_password(password: &str, phc_hash: &str) -> AuthResult<()> {
    let parsed_hash = PasswordHash::new(phc_hash).map_err(|_| AuthError::InvalidPasswordHash)?;

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| AuthError::PasswordVerification)
}

fn argon2_with_params(params: HashParams) -> AuthResult<Argon2<'static>> {
    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        None,
    )
    .map_err(secret::internal_error)?;

    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn generate_salt() -> SaltString {
    SaltString::generate(&mut OsRng)
}
