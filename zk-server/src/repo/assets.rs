use std::io::{self, ErrorKind};
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};

use crate::{ServerError, ServerResult};

const SERVER_ASSET_COLUMNS: &str = r"    asset_id,
    current_path,
    content_hash,
    mime_type,
    size_bytes,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    server_revision";
static CREATE_ASSET_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
INSERT INTO assets (
{SERVER_ASSET_COLUMNS}
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12
)
"
    )
});
const UPDATE_ASSET_SQL: &str = r"
UPDATE assets
SET
    current_path = ?2,
    content_hash = ?3,
    mime_type = ?4,
    size_bytes = ?5,
    created_at = ?6,
    updated_at = ?7,
    trashed_at = ?8,
    deleted_at = ?9,
    is_trashed = ?10,
    is_deleted = ?11,
    server_revision = ?12
WHERE asset_id = ?1
";
static GET_ASSET_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
SELECT
{SERVER_ASSET_COLUMNS}
FROM assets
WHERE asset_id = ?1
"
    )
});
static GET_ASSET_BY_PATH_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
SELECT
{SERVER_ASSET_COLUMNS}
FROM assets
WHERE current_path = ?1 AND is_deleted = 0
LIMIT 1
"
    )
});
static LIST_ALL_ASSETS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
SELECT
{SERVER_ASSET_COLUMNS}
FROM assets
ORDER BY server_revision ASC, asset_id ASC
"
    )
});
static LIST_ASSETS_SINCE_REVISION_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
SELECT
{SERVER_ASSET_COLUMNS}
FROM assets
WHERE server_revision > ?1
ORDER BY server_revision ASC, asset_id ASC
"
    )
});
const DELETE_ASSET_HARD_SQL: &str = "DELETE FROM assets WHERE asset_id = ?1";
const EXISTS_ASSET_SQL: &str = "SELECT 1 FROM assets WHERE asset_id = ?1 LIMIT 1";

/// Persisted server-authoritative `assets` row.
pub struct AssetRow {
    /// Stable asset identifier.
    pub asset_id: String,
    /// Current canonical vault path.
    pub current_path: String,
    /// Content hash for the current snapshot.
    pub content_hash: String,
    /// Detected MIME type when known.
    pub mime_type: Option<String>,
    /// Asset size in bytes.
    pub size_bytes: i64,
    /// First known creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Trash timestamp when the asset is trashed.
    pub trashed_at: Option<DateTime<Utc>>,
    /// Deletion timestamp when the asset is soft-deleted.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Whether the asset is currently trashed.
    pub is_trashed: bool,
    /// Whether the asset is currently soft-deleted.
    pub is_deleted: bool,
    /// Current server-side revision counter for the asset.
    pub server_revision: i64,
}

/// Stateless repository over the server `assets` table.
pub struct AssetsRepo;

impl AssetsRepo {
    /// Inserts one asset row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] when `asset_id` already exists, or
    /// [`ServerError`] if the insert fails.
    pub fn create(conn: &Connection, row: &AssetRow) -> ServerResult<()> {
        if asset_exists(conn, &row.asset_id)? {
            return Err(ServerError::Conflict("asset already exists".to_string()));
        }

        let created_at = encode_timestamp(row.created_at);
        let updated_at = encode_timestamp(row.updated_at);
        let trashed_at = encode_optional_timestamp(row.trashed_at);
        let deleted_at = encode_optional_timestamp(row.deleted_at);

        conn.execute(
            CREATE_ASSET_SQL.as_str(),
            rusqlite::params![
                &row.asset_id,
                &row.current_path,
                &row.content_hash,
                row.mime_type.as_deref(),
                row.size_bytes,
                &created_at,
                &updated_at,
                trashed_at.as_deref(),
                deleted_at.as_deref(),
                row.is_trashed,
                row.is_deleted,
                row.server_revision,
            ],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }

    /// Updates one asset row by `asset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn update(conn: &Connection, row: &AssetRow) -> ServerResult<()> {
        let created_at = encode_timestamp(row.created_at);
        let updated_at = encode_timestamp(row.updated_at);
        let trashed_at = encode_optional_timestamp(row.trashed_at);
        let deleted_at = encode_optional_timestamp(row.deleted_at);

        conn.execute(
            UPDATE_ASSET_SQL,
            rusqlite::params![
                &row.asset_id,
                &row.current_path,
                &row.content_hash,
                row.mime_type.as_deref(),
                row.size_bytes,
                &created_at,
                &updated_at,
                trashed_at.as_deref(),
                deleted_at.as_deref(),
                row.is_trashed,
                row.is_deleted,
                row.server_revision,
            ],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }

    /// Fetches one asset row by `asset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get(conn: &Connection, asset_id: &str) -> ServerResult<Option<AssetRow>> {
        let mut statement = conn
            .prepare_cached(GET_ASSET_SQL.as_str())
            .map_err(ServerError::Database)?;

        statement
            .query_row([asset_id], decode_asset_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Fetches one live asset row by `current_path`.
    ///
    /// Soft-deleted rows are excluded so callers can reuse purged paths without
    /// ambiguity.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get_by_path(conn: &Connection, path: &str) -> ServerResult<Option<AssetRow>> {
        let mut statement = conn
            .prepare_cached(GET_ASSET_BY_PATH_SQL.as_str())
            .map_err(ServerError::Database)?;

        statement
            .query_row([path], decode_asset_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Lists all asset rows in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list_all(conn: &Connection) -> ServerResult<Vec<AssetRow>> {
        list_assets(conn, LIST_ALL_ASSETS_SQL.as_str(), None)
    }

    /// Lists asset rows whose `server_revision` is greater than `min_rev`.
    ///
    /// Rows are ordered by `server_revision` ascending, then `asset_id`
    /// ascending for deterministic ties.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list_since_revision(conn: &Connection, min_rev: i64) -> ServerResult<Vec<AssetRow>> {
        list_assets(conn, LIST_ASSETS_SINCE_REVISION_SQL.as_str(), Some(min_rev))
    }

    /// Permanently deletes one asset row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete fails.
    pub fn delete_hard(conn: &Connection, asset_id: &str) -> ServerResult<()> {
        conn.execute(DELETE_ASSET_HARD_SQL, [asset_id])
            .map(|_| ())
            .map_err(ServerError::Database)
    }
}

fn asset_exists(conn: &Connection, asset_id: &str) -> ServerResult<bool> {
    let mut statement = conn
        .prepare_cached(EXISTS_ASSET_SQL)
        .map_err(ServerError::Database)?;

    statement
        .query_row([asset_id], |_| Ok(()))
        .optional()
        .map(|maybe_row| maybe_row.is_some())
        .map_err(ServerError::Database)
}

fn list_assets(conn: &Connection, sql: &str, min_rev: Option<i64>) -> ServerResult<Vec<AssetRow>> {
    let mut statement = conn.prepare_cached(sql).map_err(ServerError::Database)?;
    let rows = match min_rev {
        Some(min_rev) => statement
            .query_map([min_rev], decode_asset_row)
            .map_err(ServerError::Database)?,
        None => statement
            .query_map([], decode_asset_row)
            .map_err(ServerError::Database)?,
    };

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(ServerError::Database)
}

fn decode_asset_row(row: &Row<'_>) -> rusqlite::Result<AssetRow> {
    Ok(AssetRow {
        asset_id: row.get(0)?,
        current_path: row.get(1)?,
        content_hash: row.get(2)?,
        mime_type: row.get(3)?,
        size_bytes: row.get(4)?,
        created_at: decode_timestamp(row, 5)?,
        updated_at: decode_timestamp(row, 6)?,
        trashed_at: decode_optional_timestamp(row, 7)?,
        deleted_at: decode_optional_timestamp(row, 8)?,
        is_trashed: row.get(9)?,
        is_deleted: row.get(10)?,
        server_revision: row.get(11)?,
    })
}

fn encode_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339()
}

fn encode_optional_timestamp(timestamp: Option<DateTime<Utc>>) -> Option<String> {
    timestamp.map(encode_timestamp)
}

fn decode_timestamp(row: &Row<'_>, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let raw_timestamp = row.get::<_, String>(index)?;

    DateTime::parse_from_rfc3339(&raw_timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|source| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                Type::Text,
                Box::new(io::Error::new(ErrorKind::InvalidData, source.to_string())),
            )
        })
}

fn decode_optional_timestamp(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    let Some(raw_timestamp) = row.get::<_, Option<String>>(index)? else {
        return Ok(None);
    };

    DateTime::parse_from_rfc3339(&raw_timestamp)
        .map(|parsed| Some(parsed.with_timezone(&Utc)))
        .map_err(|source| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                Type::Text,
                Box::new(io::Error::new(ErrorKind::InvalidData, source.to_string())),
            )
        })
}
