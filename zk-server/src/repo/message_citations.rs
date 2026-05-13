use rusqlite::{Connection, Row};
use zk_domain::{ChunkId, Citation, MessageId, id::NoteId};

use crate::{ServerError, ServerResult};

const DELETE_CITATIONS_FOR_MESSAGE_SQL: &str =
    "DELETE FROM message_citations WHERE message_id = ?1";
const INSERT_CITATION_SQL: &str = r"
INSERT INTO message_citations (
    message_id,
    ordinal,
    note_id,
    chunk_id,
    similarity,
    snippet
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6
)
";
const LIST_CITATIONS_FOR_MESSAGE_SQL: &str = r"
SELECT
    note_id,
    chunk_id,
    similarity,
    snippet,
    ordinal
FROM message_citations
WHERE message_id = ?1 AND chunk_id IS NOT NULL
ORDER BY ordinal ASC
";
const LIST_CITATIONS_FOR_MESSAGES_PREFIX: &str = r"
SELECT
    message_id,
    note_id,
    chunk_id,
    similarity,
    snippet,
    ordinal
FROM message_citations
WHERE chunk_id IS NOT NULL
  AND message_id IN (
";
const LIST_CITATIONS_FOR_MESSAGES_SUFFIX: &str = r"
)
ORDER BY message_id ASC, ordinal ASC
";

/// Repository over the server `message_citations` table.
pub struct MessageCitationsRepo;

impl MessageCitationsRepo {
    /// Replaces all citations for one message in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the delete, insert, or commit fails.
    pub fn replace_for_message(
        conn: &Connection,
        message_id: &MessageId,
        citations: &[Citation],
    ) -> ServerResult<()> {
        let transaction = conn
            .unchecked_transaction()
            .map_err(ServerError::Database)?;
        transaction
            .execute(DELETE_CITATIONS_FOR_MESSAGE_SQL, [message_id.as_str()])
            .map_err(ServerError::Database)?;

        {
            let mut statement = transaction
                .prepare_cached(INSERT_CITATION_SQL)
                .map_err(ServerError::Database)?;

            for citation in citations {
                statement
                    .execute(rusqlite::params![
                        message_id.as_str(),
                        i64::from(citation.ordinal),
                        citation.note_id.as_str(),
                        citation.chunk_id.as_str(),
                        f64::from(citation.similarity),
                        &citation.snippet,
                    ])
                    .map_err(ServerError::Database)?;
            }
        }

        transaction.commit().map_err(ServerError::Database)
    }

    /// Lists citations for one message ordered by ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the query fails or row decoding is invalid.
    pub fn list_for_message(
        conn: &Connection,
        message_id: &MessageId,
    ) -> ServerResult<Vec<Citation>> {
        let mut statement = conn
            .prepare_cached(LIST_CITATIONS_FOR_MESSAGE_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([message_id.as_str()], decode_citation_row)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Lists citations for multiple messages ordered by message id and ordinal.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the query fails or row decoding is invalid.
    pub fn list_for_messages(
        conn: &Connection,
        message_ids: &[MessageId],
    ) -> ServerResult<Vec<(MessageId, Citation)>> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = vec!["?"; message_ids.len()].join(", ");
        let sql = format!(
            "{LIST_CITATIONS_FOR_MESSAGES_PREFIX}{placeholders}{LIST_CITATIONS_FOR_MESSAGES_SUFFIX}"
        );
        let params = message_ids.iter().map(MessageId::as_str);
        let mut statement = conn.prepare(&sql).map_err(ServerError::Database)?;
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(params),
                decode_message_citation_row,
            )
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Deletes all citations for one message.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the delete fails.
    pub fn delete_for_message(conn: &Connection, message_id: &MessageId) -> ServerResult<()> {
        conn.execute(DELETE_CITATIONS_FOR_MESSAGE_SQL, [message_id.as_str()])
            .map(|_| ())
            .map_err(ServerError::Database)
    }
}

fn decode_citation_row(row: &Row<'_>) -> rusqlite::Result<Citation> {
    Ok(Citation {
        note_id: NoteId::new(&row.get::<_, String>(0)?),
        chunk_id: ChunkId::new(&row.get::<_, String>(1)?),
        similarity: decode_f32(row, 2)?,
        snippet: row.get(3)?,
        ordinal: decode_u32(row, 4)?,
    })
}

fn decode_message_citation_row(row: &Row<'_>) -> rusqlite::Result<(MessageId, Citation)> {
    Ok((
        MessageId::from(row.get::<_, String>(0)?),
        Citation {
            note_id: NoteId::new(&row.get::<_, String>(1)?),
            chunk_id: ChunkId::new(&row.get::<_, String>(2)?),
            similarity: decode_f32(row, 3)?,
            snippet: row.get(4)?,
            ordinal: decode_u32(row, 5)?,
        },
    ))
}

fn decode_f32(row: &Row<'_>, index: usize) -> rusqlite::Result<f32> {
    let value = row.get::<_, f64>(index)?;
    Ok(value as f32)
}

fn decode_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
