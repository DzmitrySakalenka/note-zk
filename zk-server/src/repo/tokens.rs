use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::{ServerError, ServerResult};

use super::timestamps::{decode_optional_timestamp, decode_timestamp, encode_timestamp};

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
const TOUCH_TOKEN_LAST_USED_SQL: &str =
    "UPDATE device_tokens SET last_used_at = ?2 WHERE token_id = ?1";

/// Persisted `device_tokens` row used by the TG12 stub auth path.
pub struct DeviceTokenRow {
    /// Stable token identifier.
    pub token_id: String,
    /// Optional owning device identifier.
    pub device_id: Option<String>,
    /// Human-readable operator label.
    pub label: String,
    /// Lowercase `SHA-256(plaintext_token)` hex digest.
    pub token_sha256_hex: String,
    /// Opaque scopes payload stored as raw JSON.
    pub scopes_json: String,
    /// Token creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent successful authentication timestamp.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Revocation timestamp when the token is no longer valid.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Narrow repository over the `device_tokens` table.
///
/// TG14 owns issuance, revocation, rotation, and scope-aware behavior. This
/// TG12 stub only supports lookup for authentication and updating
/// `last_used_at`.
pub struct DeviceTokenRepo;

impl DeviceTokenRepo {
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
            .query_row([token_sha256_hex], decode_device_token_row)
            .optional()
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

fn decode_device_token_row(row: &Row<'_>) -> rusqlite::Result<DeviceTokenRow> {
    Ok(DeviceTokenRow {
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
