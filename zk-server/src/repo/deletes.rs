use std::io::{self, ErrorKind};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};

use crate::{ServerError, ServerResult};

const UPSERT_DELETE_RECORD_SQL: &str = r"
INSERT INTO delete_records (
    item_id,
    item_type,
    original_path,
    trashed_at,
    retention_until,
    deleted_at,
    delete_revision
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7
)
ON CONFLICT(item_id, item_type) DO UPDATE SET
    original_path = excluded.original_path,
    trashed_at = MIN(delete_records.trashed_at, excluded.trashed_at),
    retention_until = MAX(delete_records.retention_until, excluded.retention_until),
    deleted_at = excluded.deleted_at,
    delete_revision = excluded.delete_revision
";
const GET_DELETE_RECORD_SQL: &str = r"
SELECT
    item_id,
    item_type,
    original_path,
    trashed_at,
    retention_until,
    deleted_at,
    delete_revision
FROM delete_records
WHERE item_id = ?1 AND item_type = ?2
LIMIT 1
";
const MARK_DELETE_RECORD_PURGED_SQL: &str = r"
UPDATE delete_records
SET deleted_at = ?3, delete_revision = ?4
WHERE item_id = ?1 AND item_type = ?2
";
const LIST_EXPIRED_DELETE_RECORDS_SQL: &str = r"
SELECT
    item_id,
    item_type,
    original_path,
    trashed_at,
    retention_until,
    deleted_at,
    delete_revision
FROM delete_records
WHERE retention_until <= ?1 AND deleted_at IS NULL
ORDER BY retention_until ASC, item_type ASC, item_id ASC
LIMIT ?2
";
const DELETE_DELETE_RECORD_SQL: &str =
    "DELETE FROM delete_records WHERE item_id = ?1 AND item_type = ?2";

/// Persisted server-authoritative `delete_records` row.
pub struct DeleteRecordRow {
    /// Identifier of the deleted note or asset.
    pub item_id: String,
    /// Item kind text, typically `note` or `asset`.
    pub item_type: String,
    /// Last known canonical path before deletion.
    pub original_path: String,
    /// Timestamp when the item first entered trash.
    pub trashed_at: DateTime<Utc>,
    /// Timestamp when the retention window expires.
    pub retention_until: DateTime<Utc>,
    /// Timestamp when the item was hard-deleted, when known.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Revision assigned to the delete event, when known.
    pub delete_revision: Option<i64>,
}

/// Stateless repository over the server `delete_records` table.
pub struct DeleteRecordsRepo;

impl DeleteRecordsRepo {
    /// Inserts or merges one delete record by logical key `(item_id, item_type)`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the write fails.
    pub fn upsert(conn: &Connection, row: &DeleteRecordRow) -> ServerResult<()> {
        let trashed_at = encode_timestamp(row.trashed_at);
        let retention_until = encode_timestamp(row.retention_until);
        let deleted_at = encode_optional_timestamp(row.deleted_at);

        conn.execute(
            UPSERT_DELETE_RECORD_SQL,
            rusqlite::params![
                &row.item_id,
                &row.item_type,
                &row.original_path,
                &trashed_at,
                &retention_until,
                deleted_at.as_deref(),
                row.delete_revision,
            ],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }

    /// Fetches one delete record by logical key `(item_id, item_type)`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get(
        conn: &Connection,
        item_id: &str,
        item_type: &str,
    ) -> ServerResult<Option<DeleteRecordRow>> {
        let mut statement = conn
            .prepare_cached(GET_DELETE_RECORD_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([item_id, item_type], decode_delete_record_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Marks one delete record as purged and stores the final delete revision.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] when the row does not exist, or
    /// [`ServerError`] if the update fails.
    pub fn mark_purged(
        conn: &Connection,
        item_id: &str,
        item_type: &str,
        deleted_at: DateTime<Utc>,
        revision: i64,
    ) -> ServerResult<()> {
        let deleted_at = encode_timestamp(deleted_at);
        let mut statement = conn
            .prepare_cached(MARK_DELETE_RECORD_PURGED_SQL)
            .map_err(ServerError::Database)?;
        let updated_rows = statement
            .execute(rusqlite::params![item_id, item_type, deleted_at, revision])
            .map_err(ServerError::Database)?;

        if updated_rows == 0 {
            return Err(ServerError::NotFound("delete record not found".to_string()));
        }

        Ok(())
    }

    /// Lists expired delete records ordered for deterministic purge processing.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::InvalidRequest`] when `limit` is outside
    /// `[1, 10_000]`, or [`ServerError`] if the query fails.
    pub fn list_expired(
        conn: &Connection,
        now: DateTime<Utc>,
        limit: i64,
    ) -> ServerResult<Vec<DeleteRecordRow>> {
        validate_limit(limit)?;

        let encoded_now = encode_timestamp(now);
        let mut statement = conn
            .prepare_cached(LIST_EXPIRED_DELETE_RECORDS_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map(
                rusqlite::params![encoded_now, limit],
                decode_delete_record_row,
            )
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Deletes one delete record by logical key `(item_id, item_type)`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete fails.
    pub fn delete(conn: &Connection, item_id: &str, item_type: &str) -> ServerResult<()> {
        conn.execute(DELETE_DELETE_RECORD_SQL, [item_id, item_type])
            .map(|_| ())
            .map_err(ServerError::Database)
    }
}

fn validate_limit(limit: i64) -> ServerResult<()> {
    if !(1..=10_000).contains(&limit) {
        return Err(ServerError::InvalidRequest(
            "limit must be between 1 and 10000".to_string(),
        ));
    }

    Ok(())
}

fn decode_delete_record_row(row: &Row<'_>) -> rusqlite::Result<DeleteRecordRow> {
    Ok(DeleteRecordRow {
        item_id: row.get(0)?,
        item_type: row.get(1)?,
        original_path: row.get(2)?,
        trashed_at: decode_timestamp(row, 3)?,
        retention_until: decode_timestamp(row, 4)?,
        deleted_at: decode_optional_timestamp(row, 5)?,
        delete_revision: row.get(6)?,
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
    let raw_timestamp = row.get::<_, Option<String>>(index)?;

    raw_timestamp
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|source| {
                    rusqlite::Error::FromSqlConversionFailure(
                        index,
                        Type::Text,
                        Box::new(io::Error::new(ErrorKind::InvalidData, source.to_string())),
                    )
                })
        })
        .transpose()
}
