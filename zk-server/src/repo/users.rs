use chrono::{DateTime, Utc};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, ffi};

use crate::{ServerError, ServerResult};

use super::timestamps::{decode_optional_timestamp, decode_timestamp, encode_timestamp};

const CREATE_USER_SQL: &str = r"
INSERT INTO users (
    user_id,
    username,
    password_hash,
    created_at
) VALUES (
    ?1, ?2, ?3, ?4
)
";
const GET_PASSWORD_HASH_FOR_LOGIN_SQL: &str = r"
SELECT
    user_id,
    password_hash,
    disabled_at
FROM users
WHERE username = ?1
LIMIT 1
";
const GET_USER_BY_ID_SQL: &str = r"
SELECT
    user_id,
    username,
    created_at,
    disabled_at
FROM users
WHERE user_id = ?1
LIMIT 1
";
const UPDATE_PASSWORD_HASH_SQL: &str = "UPDATE users SET password_hash = ?2 WHERE user_id = ?1";
const DISABLE_USER_SQL: &str = "UPDATE users SET disabled_at = ?2 WHERE user_id = ?1";
const COUNT_USERS_SQL: &str = "SELECT COUNT(*) FROM users";
const LIST_USERS_SQL: &str = r"
SELECT
    user_id,
    username,
    created_at,
    disabled_at
FROM users
ORDER BY user_id ASC
";

/// Public user row without password hash material.
pub struct UserRow {
    /// Stable user identifier.
    pub user_id: String,
    /// Login username.
    pub username: String,
    /// User creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the user was disabled, if disabled.
    pub disabled_at: Option<DateTime<Utc>>,
}

/// Login lookup row containing password hash material for service verification.
pub struct UserPasswordRow {
    /// Stable user identifier.
    pub user_id: String,
    /// Argon2id PHC password hash.
    pub password_hash: String,
    /// Timestamp when the user was disabled, if disabled.
    pub disabled_at: Option<DateTime<Utc>>,
}

/// Narrow repository over the `users` table.
///
/// TG14-010 owns password verification in the service layer. `UserRepo` never
/// sees plaintext passwords; it only persists and returns already-hashed PHC
/// strings through the login-specific lookup.
pub struct UserRepo;

impl UserRepo {
    /// Inserts one password-authenticated user row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] when `username` already exists, or
    /// [`ServerError`] if the insert fails.
    pub fn create(
        conn: &Connection,
        user_id: &str,
        username: &str,
        password_hash: &str,
        created_at: DateTime<Utc>,
    ) -> ServerResult<()> {
        let encoded_created_at = encode_timestamp(created_at);
        let mut statement = conn
            .prepare_cached(CREATE_USER_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![
                user_id,
                username,
                password_hash,
                encoded_created_at,
            ])
            .map(|_| ())
            .map_err(map_create_user_error)
    }

    /// Fetches password-hash material by `username` for login verification.
    ///
    /// Disabled rows are returned so the service layer can make the auth
    /// decision without changing the repository query shape.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get_password_hash_for_login(
        conn: &Connection,
        username: &str,
    ) -> ServerResult<Option<UserPasswordRow>> {
        let mut statement = conn
            .prepare_cached(GET_PASSWORD_HASH_FOR_LOGIN_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([username], decode_user_password_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Fetches one user row without password hash material by `user_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get_by_id(conn: &Connection, user_id: &str) -> ServerResult<Option<UserRow>> {
        let mut statement = conn
            .prepare_cached(GET_USER_BY_ID_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([user_id], decode_user_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Updates only the stored password hash for one user.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn update_password_hash(
        conn: &Connection,
        user_id: &str,
        password_hash: &str,
    ) -> ServerResult<()> {
        let mut statement = conn
            .prepare_cached(UPDATE_PASSWORD_HASH_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![user_id, password_hash])
            .map(|_| ())
            .map_err(ServerError::Database)
    }

    /// Sets `disabled_at` for one user.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn disable(
        conn: &Connection,
        user_id: &str,
        disabled_at: DateTime<Utc>,
    ) -> ServerResult<()> {
        let encoded_disabled_at = encode_timestamp(disabled_at);
        let mut statement = conn
            .prepare_cached(DISABLE_USER_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![user_id, encoded_disabled_at])
            .map(|_| ())
            .map_err(ServerError::Database)
    }

    /// Counts all user rows, including disabled accounts.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the count query fails.
    pub fn count(conn: &Connection) -> ServerResult<u32> {
        let mut statement = conn
            .prepare_cached(COUNT_USERS_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([], |row| row.get(0))
            .map_err(ServerError::Database)
    }

    /// Lists all users sorted by `user_id` ascending.
    ///
    /// Password hash material is intentionally omitted from this listing.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list(conn: &Connection) -> ServerResult<Vec<UserRow>> {
        let mut statement = conn
            .prepare(LIST_USERS_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([], decode_user_row)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }
}

fn decode_user_row(row: &Row<'_>) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        user_id: row.get(0)?,
        username: row.get(1)?,
        created_at: decode_timestamp(row, 2)?,
        disabled_at: decode_optional_timestamp(row, 3)?,
    })
}

fn decode_user_password_row(row: &Row<'_>) -> rusqlite::Result<UserPasswordRow> {
    Ok(UserPasswordRow {
        user_id: row.get(0)?,
        password_hash: row.get(1)?,
        disabled_at: decode_optional_timestamp(row, 2)?,
    })
}

fn map_create_user_error(source: rusqlite::Error) -> ServerError {
    if is_username_unique_constraint(&source) {
        ServerError::Conflict("username already exists".to_string())
    } else {
        ServerError::Database(source)
    }
}

fn is_username_unique_constraint(source: &rusqlite::Error) -> bool {
    matches!(
        source,
        rusqlite::Error::SqliteFailure(error, Some(message))
            if error.code == ErrorCode::ConstraintViolation
                && error.extended_code == ffi::SQLITE_CONSTRAINT_UNIQUE
                && message.contains("users.username")
    )
}
