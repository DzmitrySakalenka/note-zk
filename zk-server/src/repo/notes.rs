use std::io::{self, ErrorKind};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};

use crate::{ServerError, ServerResult};

const CREATE_NOTE_SQL: &str = r"
INSERT INTO notes (
    note_id,
    current_path,
    title,
    content_hash,
    body_markdown,
    frontmatter_json,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
)
";
const UPDATE_NOTE_SQL: &str = r"
UPDATE notes
SET
    current_path = ?2,
    title = ?3,
    content_hash = ?4,
    body_markdown = ?5,
    frontmatter_json = ?6,
    created_at = ?7,
    updated_at = ?8,
    trashed_at = ?9,
    deleted_at = ?10,
    is_trashed = ?11,
    is_deleted = ?12,
    last_editor_device_id = ?13,
    server_revision = ?14
WHERE note_id = ?1
";
const GET_NOTE_SQL: &str = r"
SELECT
    note_id,
    current_path,
    title,
    content_hash,
    body_markdown,
    frontmatter_json,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision
FROM notes
WHERE note_id = ?1
";
const GET_NOTE_BY_PATH_SQL: &str = r"
SELECT
    note_id,
    current_path,
    title,
    content_hash,
    body_markdown,
    frontmatter_json,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision
FROM notes
WHERE current_path = ?1 AND is_deleted = 0
LIMIT 1
";
const LIST_ALL_NOTES_SQL: &str = r"
SELECT
    note_id,
    current_path,
    title,
    content_hash,
    body_markdown,
    frontmatter_json,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision
FROM notes
ORDER BY server_revision ASC, note_id ASC
";
const LIST_NOTES_SINCE_REVISION_SQL: &str = r"
SELECT
    note_id,
    current_path,
    title,
    content_hash,
    body_markdown,
    frontmatter_json,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision
FROM notes
WHERE server_revision > ?1
ORDER BY server_revision ASC, note_id ASC
";
const DELETE_NOTE_HARD_SQL: &str = "DELETE FROM notes WHERE note_id = ?1";
const EXISTS_NOTE_SQL: &str = "SELECT 1 FROM notes WHERE note_id = ?1 LIMIT 1";

/// Persisted server-authoritative `notes` row.
pub struct NoteRow {
    /// Stable note identifier.
    pub note_id: String,
    /// Current canonical vault path.
    pub current_path: String,
    /// Effective note title.
    pub title: String,
    /// Content hash for the current snapshot.
    pub content_hash: String,
    /// Full markdown body stored by the server.
    pub body_markdown: String,
    /// Raw frontmatter JSON payload.
    pub frontmatter_json: String,
    /// First known creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Trash timestamp when the note is trashed.
    pub trashed_at: Option<DateTime<Utc>>,
    /// Deletion timestamp when the note is soft-deleted.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Whether the note is currently trashed.
    pub is_trashed: bool,
    /// Whether the note is currently soft-deleted.
    pub is_deleted: bool,
    /// Device id that last edited the note, when known.
    pub last_editor_device_id: Option<String>,
    /// Current server-side revision counter for the note.
    pub server_revision: i64,
}

/// Stateless repository over the server `notes` table.
pub struct NotesRepo;

impl NotesRepo {
    /// Inserts one note row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] when `note_id` already exists, or
    /// [`ServerError`] if the insert fails.
    pub fn create(conn: &Connection, row: &NoteRow) -> ServerResult<()> {
        if note_exists(conn, &row.note_id)? {
            return Err(ServerError::Conflict("note already exists".to_string()));
        }

        let created_at = encode_timestamp(row.created_at);
        let updated_at = encode_timestamp(row.updated_at);
        let trashed_at = encode_optional_timestamp(row.trashed_at);
        let deleted_at = encode_optional_timestamp(row.deleted_at);

        conn.execute(
            CREATE_NOTE_SQL,
            rusqlite::params![
                &row.note_id,
                &row.current_path,
                &row.title,
                &row.content_hash,
                &row.body_markdown,
                &row.frontmatter_json,
                &created_at,
                &updated_at,
                trashed_at.as_deref(),
                deleted_at.as_deref(),
                row.is_trashed,
                row.is_deleted,
                row.last_editor_device_id.as_deref(),
                row.server_revision,
            ],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }

    /// Updates one note row by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the update fails.
    pub fn update(conn: &Connection, row: &NoteRow) -> ServerResult<()> {
        let created_at = encode_timestamp(row.created_at);
        let updated_at = encode_timestamp(row.updated_at);
        let trashed_at = encode_optional_timestamp(row.trashed_at);
        let deleted_at = encode_optional_timestamp(row.deleted_at);

        conn.execute(
            UPDATE_NOTE_SQL,
            rusqlite::params![
                &row.note_id,
                &row.current_path,
                &row.title,
                &row.content_hash,
                &row.body_markdown,
                &row.frontmatter_json,
                &created_at,
                &updated_at,
                trashed_at.as_deref(),
                deleted_at.as_deref(),
                row.is_trashed,
                row.is_deleted,
                row.last_editor_device_id.as_deref(),
                row.server_revision,
            ],
        )
        .map(|_| ())
        .map_err(ServerError::Database)
    }

    /// Fetches one note row by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get(conn: &Connection, note_id: &str) -> ServerResult<Option<NoteRow>> {
        let mut statement = conn
            .prepare_cached(GET_NOTE_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([note_id], decode_note_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Fetches one live note row by `current_path`.
    ///
    /// Soft-deleted rows are excluded so callers can reuse purged paths without
    /// ambiguity.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get_by_path(conn: &Connection, path: &str) -> ServerResult<Option<NoteRow>> {
        let mut statement = conn
            .prepare_cached(GET_NOTE_BY_PATH_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([path], decode_note_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Lists all note rows in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list_all(conn: &Connection) -> ServerResult<Vec<NoteRow>> {
        list_notes(conn, LIST_ALL_NOTES_SQL, None)
    }

    /// Lists note rows whose `server_revision` is greater than `min_rev`.
    ///
    /// Rows are ordered by `server_revision` ascending, then `note_id`
    /// ascending for deterministic ties.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list_since_revision(conn: &Connection, min_rev: i64) -> ServerResult<Vec<NoteRow>> {
        list_notes(conn, LIST_NOTES_SINCE_REVISION_SQL, Some(min_rev))
    }

    /// Permanently deletes one note row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete fails.
    pub fn delete_hard(conn: &Connection, note_id: &str) -> ServerResult<()> {
        conn.execute(DELETE_NOTE_HARD_SQL, [note_id])
            .map(|_| ())
            .map_err(ServerError::Database)
    }
}

fn note_exists(conn: &Connection, note_id: &str) -> ServerResult<bool> {
    let mut statement = conn
        .prepare_cached(EXISTS_NOTE_SQL)
        .map_err(ServerError::Database)?;

    statement
        .query_row([note_id], |_| Ok(()))
        .optional()
        .map(|maybe_row| maybe_row.is_some())
        .map_err(ServerError::Database)
}

fn list_notes(conn: &Connection, sql: &str, min_rev: Option<i64>) -> ServerResult<Vec<NoteRow>> {
    let mut statement = conn.prepare(sql).map_err(ServerError::Database)?;
    let rows = match min_rev {
        Some(min_rev) => statement
            .query_map([min_rev], decode_note_row)
            .map_err(ServerError::Database)?,
        None => statement
            .query_map([], decode_note_row)
            .map_err(ServerError::Database)?,
    };

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(ServerError::Database)
}

fn decode_note_row(row: &Row<'_>) -> rusqlite::Result<NoteRow> {
    Ok(NoteRow {
        note_id: row.get(0)?,
        current_path: row.get(1)?,
        title: row.get(2)?,
        content_hash: row.get(3)?,
        body_markdown: row.get(4)?,
        frontmatter_json: row.get(5)?,
        created_at: decode_timestamp(row, 6)?,
        updated_at: decode_timestamp(row, 7)?,
        trashed_at: decode_optional_timestamp(row, 8)?,
        deleted_at: decode_optional_timestamp(row, 9)?,
        is_trashed: row.get(10)?,
        is_deleted: row.get(11)?,
        last_editor_device_id: row.get(12)?,
        server_revision: row.get(13)?,
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
