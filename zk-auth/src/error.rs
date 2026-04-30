//! Error types returned by authentication primitives.

/// Result alias used by `zk-auth` public APIs.
pub type AuthResult<T> = Result<T, AuthError>;

/// Error type for password hashing, token validation, and session validation.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// A stored password hash is not a valid PHC string.
    #[error("invalid password hash format")]
    InvalidPasswordHash,
    /// Password verification did not match the stored hash.
    #[error("password verification failed")]
    PasswordVerification,
    /// A device token does not match the accepted plaintext format.
    #[error("token format invalid")]
    InvalidToken,
    /// A web-session id does not match the accepted plaintext format.
    #[error("session id format invalid")]
    InvalidSessionId,
    /// Authentication failed for an implementation detail callers should not branch on.
    #[error("internal error: {0}")]
    Internal(String),
}
