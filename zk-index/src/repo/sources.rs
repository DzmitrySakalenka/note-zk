use rusqlite::Connection;
use zk_domain::id::NoteId;

use crate::error::IndexError;

const DELETE_SOURCES_FOR_NOTE: &str = "DELETE FROM note_sources WHERE note_id = ?1";
const INSERT_SOURCE_FOR_NOTE: &str =
    "INSERT INTO note_sources (note_id, source_note_id) VALUES (?1, ?2)";
const SELECT_SOURCES_FOR_NOTE: &str = r"
SELECT source_note_id
FROM note_sources
WHERE note_id = ?1
ORDER BY rowid ASC
";
const SELECT_PAGES_FOR_SOURCE: &str = r"
SELECT note_id
FROM note_sources
WHERE source_note_id = ?1
ORDER BY rowid ASC
";
const SAVEPOINT_REPLACE_SOURCES: &str = "sources_repo_replace_for_note";

/// Repository for wiki provenance source rows.
pub struct SourcesRepo;

impl SourcesRepo {
    /// Replaces all source-note rows for one wiki page.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the savepoint, delete, or insert steps fail.
    pub fn replace_for_note(
        conn: &Connection,
        note_id: &NoteId,
        sources: &[NoteId],
    ) -> Result<(), IndexError> {
        with_savepoint(
            conn,
            SAVEPOINT_REPLACE_SOURCES,
            "replace note sources",
            |conn| {
                Self::delete_for_note(conn, note_id)?;

                let mut statement =
                    conn.prepare(INSERT_SOURCE_FOR_NOTE)
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "prepare insert note source".to_string(),
                        })?;
                for source_note_id in sources {
                    statement
                        .execute(rusqlite::params![note_id.as_str(), source_note_id.as_str()])
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "insert note source".to_string(),
                        })?;
                }

                Ok(())
            },
        )
    }

    /// Fetches source-note IDs for one wiki page in insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails.
    pub fn list_sources_for_note(
        conn: &Connection,
        note_id: &NoteId,
    ) -> Result<Vec<NoteId>, IndexError> {
        list_note_ids(
            conn,
            SELECT_SOURCES_FOR_NOTE,
            note_id.as_str(),
            "list sources for note",
        )
    }

    /// Fetches wiki pages that reference one source note in insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails.
    pub fn list_pages_using_source(
        conn: &Connection,
        source_note_id: &NoteId,
    ) -> Result<Vec<NoteId>, IndexError> {
        list_note_ids(
            conn,
            SELECT_PAGES_FOR_SOURCE,
            source_note_id.as_str(),
            "list pages using source",
        )
    }

    /// Deletes all source-note rows for one wiki page.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the delete fails.
    pub fn delete_for_note(conn: &Connection, note_id: &NoteId) -> Result<(), IndexError> {
        conn.execute(DELETE_SOURCES_FOR_NOTE, [note_id.as_str()])
            .map(|_| ())
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "delete note sources".to_string(),
            })
    }
}

fn list_note_ids(
    conn: &Connection,
    sql: &str,
    value: &str,
    context: &str,
) -> Result<Vec<NoteId>, IndexError> {
    let mut statement = conn.prepare(sql).map_err(|source| IndexError::Sqlite {
        source,
        context: format!("prepare {context} query"),
    })?;
    let rows = statement
        .query_map([value], |row| row.get::<_, String>(0))
        .map_err(|source| IndexError::Sqlite {
            source,
            context: context.to_string(),
        })?;

    rows.map(|row| row.map(|value| NoteId::new(&value)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: format!("decode {context}"),
        })
}

fn with_savepoint<T, F>(
    conn: &Connection,
    savepoint_name: &str,
    context: &str,
    operation: F,
) -> Result<T, IndexError>
where
    F: FnOnce(&Connection) -> Result<T, IndexError>,
{
    conn.execute_batch(&format!("SAVEPOINT {savepoint_name}"))
        .map_err(|source| IndexError::Sqlite {
            source,
            context: format!("begin {context} savepoint"),
        })?;

    let result = operation(conn);
    match result {
        Ok(value) => {
            conn.execute_batch(&format!("RELEASE SAVEPOINT {savepoint_name}"))
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: format!("release {context} savepoint"),
                })?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {savepoint_name}; RELEASE SAVEPOINT {savepoint_name};"
            ));
            Err(error)
        }
    }
}
