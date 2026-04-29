use std::{thread, time::Duration};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_auth::{
    AuthError,
    password::{HashParams, hash_password, verify_password as verify_password_hash},
};
use zk_domain::id::NoteId;

use crate::{
    ServerError, ServerResult,
    repo::{UserPasswordRow, UserRepo},
};

const MISSING_USER_DELAY: Duration = Duration::from_millis(50);

/// User returned after successful registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredUser {
    /// Stable user identifier.
    pub user_id: String,
    /// Login username.
    pub username: String,
}

/// User returned after successful password verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedUser {
    /// Stable user identifier.
    pub user_id: String,
}

/// Registers one password-authenticated user.
///
/// # Errors
///
/// Returns [`ServerError::InvalidRequest`] for invalid username or password
/// shape, [`ServerError::Conflict`] when the username already exists, or
/// [`ServerError`] if password hashing or persistence fails.
pub fn register_user(
    conn: &Connection,
    username: &str,
    password: &str,
    now: DateTime<Utc>,
) -> ServerResult<RegisteredUser> {
    validate_username(username)?;
    validate_password(password)?;

    let user_id = NoteId::generate().to_string();
    let password_hash = hash_password(password, HashParams::default()).map_err(map_hash_error)?;

    UserRepo::create(conn, &user_id, username, &password_hash, now)?;

    Ok(RegisteredUser {
        user_id,
        username: username.to_string(),
    })
}

/// Verifies a plaintext password for one username.
///
/// # Errors
///
/// Returns [`ServerError::Unauthorized`] for missing, disabled, or mismatched
/// credentials. Returns [`ServerError`] if the stored password hash is invalid
/// or database lookup fails.
pub fn verify_password(
    conn: &Connection,
    username: &str,
    password: &str,
) -> ServerResult<VerifiedUser> {
    let Some(row) = UserRepo::get_password_hash_for_login(conn, username)? else {
        thread::sleep(MISSING_USER_DELAY);
        return Err(ServerError::Unauthorized);
    };

    verify_active_user_password(&row, password)?;

    Ok(VerifiedUser {
        user_id: row.user_id,
    })
}

fn verify_active_user_password(row: &UserPasswordRow, password: &str) -> ServerResult<()> {
    if row.disabled_at.is_some() {
        return Err(ServerError::Unauthorized);
    }

    verify_password_hash(password, &row.password_hash).map_err(map_verify_error)
}

fn validate_username(username: &str) -> ServerResult<()> {
    if username.is_empty() || username.chars().count() > 64 {
        return Err(ServerError::InvalidRequest("username format".to_string()));
    }

    if !username
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ServerError::InvalidRequest("username format".to_string()));
    }

    Ok(())
}

fn validate_password(password: &str) -> ServerResult<()> {
    if !(8..=256).contains(&password.chars().count()) {
        return Err(ServerError::InvalidRequest("password length".to_string()));
    }

    Ok(())
}

fn map_hash_error(source: AuthError) -> ServerError {
    ServerError::Internal(Box::new(source))
}

fn map_verify_error(source: AuthError) -> ServerError {
    match source {
        AuthError::PasswordVerification => ServerError::Unauthorized,
        other => ServerError::Internal(Box::new(other)),
    }
}
