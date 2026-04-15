use rusqlite::Connection;
use zk_domain::{
    derived::{ExternalLink, ResolvedNoteLink},
    id::NoteId,
};

use crate::error::IndexError;

const DELETE_NOTE_LINKS_FOR_SOURCE: &str = "DELETE FROM note_links WHERE source_note_id = ?1";
const INSERT_NOTE_LINK: &str = r"
INSERT INTO note_links (
    source_note_id,
    raw_target,
    target_note_id,
    link_text
) VALUES (?1, ?2, ?3, ?4)
";
const DELETE_EXTERNAL_LINKS_FOR_SOURCE: &str =
    "DELETE FROM external_links WHERE source_note_id = ?1";
const INSERT_EXTERNAL_LINK: &str = r"
INSERT INTO external_links (
    source_note_id,
    url,
    link_text
) VALUES (?1, ?2, ?3)
";
const SELECT_NOTE_LINKS_FOR_SOURCE: &str = r"
SELECT
    source_note_id,
    raw_target,
    target_note_id,
    link_text
FROM note_links
WHERE source_note_id = ?1
ORDER BY rowid ASC
";
const SELECT_EXTERNAL_LINKS_FOR_SOURCE: &str = r"
SELECT
    source_note_id,
    url,
    link_text
FROM external_links
WHERE source_note_id = ?1
ORDER BY rowid ASC
";
const SELECT_BACKLINKS_FOR_TARGET: &str = r"
SELECT
    source_note_id,
    raw_target,
    target_note_id,
    link_text
FROM note_links
WHERE target_note_id = ?1
ORDER BY rowid ASC
";
const SAVEPOINT_REPLACE_NOTE_LINKS: &str = "link_repo_replace_note_links";
const SAVEPOINT_REPLACE_EXTERNAL_LINKS: &str = "link_repo_replace_external_links";

pub struct LinkRepo;

impl LinkRepo {
    /// Replaces all outbound note links for one source note atomically.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the savepoint, delete, or insert steps fail.
    pub fn replace_note_links(
        conn: &Connection,
        source_note_id: &NoteId,
        links: &[ResolvedNoteLink],
    ) -> Result<(), IndexError> {
        with_savepoint(
            conn,
            SAVEPOINT_REPLACE_NOTE_LINKS,
            "replace note links",
            |conn| {
                conn.execute(DELETE_NOTE_LINKS_FOR_SOURCE, [source_note_id.as_str()])
                    .map_err(|source| IndexError::Sqlite {
                        source,
                        context: "delete note links".to_string(),
                    })?;

                let mut statement =
                    conn.prepare(INSERT_NOTE_LINK)
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "prepare insert note link".to_string(),
                        })?;
                for link in links {
                    statement
                        .execute(rusqlite::params![
                            link.source_note_id.as_str(),
                            link.raw_target,
                            link.target_note_id.as_ref().map(NoteId::as_str),
                            link.link_text,
                        ])
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "insert note link".to_string(),
                        })?;
                }

                Ok(())
            },
        )
    }

    /// Replaces all outbound external links for one source note atomically.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the savepoint, delete, or insert steps fail.
    pub fn replace_external_links(
        conn: &Connection,
        source_note_id: &NoteId,
        links: &[ExternalLink],
    ) -> Result<(), IndexError> {
        with_savepoint(
            conn,
            SAVEPOINT_REPLACE_EXTERNAL_LINKS,
            "replace external links",
            |conn| {
                conn.execute(DELETE_EXTERNAL_LINKS_FOR_SOURCE, [source_note_id.as_str()])
                    .map_err(|source| IndexError::Sqlite {
                        source,
                        context: "delete external links".to_string(),
                    })?;

                let mut statement =
                    conn.prepare(INSERT_EXTERNAL_LINK)
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "prepare insert external link".to_string(),
                        })?;
                for link in links {
                    statement
                        .execute(rusqlite::params![
                            link.source_note_id.as_str(),
                            link.url,
                            link.link_text,
                        ])
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "insert external link".to_string(),
                        })?;
                }

                Ok(())
            },
        )
    }

    /// Fetches outbound note links for one source note in insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_note_links(
        conn: &Connection,
        source_note_id: &NoteId,
    ) -> Result<Vec<ResolvedNoteLink>, IndexError> {
        list_note_links(
            conn,
            SELECT_NOTE_LINKS_FOR_SOURCE,
            source_note_id.as_str(),
            "get note links",
        )
    }

    /// Fetches outbound external links for one source note in insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_external_links(
        conn: &Connection,
        source_note_id: &NoteId,
    ) -> Result<Vec<ExternalLink>, IndexError> {
        list_external_links(
            conn,
            SELECT_EXTERNAL_LINKS_FOR_SOURCE,
            source_note_id.as_str(),
            "get external links",
        )
    }

    /// Fetches inbound note links that resolve to one target note.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_backlinks(
        conn: &Connection,
        target_note_id: &NoteId,
    ) -> Result<Vec<ResolvedNoteLink>, IndexError> {
        list_note_links(
            conn,
            SELECT_BACKLINKS_FOR_TARGET,
            target_note_id.as_str(),
            "get backlinks",
        )
    }
}

fn decode_note_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<ResolvedNoteLink> {
    let source_note_id = row.get::<_, String>(0)?;
    let target_note_id = row.get::<_, Option<String>>(2)?;

    Ok(ResolvedNoteLink {
        source_note_id: NoteId::from(&source_note_id),
        raw_target: row.get(1)?,
        target_note_id: target_note_id.as_deref().map(NoteId::from),
        link_text: row.get(3)?,
        is_ambiguous: false,
    })
}

fn decode_external_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExternalLink> {
    let source_note_id = row.get::<_, String>(0)?;

    Ok(ExternalLink {
        source_note_id: NoteId::from(&source_note_id),
        url: row.get(1)?,
        link_text: row.get(2)?,
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

fn list_note_links(
    conn: &Connection,
    query: &str,
    note_id: &str,
    context: &str,
) -> Result<Vec<ResolvedNoteLink>, IndexError> {
    let mut statement = conn.prepare(query).map_err(|source| IndexError::Sqlite {
        source,
        context: format!("prepare {context} query"),
    })?;
    let rows = statement
        .query_map([note_id], decode_note_link)
        .map_err(|source| IndexError::Sqlite {
            source,
            context: context.to_string(),
        })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: format!("decode {context} rows"),
        })
}

fn list_external_links(
    conn: &Connection,
    query: &str,
    note_id: &str,
    context: &str,
) -> Result<Vec<ExternalLink>, IndexError> {
    let mut statement = conn.prepare(query).map_err(|source| IndexError::Sqlite {
        source,
        context: format!("prepare {context} query"),
    })?;
    let rows = statement
        .query_map([note_id], decode_external_link)
        .map_err(|source| IndexError::Sqlite {
            source,
            context: context.to_string(),
        })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: format!("decode {context} rows"),
        })
}
