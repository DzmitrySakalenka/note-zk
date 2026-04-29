use std::{fmt, thread, time::Duration};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
pub use zk_auth::session::SessionLifetime;
use zk_auth::{
    AuthError,
    password::{HashParams, hash_password, verify_password as verify_password_hash},
    session::{hash_session_id, issue_csrf_token, issue_session},
    token::issue_device_token as issue_raw_device_token,
};
use zk_domain::{AuthScope, id::NoteId};

use crate::{
    ServerError, ServerResult,
    repo::{
        DeviceTokenInsert, DeviceTokenRepo, UserPasswordRow, UserRepo, WebSessionInsert,
        WebSessionRepo,
    },
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

/// Device-token issue request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTokenSpec {
    /// Human-readable operator label.
    pub label: String,
    /// Optional owning device identifier.
    pub device_id: Option<String>,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
}

/// Device-token rotation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateTokenSpec {
    /// Currently active token identifier to revoke.
    pub current_token_id: String,
    /// Human-readable operator label for the replacement token.
    pub label: String,
    /// Optional owning device identifier for the replacement token.
    pub device_id: Option<String>,
    /// Authorization scopes granted to the replacement token.
    pub scopes: Vec<AuthScope>,
}

/// Freshly issued device token returned once to the caller.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedToken {
    /// Stable token identifier.
    pub token_id: String,
    /// One-time plaintext bearer token.
    pub plaintext: String,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
    /// Token creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl fmt::Debug for IssuedToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "IssuedToken {{ token_id: {:?}, plaintext: <REDACTED>, scopes: {:?}, created_at: {:?} }}",
            self.token_id, self.scopes, self.created_at
        )
    }
}

/// Web-session creation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionSpec {
    /// Owning user identifier.
    pub user_id: String,
    /// Authorization scopes granted to the session.
    pub scopes: Vec<AuthScope>,
    /// Maximum lifetime for the new session.
    pub lifetime: SessionLifetime,
}

/// Freshly issued browser session returned once to the caller.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedSession {
    /// Plaintext session id for the HTTP-only browser cookie.
    pub plaintext_session_id: String,
    /// Plaintext CSRF token for the readable browser cookie or response body.
    pub plaintext_csrf_token: String,
    /// Session expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for IssuedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "IssuedSession {{ plaintext_session_id: <REDACTED>, plaintext_csrf_token: <REDACTED>, expires_at: {:?} }}",
            self.expires_at
        )
    }
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
    let password_hash =
        hash_password(password, HashParams::default()).map_err(map_internal_auth_error)?;

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

/// Issues one fresh device token.
///
/// # Errors
///
/// Returns [`ServerError::InvalidRequest`] for invalid label or empty scopes,
/// or [`ServerError`] if token issuance or persistence fails.
pub fn issue_device_token(
    conn: &Connection,
    spec: &IssueTokenSpec,
    now: DateTime<Utc>,
) -> ServerResult<IssuedToken> {
    validate_issue_token_spec(spec)?;

    let raw_token = issue_raw_device_token().map_err(map_internal_auth_error)?;
    let token_id = NoteId::generate().to_string();
    let scopes = spec.scopes.clone();

    DeviceTokenRepo::issue(
        conn,
        &DeviceTokenInsert {
            token_id: token_id.clone(),
            device_id: spec.device_id.clone(),
            label: spec.label.clone(),
            token_sha256_hex: raw_token.sha256_hex,
            scopes: scopes.clone(),
            created_at: now,
        },
    )?;

    Ok(IssuedToken {
        token_id,
        plaintext: raw_token.plaintext,
        scopes,
        created_at: now,
    })
}

/// Revokes one active device token.
///
/// # Errors
///
/// Returns [`ServerError`] if the revocation update fails.
pub fn revoke_device_token(
    conn: &Connection,
    token_id: &str,
    now: DateTime<Utc>,
) -> ServerResult<bool> {
    DeviceTokenRepo::revoke(conn, token_id, now)
}

/// Rotates one active device token atomically.
///
/// # Errors
///
/// Returns [`ServerError::NotFound`] when the current token is missing or
/// already revoked, [`ServerError::InvalidRequest`] for invalid replacement
/// token input, or [`ServerError`] if token issuance or persistence fails.
pub fn rotate_device_token(
    conn: &mut Connection,
    spec: &RotateTokenSpec,
    now: DateTime<Utc>,
) -> ServerResult<IssuedToken> {
    let transaction = conn.transaction().map_err(ServerError::Database)?;
    let revoked = DeviceTokenRepo::revoke(&transaction, &spec.current_token_id, now)?;

    if !revoked {
        return Err(ServerError::NotFound("token id".to_string()));
    }

    let issue_spec = IssueTokenSpec {
        label: spec.label.clone(),
        device_id: spec.device_id.clone(),
        scopes: spec.scopes.clone(),
    };
    let issued = issue_device_token(&transaction, &issue_spec, now)?;

    transaction.commit().map_err(ServerError::Database)?;

    Ok(issued)
}

/// Creates one browser session and its CSRF token.
///
/// # Errors
///
/// Returns [`ServerError`] if session issuance or persistence fails.
pub fn create_web_session(
    conn: &Connection,
    spec: &CreateSessionSpec,
    now: DateTime<Utc>,
) -> ServerResult<IssuedSession> {
    let raw_session = issue_session(now, spec.lifetime).map_err(map_internal_auth_error)?;
    let csrf_token = issue_csrf_token().map_err(map_internal_auth_error)?;
    let expires_at = raw_session.expires_at;

    WebSessionRepo::create(
        conn,
        &WebSessionInsert {
            session_id_sha256_hex: raw_session.sha256_hex,
            user_id: spec.user_id.clone(),
            csrf_token_sha256_hex: csrf_token.sha256_hex,
            scopes: spec.scopes.clone(),
            created_at: now,
            expires_at,
        },
    )?;

    Ok(IssuedSession {
        plaintext_session_id: raw_session.plaintext_id,
        plaintext_csrf_token: csrf_token.plaintext,
        expires_at,
    })
}

/// Rotates one active browser session.
///
/// # Errors
///
/// Returns [`ServerError::Unauthorized`] for unknown, revoked, or expired
/// sessions, or [`ServerError`] if issuance or persistence fails.
pub fn refresh_web_session(
    conn: &mut Connection,
    current_session_id: &str,
    now: DateTime<Utc>,
    lifetime: SessionLifetime,
) -> ServerResult<IssuedSession> {
    let current_session_hash = hash_session_id(current_session_id);
    let transaction = conn.transaction().map_err(ServerError::Database)?;
    let Some(row) = WebSessionRepo::get_by_session_hash(&transaction, &current_session_hash, now)?
    else {
        return Err(ServerError::Unauthorized);
    };

    WebSessionRepo::revoke(&transaction, &current_session_hash, now)?;
    let issued = create_web_session(
        &transaction,
        &CreateSessionSpec {
            user_id: row.user_id,
            scopes: row.scopes,
            lifetime,
        },
        now,
    )?;

    transaction.commit().map_err(ServerError::Database)?;

    Ok(issued)
}

/// Revokes one browser session by plaintext session id.
///
/// # Errors
///
/// Returns [`ServerError`] if the revocation update fails.
pub fn revoke_web_session(
    conn: &Connection,
    session_id: &str,
    now: DateTime<Utc>,
) -> ServerResult<bool> {
    WebSessionRepo::revoke(conn, &hash_session_id(session_id), now)
}

/// Deletes expired browser sessions according to repository retention policy.
///
/// # Errors
///
/// Returns [`ServerError`] if the sweep fails.
pub fn sweep_expired_sessions(conn: &Connection, now: DateTime<Utc>) -> ServerResult<u32> {
    WebSessionRepo::sweep_expired(conn, now)
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

fn validate_issue_token_spec(spec: &IssueTokenSpec) -> ServerResult<()> {
    validate_token_label(&spec.label)?;
    validate_token_scopes(&spec.scopes)
}

fn validate_token_label(label: &str) -> ServerResult<()> {
    if !(1..=128).contains(&label.chars().count()) {
        return Err(ServerError::InvalidRequest(
            "token label format".to_string(),
        ));
    }

    Ok(())
}

fn validate_token_scopes(scopes: &[AuthScope]) -> ServerResult<()> {
    if scopes.is_empty() {
        return Err(ServerError::InvalidRequest(
            "scopes must not be empty".to_string(),
        ));
    }

    Ok(())
}

fn map_internal_auth_error(source: AuthError) -> ServerError {
    ServerError::Internal(Box::new(source))
}

fn map_verify_error(source: AuthError) -> ServerError {
    match source {
        AuthError::PasswordVerification => ServerError::Unauthorized,
        other => ServerError::Internal(Box::new(other)),
    }
}
