use rusqlite::{Connection, OptionalExtension, Row};
use zk_domain::{
    ChunkId, ChunkRecord,
    id::{ContentHash, NoteId},
};

use super::timestamps::{decode_timestamp, encode_timestamp};
use crate::{ServerError, ServerResult};

const DELETE_CHUNKS_FOR_NOTE_SQL: &str = "DELETE FROM note_chunks WHERE note_id = ?1";
const INSERT_CHUNK_SQL: &str = r"
INSERT INTO note_chunks (
    chunk_id,
    note_id,
    ordinal,
    content_hash,
    text,
    token_count,
    created_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7
)
";
const LIST_CHUNKS_FOR_NOTE_SQL: &str = r"
SELECT
    chunk_id,
    note_id,
    ordinal,
    content_hash,
    text,
    token_count,
    created_at
FROM note_chunks
WHERE note_id = ?1
ORDER BY ordinal ASC
";
const GET_CHUNK_SQL: &str = r"
SELECT
    chunk_id,
    note_id,
    ordinal,
    content_hash,
    text,
    token_count,
    created_at
FROM note_chunks
WHERE chunk_id = ?1
";

/// Repository over the server `note_chunks` table.
pub struct ChunksRepo<'a> {
    conn: &'a Connection,
}

impl<'a> ChunksRepo<'a> {
    /// Creates a repository backed by one `SQLite` connection.
    #[must_use]
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Replaces all chunks for one note and returns the inserted row count.
    ///
    /// The caller owns transaction scope. Pass a transaction-backed connection
    /// when delete-and-insert must commit atomically with other writes.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete or any insert fails.
    pub fn replace_for_note(&self, note_id: &NoteId, chunks: &[ChunkRecord]) -> ServerResult<u64> {
        self.conn
            .execute(DELETE_CHUNKS_FOR_NOTE_SQL, [note_id.as_str()])
            .map_err(ServerError::Database)?;

        let mut statement = self
            .conn
            .prepare_cached(INSERT_CHUNK_SQL)
            .map_err(ServerError::Database)?;
        let mut inserted = 0_u64;

        for chunk in chunks {
            let created_at = encode_timestamp(chunk.created_at);
            let token_count = chunk.token_count.map(i64::from);

            statement
                .execute(rusqlite::params![
                    chunk.chunk_id.as_str(),
                    chunk.note_id.as_str(),
                    i64::from(chunk.ordinal),
                    chunk.content_hash.as_str(),
                    &chunk.text,
                    token_count,
                    created_at,
                ])
                .map_err(ServerError::Database)?;
            inserted += 1;
        }

        Ok(inserted)
    }

    /// Lists chunks for one note ordered by `ordinal`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn list_for_note(&self, note_id: &NoteId) -> ServerResult<Vec<ChunkRecord>> {
        let mut statement = self
            .conn
            .prepare_cached(LIST_CHUNKS_FOR_NOTE_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([note_id.as_str()], decode_chunk_record)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Deletes all chunks for one note and returns the deleted row count.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete fails.
    pub fn delete_for_note(&self, note_id: &NoteId) -> ServerResult<u64> {
        self.conn
            .execute(DELETE_CHUNKS_FOR_NOTE_SQL, [note_id.as_str()])
            .map(rows_changed_to_u64)
            .map_err(ServerError::Database)
    }

    /// Fetches one chunk by `chunk_id`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the query fails or row decoding is invalid.
    pub fn get(&self, chunk_id: &ChunkId) -> ServerResult<Option<ChunkRecord>> {
        let mut statement = self
            .conn
            .prepare_cached(GET_CHUNK_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([chunk_id.as_str()], decode_chunk_record)
            .optional()
            .map_err(ServerError::Database)
    }
}

fn decode_chunk_record(row: &Row<'_>) -> rusqlite::Result<ChunkRecord> {
    let chunk_id = row.get::<_, String>(0)?;
    let note_id = row.get::<_, String>(1)?;
    let content_hash = row.get::<_, String>(3)?;

    Ok(ChunkRecord {
        chunk_id: ChunkId::new(&chunk_id),
        note_id: NoteId::new(&note_id),
        ordinal: decode_u32(row, 2)?,
        content_hash: ContentHash::new(&content_hash),
        text: row.get(4)?,
        token_count: decode_optional_u32(row, 5)?,
        created_at: decode_timestamp(row, 6)?,
    })
}

fn decode_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn decode_optional_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u32>> {
    let Some(value) = row.get::<_, Option<i64>>(index)? else {
        return Ok(None);
    };

    u32::try_from(value)
        .map(Some)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn rows_changed_to_u64(rows_changed: usize) -> u64 {
    u64::try_from(rows_changed).expect("row count should fit u64")
}
