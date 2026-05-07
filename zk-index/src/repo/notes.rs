use std::sync::LazyLock;

use rusqlite::{Connection, OptionalExtension};
use zk_domain::{
    id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
};

use crate::error::IndexError;

const DEFAULT_SERVER_REVISION: i64 = 0;
const NOTE_COLUMNS: &str = r"    note_id,
    current_path,
    title,
    content_hash,
    frontmatter_json,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision";
const NOTE_RECORD_COLUMNS: &str = r"    note_id,
    current_path,
    content_hash,
    created_at,
    updated_at,
    trashed_at,
    deleted_at,
    is_trashed,
    is_deleted,
    last_editor_device_id,
    server_revision";
const MARK_NOTE_DELETED: &str = r"
UPDATE notes_index
SET
    trashed_at = NULL,
    deleted_at = ?2,
    is_trashed = 0,
    is_deleted = 1
WHERE note_id = ?1
";
const MARK_NOTE_DELETED_WITH_SERVER_REVISION: &str = r"
UPDATE notes_index
SET
    trashed_at = NULL,
    deleted_at = ?2,
    is_trashed = 0,
    is_deleted = 1,
    server_revision = ?3
WHERE note_id = ?1
";
static INSERT_OR_UPDATE_NOTE: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
INSERT INTO notes_index (
{NOTE_COLUMNS}
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, 0, NULL, 0
)
ON CONFLICT(note_id) DO UPDATE SET
    current_path = excluded.current_path,
    title = excluded.title,
    content_hash = excluded.content_hash,
    frontmatter_json = excluded.frontmatter_json,
    updated_at = excluded.updated_at,
    trashed_at = excluded.trashed_at,
    is_trashed = excluded.is_trashed
"
    )
});
static INSERT_OR_UPDATE_NOTE_WITH_SERVER_REVISION: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
INSERT INTO notes_index (
{NOTE_COLUMNS}
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, 0, NULL, ?10
)
ON CONFLICT(note_id) DO UPDATE SET
    current_path = excluded.current_path,
    title = excluded.title,
    content_hash = excluded.content_hash,
    frontmatter_json = excluded.frontmatter_json,
    updated_at = excluded.updated_at,
    trashed_at = excluded.trashed_at,
    is_trashed = excluded.is_trashed,
    is_deleted = excluded.is_deleted,
    server_revision = excluded.server_revision
"
    )
});
const DELETE_NOTE_BY_ID: &str = "DELETE FROM notes_index WHERE note_id = ?1";
static SELECT_NOTE_BY_ID: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
SELECT
{NOTE_RECORD_COLUMNS}
FROM notes_index
WHERE note_id = ?1
"
    )
});
static SELECT_NOTE_BY_PATH: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
SELECT
{NOTE_RECORD_COLUMNS}
FROM notes_index
WHERE current_path = ?1
"
    )
});
static LIST_NOTES_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        r"
SELECT
{NOTE_RECORD_COLUMNS}
FROM notes_index
ORDER BY current_path ASC
"
    )
});
const DELETE_TAGS_FOR_NOTE: &str = "DELETE FROM note_tags WHERE note_id = ?1";
const INSERT_TAG_FOR_NOTE: &str = "INSERT INTO note_tags (note_id, tag) VALUES (?1, ?2)";
const DELETE_ALIASES_FOR_NOTE: &str = "DELETE FROM note_aliases WHERE note_id = ?1";
const INSERT_ALIAS_FOR_NOTE: &str = "INSERT INTO note_aliases (note_id, alias) VALUES (?1, ?2)";
const SELECT_TAGS_FOR_NOTE: &str = "SELECT tag FROM note_tags WHERE note_id = ?1 ORDER BY tag ASC";
const SELECT_ALIASES_FOR_NOTE: &str =
    "SELECT alias FROM note_aliases WHERE note_id = ?1 ORDER BY alias ASC";
const SELECT_NOTES_BY_TAG: &str =
    "SELECT note_id FROM note_tags WHERE tag = ?1 ORDER BY note_id ASC";
const SELECT_NOTE_METADATA_BY_ID: &str =
    "SELECT title, frontmatter_json FROM notes_index WHERE note_id = ?1 LIMIT 1";
const COUNT_NOTES_SQL: &str = "SELECT COUNT(*) FROM notes_index";
const SAVEPOINT_REPLACE_TAGS: &str = "note_repo_replace_tags";
const SAVEPOINT_REPLACE_ALIASES: &str = "note_repo_replace_aliases";

/// Repository for persisted note rows and note-owned derived tables.
pub struct NoteRepo;

/// Indexed note metadata preserved alongside the main note row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteIndexedMetadata {
    /// Effective title stored in the index.
    pub title: String,
    /// Serialized frontmatter JSON stored in the index.
    pub frontmatter_json: String,
}

/// Parameters for inserting or updating one note row.
#[derive(Debug, Clone, Copy)]
pub struct UpsertNoteParams<'a> {
    /// Stable note identifier.
    pub note_id: &'a NoteId,
    /// Current vault-relative note path.
    pub path: &'a RelativePath,
    /// Effective note title stored for lookup.
    pub title: &'a str,
    /// Hash of the note file content.
    pub content_hash: &'a ContentHash,
    /// Serialized frontmatter JSON.
    pub frontmatter_json: &'a str,
    /// Note creation timestamp.
    pub created_at: &'a str,
    /// Note update timestamp.
    pub updated_at: &'a str,
    /// Timestamp when the note entered trash, if applicable.
    pub trashed_at: Option<&'a str>,
    /// Whether the current note path is inside the trash tree.
    pub is_trashed: bool,
}

impl NoteRepo {
    /// Returns the total number of note rows in `notes_index`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the `COUNT(*)` query fails.
    pub fn count(conn: &Connection) -> Result<u32, IndexError> {
        conn.query_row(COUNT_NOTES_SQL, [], |row| row.get(0))
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "count notes".to_string(),
            })
    }

    /// Inserts a new note row or updates mutable note content fields by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the `SQLite` upsert fails.
    pub fn upsert_note(conn: &Connection, params: UpsertNoteParams<'_>) -> Result<(), IndexError> {
        conn.execute(
            INSERT_OR_UPDATE_NOTE.as_str(),
            rusqlite::params![
                params.note_id.as_str(),
                params.path.as_str(),
                params.title,
                params.content_hash.as_str(),
                params.frontmatter_json,
                params.created_at,
                params.updated_at,
                params.trashed_at,
                i64::from(params.is_trashed),
            ],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "upsert note".to_string(),
        })
    }

    /// Inserts or replaces one note row while also updating `server_revision`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the `SQLite` upsert fails.
    pub fn upsert_note_with_server_revision(
        conn: &Connection,
        params: UpsertNoteParams<'_>,
        server_revision: &ServerRevision,
    ) -> Result<(), IndexError> {
        conn.execute(
            INSERT_OR_UPDATE_NOTE_WITH_SERVER_REVISION.as_str(),
            rusqlite::params![
                params.note_id.as_str(),
                params.path.as_str(),
                params.title,
                params.content_hash.as_str(),
                params.frontmatter_json,
                params.created_at,
                params.updated_at,
                params.trashed_at,
                i64::from(params.is_trashed),
                server_revision.value(),
            ],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "upsert note with server revision".to_string(),
        })
    }

    /// Deletes one note row by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the delete fails.
    pub fn delete_note(conn: &Connection, note_id: &NoteId) -> Result<(), IndexError> {
        conn.execute(DELETE_NOTE_BY_ID, [note_id.as_str()])
            .map(|_| ())
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "delete note".to_string(),
            })
    }

    /// Marks one note row as a persisted hard-delete tombstone.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the update fails.
    pub fn mark_deleted(
        conn: &Connection,
        note_id: &NoteId,
        deleted_at: &str,
    ) -> Result<(), IndexError> {
        conn.execute(
            MARK_NOTE_DELETED,
            rusqlite::params![note_id.as_str(), deleted_at],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "mark note deleted".to_string(),
        })
    }

    /// Marks one note row as deleted and refreshes its `server_revision`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the update fails.
    pub fn mark_deleted_with_server_revision(
        conn: &Connection,
        note_id: &NoteId,
        deleted_at: &str,
        server_revision: &ServerRevision,
    ) -> Result<(), IndexError> {
        conn.execute(
            MARK_NOTE_DELETED_WITH_SERVER_REVISION,
            rusqlite::params![note_id.as_str(), deleted_at, server_revision.value()],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "mark note deleted with server revision".to_string(),
        })
    }

    /// Fetches one note row by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_note(conn: &Connection, note_id: &NoteId) -> Result<Option<NoteRecord>, IndexError> {
        conn.query_row(
            SELECT_NOTE_BY_ID.as_str(),
            [note_id.as_str()],
            decode_note_record,
        )
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "get note by id".to_string(),
        })
    }

    /// Fetches indexed title/frontmatter metadata for one note by `note_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_note_metadata(
        conn: &Connection,
        note_id: &NoteId,
    ) -> Result<Option<NoteIndexedMetadata>, IndexError> {
        conn.query_row(SELECT_NOTE_METADATA_BY_ID, [note_id.as_str()], |row| {
            Ok(NoteIndexedMetadata {
                title: row.get(0)?,
                frontmatter_json: row.get(1)?,
            })
        })
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "get note indexed metadata".to_string(),
        })
    }

    /// Fetches one note row by current vault-relative path.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_note_by_path(
        conn: &Connection,
        path: &RelativePath,
    ) -> Result<Option<NoteRecord>, IndexError> {
        conn.query_row(
            SELECT_NOTE_BY_PATH.as_str(),
            [path.as_str()],
            decode_note_record,
        )
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "get note by path".to_string(),
        })
    }

    /// Lists all note rows in deterministic path order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn list_notes(conn: &Connection) -> Result<Vec<NoteRecord>, IndexError> {
        let mut statement =
            conn.prepare(LIST_NOTES_SQL.as_str())
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: "prepare list notes query".to_string(),
                })?;
        let rows = statement
            .query_map([], decode_note_record)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "list notes".to_string(),
            })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode listed notes".to_string(),
            })
    }

    /// Replaces the full tag set for one note atomically.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the savepoint, delete, or insert steps fail.
    pub fn replace_tags(
        conn: &Connection,
        note_id: &NoteId,
        tags: &[String],
    ) -> Result<(), IndexError> {
        with_savepoint(conn, SAVEPOINT_REPLACE_TAGS, "replace note tags", |conn| {
            conn.execute(DELETE_TAGS_FOR_NOTE, [note_id.as_str()])
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: "delete note tags".to_string(),
                })?;

            let mut statement =
                conn.prepare(INSERT_TAG_FOR_NOTE)
                    .map_err(|source| IndexError::Sqlite {
                        source,
                        context: "prepare insert note tag".to_string(),
                    })?;
            for tag in tags {
                statement
                    .execute(rusqlite::params![note_id.as_str(), tag])
                    .map_err(|source| IndexError::Sqlite {
                        source,
                        context: "insert note tag".to_string(),
                    })?;
            }

            Ok(())
        })
    }

    /// Replaces the full alias set for one note atomically.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the savepoint, delete, or insert steps fail.
    pub fn replace_aliases(
        conn: &Connection,
        note_id: &NoteId,
        aliases: &[String],
    ) -> Result<(), IndexError> {
        with_savepoint(
            conn,
            SAVEPOINT_REPLACE_ALIASES,
            "replace note aliases",
            |conn| {
                conn.execute(DELETE_ALIASES_FOR_NOTE, [note_id.as_str()])
                    .map_err(|source| IndexError::Sqlite {
                        source,
                        context: "delete note aliases".to_string(),
                    })?;

                let mut statement =
                    conn.prepare(INSERT_ALIAS_FOR_NOTE)
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "prepare insert note alias".to_string(),
                        })?;
                for alias in aliases {
                    statement
                        .execute(rusqlite::params![note_id.as_str(), alias])
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "insert note alias".to_string(),
                        })?;
                }

                Ok(())
            },
        )
    }

    /// Fetches all tags for one note in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails.
    pub fn get_tags(conn: &Connection, note_id: &NoteId) -> Result<Vec<String>, IndexError> {
        list_text_column(
            conn,
            SELECT_TAGS_FOR_NOTE,
            note_id.as_str(),
            "get note tags",
        )
    }

    /// Fetches all aliases for one note in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails.
    pub fn get_aliases(conn: &Connection, note_id: &NoteId) -> Result<Vec<String>, IndexError> {
        list_text_column(
            conn,
            SELECT_ALIASES_FOR_NOTE,
            note_id.as_str(),
            "get note aliases",
        )
    }

    /// Fetches note IDs that currently carry a given tag in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails.
    pub fn get_notes_by_tag(conn: &Connection, tag: &str) -> Result<Vec<NoteId>, IndexError> {
        let mut statement =
            conn.prepare(SELECT_NOTES_BY_TAG)
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: "prepare notes by tag query".to_string(),
                })?;
        let rows = statement
            .query_map([tag], |row| row.get::<_, String>(0))
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "get notes by tag".to_string(),
            })?;

        rows.map(|row| row.map(NoteId::from))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode notes by tag".to_string(),
            })
    }
}

fn decode_note_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteRecord> {
    let note_id = row.get::<_, String>(0)?;
    let current_path = row.get::<_, String>(1)?;
    let content_hash = row.get::<_, String>(2)?;
    let server_revision = row
        .get::<_, Option<i64>>(10)?
        .unwrap_or(DEFAULT_SERVER_REVISION);

    Ok(NoteRecord {
        note_id: NoteId::from(note_id),
        current_path: RelativePath::new(&current_path),
        content_hash: ContentHash::from(content_hash),
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        trashed_at: row.get(5)?,
        deleted_at: row.get(6)?,
        is_trashed: row.get::<_, i64>(7)? != 0,
        is_deleted: row.get::<_, i64>(8)? != 0,
        last_editor_device_id: row.get::<_, Option<String>>(9)?.map(DeviceId::from),
        server_revision: ServerRevision::new(server_revision),
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

fn list_text_column(
    conn: &Connection,
    query: &str,
    param: &str,
    context: &str,
) -> Result<Vec<String>, IndexError> {
    let mut statement = conn.prepare(query).map_err(|source| IndexError::Sqlite {
        source,
        context: format!("prepare {context} query"),
    })?;
    let rows = statement
        .query_map([param], |row| row.get::<_, String>(0))
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
