use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};
use zk_domain::AuthScope;

use crate::{ServerError, ServerResult};

use super::timestamps::{decode_optional_timestamp, decode_timestamp, encode_timestamp};

const INSERT_TOKEN_SQL: &str = r"
INSERT INTO device_tokens (
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6
)
";
const GET_TOKEN_BY_ID_SQL: &str = r"
SELECT
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
FROM device_tokens
WHERE token_id = ?1
LIMIT 1
";
const LOOKUP_TOKEN_BY_HASH_SQL: &str = r"
SELECT
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
FROM device_tokens
WHERE token_sha256_hex = ?1 AND revoked_at IS NULL
LIMIT 1
";
const LIST_TOKENS_FOR_DEVICE_SQL: &str = r"
SELECT
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
FROM device_tokens
WHERE device_id = ?1 AND revoked_at IS NULL
ORDER BY created_at DESC, token_id ASC
";
const LIST_UNBOUND_TOKENS_SQL: &str = r"
SELECT
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
FROM device_tokens
WHERE device_id IS NULL AND revoked_at IS NULL
ORDER BY created_at DESC, token_id ASC
";
const LIST_ALL_ACTIVE_TOKENS_SQL: &str = r"
SELECT
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
FROM device_tokens
WHERE revoked_at IS NULL
ORDER BY created_at DESC, token_id ASC
LIMIT ?1
";
const TOUCH_TOKEN_LAST_USED_SQL: &str =
    "UPDATE device_tokens SET last_used_at = ?2 WHERE token_id = ?1";
const REVOKE_TOKEN_SQL: &str =
    "UPDATE device_tokens SET revoked_at = ?2 WHERE token_id = ?1 AND revoked_at IS NULL";
const LIST_ALL_ACTIVE_CAP: usize = 1_000;
const LIST_ALL_ACTIVE_QUERY_LIMIT: i64 = 1_001;

/// New `device_tokens` row to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTokenInsert {
    /// Stable token identifier.
    pub token_id: String,
    /// Optional owning device identifier.
    pub device_id: Option<String>,
    /// Human-readable operator label.
    pub label: String,
    /// Lowercase `SHA-256(plaintext_token)` hex digest.
    pub token_sha256_hex: String,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
    /// Token creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Persisted `device_tokens` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTokenRow {
    /// Stable token identifier.
    pub token_id: String,
    /// Optional owning device identifier.
    pub device_id: Option<String>,
    /// Human-readable operator label.
    pub label: String,
    /// Lowercase `SHA-256(plaintext_token)` hex digest.
    pub token_sha256_hex: String,
    /// Authorization scopes granted to the token.
    pub scopes: Vec<AuthScope>,
    /// Token creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent successful authentication timestamp.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Revocation timestamp when the token is no longer valid.
    pub revoked_at: Option<DateTime<Utc>>,
}

struct DeviceTokenLookupRow {
    token_id: String,
    device_id: Option<String>,
    label: String,
    token_sha256_hex: String,
    scopes_json: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

/// Narrow repository over the `device_tokens` table.
///
/// TG14-011 owns rotation and other service-layer behavior. This repository
/// only persists and reads token rows.
pub struct DeviceTokenRepo;

impl DeviceTokenRepo {
    /// Inserts one device-token row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if scope serialization or the insert fails.
    pub fn issue(conn: &Connection, row: &DeviceTokenInsert) -> ServerResult<()> {
        let scopes_json = serde_json::to_string(&row.scopes)
            .map_err(|source| ServerError::Internal(Box::new(source)))?;
        let encoded_created_at = encode_timestamp(row.created_at);
        let mut statement = conn
            .prepare_cached(INSERT_TOKEN_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![
                row.token_id,
                row.device_id,
                row.label,
                row.token_sha256_hex,
                scopes_json,
                encoded_created_at,
            ])
            .map(|_| ())
            .map_err(ServerError::Database)
    }

    /// Fetches one token row by `token_id`, including revoked rows.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get(conn: &Connection, token_id: &str) -> ServerResult<Option<DeviceTokenRow>> {
        let mut statement = conn
            .prepare_cached(GET_TOKEN_BY_ID_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([token_id], decode_device_token_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Fetches one active token row by `token_sha256_hex`.
    ///
    /// Revoked rows are hidden and return `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn lookup_by_hash(
        conn: &Connection,
        token_sha256_hex: &str,
    ) -> ServerResult<Option<DeviceTokenRow>> {
        let mut statement = conn
            .prepare_cached(LOOKUP_TOKEN_BY_HASH_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([token_sha256_hex], decode_device_token_lookup_row)
            .optional()
            .map_err(ServerError::Database)?
            .map(DeviceTokenLookupRow::into_device_token_row)
            .transpose()
    }

    /// Lists active tokens for a specific device, or unbound active tokens.
    ///
    /// Passing `None` returns rows where `device_id IS NULL`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list_for_device(
        conn: &Connection,
        device_id: Option<&str>,
    ) -> ServerResult<Vec<DeviceTokenRow>> {
        let sql = if device_id.is_some() {
            LIST_TOKENS_FOR_DEVICE_SQL
        } else {
            LIST_UNBOUND_TOKENS_SQL
        };
        let mut statement = conn.prepare_cached(sql).map_err(ServerError::Database)?;
        let rows = match device_id {
            Some(device_id) => statement
                .query_map([device_id], decode_device_token_row)
                .map_err(ServerError::Database)?,
            None => statement
                .query_map([], decode_device_token_row)
                .map_err(ServerError::Database)?,
        };

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Lists active tokens across all devices, capped at 1000 rows.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list_all_active(conn: &Connection) -> ServerResult<Vec<DeviceTokenRow>> {
        let mut statement = conn
            .prepare_cached(LIST_ALL_ACTIVE_TOKENS_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([LIST_ALL_ACTIVE_QUERY_LIMIT], decode_device_token_row)
            .map_err(ServerError::Database)?;
        let mut rows = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)?;

        if rows.len() > LIST_ALL_ACTIVE_CAP {
            tracing::warn!("active device token list exceeded 1000 rows; truncating result");
            rows.truncate(LIST_ALL_ACTIVE_CAP);
        }

        Ok(rows)
    }

    /// Marks one active token row as revoked.
    ///
    /// Returns `false` when the token is missing or already revoked.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn revoke(
        conn: &Connection,
        token_id: &str,
        revoked_at: DateTime<Utc>,
    ) -> ServerResult<bool> {
        let encoded_revoked_at = encode_timestamp(revoked_at);
        let mut statement = conn
            .prepare_cached(REVOKE_TOKEN_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![token_id, encoded_revoked_at])
            .map(|rows_updated| rows_updated == 1)
            .map_err(ServerError::Database)
    }

    /// Updates only the `last_used_at` column for one token row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn touch_last_used(
        conn: &Connection,
        token_id: &str,
        now: DateTime<Utc>,
    ) -> ServerResult<()> {
        let encoded_timestamp = encode_timestamp(now);
        let mut statement = conn
            .prepare_cached(TOUCH_TOKEN_LAST_USED_SQL)
            .map_err(ServerError::Database)?;

        statement
            .execute(rusqlite::params![token_id, encoded_timestamp])
            .map(|_| ())
            .map_err(ServerError::Database)
    }
}

impl DeviceTokenLookupRow {
    fn into_device_token_row(self) -> ServerResult<DeviceTokenRow> {
        let scopes = serde_json::from_str(&self.scopes_json)
            .map_err(|_| ServerError::ScopeDecode(self.token_id.clone()))?;

        Ok(DeviceTokenRow {
            token_id: self.token_id,
            device_id: self.device_id,
            label: self.label,
            token_sha256_hex: self.token_sha256_hex,
            scopes,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            revoked_at: self.revoked_at,
        })
    }
}

fn decode_device_token_lookup_row(row: &Row<'_>) -> rusqlite::Result<DeviceTokenLookupRow> {
    Ok(DeviceTokenLookupRow {
        token_id: row.get(0)?,
        device_id: row.get(1)?,
        label: row.get(2)?,
        token_sha256_hex: row.get(3)?,
        scopes_json: row.get(4)?,
        created_at: decode_timestamp(row, 5)?,
        last_used_at: decode_optional_timestamp(row, 6)?,
        revoked_at: decode_optional_timestamp(row, 7)?,
    })
}

fn decode_device_token_row(row: &Row<'_>) -> rusqlite::Result<DeviceTokenRow> {
    let scopes_json = row.get::<_, String>(4)?;

    Ok(DeviceTokenRow {
        token_id: row.get(0)?,
        device_id: row.get(1)?,
        label: row.get(2)?,
        token_sha256_hex: row.get(3)?,
        scopes: decode_scopes(&scopes_json)?,
        created_at: decode_timestamp(row, 5)?,
        last_used_at: decode_optional_timestamp(row, 6)?,
        revoked_at: decode_optional_timestamp(row, 7)?,
    })
}

fn decode_scopes(scopes_json: &str) -> rusqlite::Result<Vec<AuthScope>> {
    serde_json::from_str(scopes_json).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(source))
    })
}
