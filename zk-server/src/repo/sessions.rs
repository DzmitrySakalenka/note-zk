use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, types::Type};
use zk_domain::AuthScope;

use crate::{ServerError, ServerResult};

use super::timestamps::{decode_optional_timestamp, decode_timestamp, encode_timestamp};

const CREATE_SESSION_SQL: &str = r"
INSERT INTO web_sessions (
    session_id_sha256_hex,
    user_id,
    csrf_token_sha256_hex,
    scopes_json,
    created_at,
    expires_at,
    last_seen_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7
)
";
const GET_BY_SESSION_HASH_SQL: &str = r"
SELECT
    session_id_sha256_hex,
    user_id,
    csrf_token_sha256_hex,
    scopes_json,
    created_at,
    expires_at,
    last_seen_at,
    revoked_at
FROM web_sessions
WHERE session_id_sha256_hex = ?1
  AND revoked_at IS NULL
  AND expires_at > ?2
LIMIT 1
";
const REFRESH_LAST_SEEN_SQL: &str =
    "UPDATE web_sessions SET last_seen_at = ?2 WHERE session_id_sha256_hex = ?1";
const REVOKE_SESSION_SQL: &str = r"
UPDATE web_sessions
SET revoked_at = ?2
WHERE session_id_sha256_hex = ?1 AND revoked_at IS NULL
";
const REVOKE_ALL_FOR_USER_SQL: &str = r"
UPDATE web_sessions
SET revoked_at = ?2
WHERE user_id = ?1 AND revoked_at IS NULL
";
const SWEEP_EXPIRED_SQL: &str = r"
DELETE FROM web_sessions
WHERE expires_at <= ?1
  AND (revoked_at IS NOT NULL OR expires_at <= ?2)
";

/// New browser-session row to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSessionInsert {
    /// Lowercase `SHA-256(plaintext_session_id)` hex digest.
    pub session_id_sha256_hex: String,
    /// Owning user identifier.
    pub user_id: String,
    /// Lowercase `SHA-256(plaintext_csrf_token)` hex digest.
    pub csrf_token_sha256_hex: String,
    /// Authorization scopes granted to the session.
    pub scopes: Vec<AuthScope>,
    /// Session creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Session expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

/// Persisted `web_sessions` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSessionRow {
    /// Lowercase `SHA-256(plaintext_session_id)` hex digest.
    pub session_id_sha256_hex: String,
    /// Owning user identifier.
    pub user_id: String,
    /// Lowercase `SHA-256(plaintext_csrf_token)` hex digest.
    pub csrf_token_sha256_hex: String,
    /// Authorization scopes granted to the session.
    pub scopes: Vec<AuthScope>,
    /// Session creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Session expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Most recent successful use timestamp.
    pub last_seen_at: DateTime<Utc>,
    /// Revocation timestamp, if revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Narrow repository over the `web_sessions` table.
pub struct WebSessionRepo;

impl WebSessionRepo {
    /// Inserts one browser-session row.
    ///
    /// `last_seen_at` is initialized to `created_at`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] when the session id hash already
    /// exists, or [`ServerError`] if scope serialization or insertion fails.
    pub fn create(conn: &Connection, row: &WebSessionInsert) -> ServerResult<()> {
        let scopes_json = serde_json::to_string(&row.scopes)
            .map_err(|source| ServerError::Internal(Box::new(source)))?;
        let encoded_created_at = encode_timestamp(row.created_at);
        let encoded_expires_at = encode_timestamp(row.expires_at);
        let mut statement = conn
            .prepare_cached(CREATE_SESSION_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![
                row.session_id_sha256_hex,
                row.user_id,
                row.csrf_token_sha256_hex,
                scopes_json,
                encoded_created_at,
                encoded_expires_at,
                encoded_created_at,
            ])
            .map(|_| ())
            .map_err(map_create_session_error)
    }

    /// Fetches one active, unexpired session row by session id hash.
    ///
    /// Revoked and expired rows are hidden and return `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get_by_session_hash(
        conn: &Connection,
        session_id_sha256_hex: &str,
        now: DateTime<Utc>,
    ) -> ServerResult<Option<WebSessionRow>> {
        let encoded_now = encode_timestamp(now);
        let mut statement = conn
            .prepare_cached(GET_BY_SESSION_HASH_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row(
                rusqlite::params![session_id_sha256_hex, encoded_now],
                decode_web_session_row,
            )
            .optional()
            .map_err(ServerError::Database)
    }

    /// Updates only the `last_seen_at` column for one session row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn refresh_last_seen(
        conn: &Connection,
        session_id_sha256_hex: &str,
        now: DateTime<Utc>,
    ) -> ServerResult<()> {
        let encoded_now = encode_timestamp(now);
        let mut statement = conn
            .prepare_cached(REFRESH_LAST_SEEN_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![session_id_sha256_hex, encoded_now])
            .map(|_| ())
            .map_err(ServerError::Database)
    }

    /// Marks one active session row as revoked.
    ///
    /// Returns `false` when the session is missing or already revoked.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn revoke(
        conn: &Connection,
        session_id_sha256_hex: &str,
        revoked_at: DateTime<Utc>,
    ) -> ServerResult<bool> {
        let encoded_revoked_at = encode_timestamp(revoked_at);
        let mut statement = conn
            .prepare_cached(REVOKE_SESSION_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![session_id_sha256_hex, encoded_revoked_at,])
            .map(|rows_updated| rows_updated == 1)
            .map_err(ServerError::Database)
    }

    /// Marks all active session rows for one user as revoked.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails or the affected-row count
    /// does not fit in `u32`.
    pub fn revoke_all_for_user(
        conn: &Connection,
        user_id: &str,
        revoked_at: DateTime<Utc>,
    ) -> ServerResult<u32> {
        let encoded_revoked_at = encode_timestamp(revoked_at);
        let mut statement = conn
            .prepare_cached(REVOKE_ALL_FOR_USER_SQL)
            .map_err(ServerError::Database)?;

        let rows_updated = statement
            .execute(rusqlite::params![user_id, encoded_revoked_at])
            .map_err(ServerError::Database)?;

        rows_updated_to_u32(rows_updated).map_err(ServerError::Database)
    }

    /// Hard-deletes expired session rows past the retention predicate.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete fails or the affected-row count
    /// does not fit in `u32`.
    pub fn sweep_expired(conn: &Connection, now: DateTime<Utc>) -> ServerResult<u32> {
        let encoded_now = encode_timestamp(now);
        let encoded_grace_cutoff = encode_timestamp(now - Duration::hours(24));
        let mut statement = conn
            .prepare_cached(SWEEP_EXPIRED_SQL)
            .map_err(ServerError::Database)?;

        let rows_deleted = statement
            .execute(rusqlite::params![encoded_now, encoded_grace_cutoff])
            .map_err(ServerError::Database)?;

        rows_updated_to_u32(rows_deleted).map_err(ServerError::Database)
    }
}

fn decode_web_session_row(row: &Row<'_>) -> rusqlite::Result<WebSessionRow> {
    let scopes_json = row.get::<_, String>(3)?;

    Ok(WebSessionRow {
        session_id_sha256_hex: row.get(0)?,
        user_id: row.get(1)?,
        csrf_token_sha256_hex: row.get(2)?,
        scopes: decode_scopes(&scopes_json)?,
        created_at: decode_timestamp(row, 4)?,
        expires_at: decode_timestamp(row, 5)?,
        last_seen_at: decode_timestamp(row, 6)?,
        revoked_at: decode_optional_timestamp(row, 7)?,
    })
}

fn decode_scopes(scopes_json: &str) -> rusqlite::Result<Vec<AuthScope>> {
    serde_json::from_str(scopes_json).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(source))
    })
}

fn rows_updated_to_u32(rows_updated: usize) -> rusqlite::Result<u32> {
    u32::try_from(rows_updated).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, i64::MAX))
}

fn map_create_session_error(source: rusqlite::Error) -> ServerError {
    if is_session_id_collision(&source) {
        ServerError::Conflict("session id collision".to_string())
    } else {
        ServerError::Database(source)
    }
}

fn is_session_id_collision(source: &rusqlite::Error) -> bool {
    matches!(
        source,
        rusqlite::Error::SqliteFailure(error, Some(message))
            if error.code == ErrorCode::ConstraintViolation
                && message.contains("web_sessions.session_id_sha256_hex")
    )
}
