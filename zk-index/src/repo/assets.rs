use rusqlite::{Connection, OptionalExtension, types::Type};
use zk_domain::{
    asset::AssetRecord,
    derived::AssetReference,
    id::{AssetId, ContentHash, NoteId, RelativePath, ServerRevision},
};

use crate::error::IndexError;

const DEFAULT_SERVER_REVISION: i64 = 0;
const INSERT_OR_REPLACE_ASSET: &str = r"
INSERT OR REPLACE INTO assets_index (
    asset_id,
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
    server_revision
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
";
const DELETE_ASSET_BY_ID: &str = "DELETE FROM assets_index WHERE asset_id = ?1";
const SELECT_ASSET_BY_ID: &str = r"
SELECT
    asset_id,
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
    server_revision
FROM assets_index
WHERE asset_id = ?1
";
const SELECT_ASSET_BY_PATH: &str = r"
SELECT
    asset_id,
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
    server_revision
FROM assets_index
WHERE current_path = ?1
";
const SELECT_ASSET_BY_CONTENT_HASH: &str = r"
SELECT
    asset_id,
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
    server_revision
FROM assets_index
WHERE content_hash = ?1
ORDER BY current_path ASC
LIMIT 1
";
const LIST_ASSETS_SQL: &str = r"
SELECT
    asset_id,
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
    server_revision
FROM assets_index
ORDER BY current_path ASC
";
const DELETE_ASSET_REFERENCES_FOR_SOURCE: &str =
    "DELETE FROM asset_references WHERE source_note_id = ?1";
const INSERT_ASSET_REFERENCE: &str = r"
INSERT INTO asset_references (
    source_note_id,
    raw_path,
    target_asset_id
) VALUES (?1, ?2, ?3)
";
const SELECT_ASSET_REFERENCES_FOR_SOURCE: &str = r"
SELECT
    source_note_id,
    raw_path,
    target_asset_id
FROM asset_references
WHERE source_note_id = ?1
ORDER BY rowid ASC
";
const SAVEPOINT_REPLACE_ASSET_REFERENCES: &str = "asset_repo_replace_asset_references";

pub struct AssetRepo;

impl AssetRepo {
    /// Inserts or replaces one asset row by `asset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if conversion or `SQLite` persistence fails.
    pub fn upsert_asset(conn: &Connection, record: &AssetRecord) -> Result<(), IndexError> {
        let size_bytes = encode_size_bytes(record.size_bytes)?;

        conn.execute(
            INSERT_OR_REPLACE_ASSET,
            rusqlite::params![
                record.asset_id.as_str(),
                record.current_path.as_str(),
                record.content_hash.as_str(),
                record.mime_type,
                size_bytes,
                record.created_at,
                record.updated_at,
                record.trashed_at,
                record.deleted_at,
                i64::from(record.is_trashed),
                i64::from(record.is_deleted),
                record.server_revision.value(),
            ],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "upsert asset".to_string(),
        })
    }

    /// Deletes one asset row by `asset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the delete fails.
    pub fn delete_asset(conn: &Connection, asset_id: &AssetId) -> Result<(), IndexError> {
        conn.execute(DELETE_ASSET_BY_ID, [asset_id.as_str()])
            .map(|_| ())
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "delete asset".to_string(),
            })
    }

    /// Fetches one asset row by `asset_id`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_asset(
        conn: &Connection,
        asset_id: &AssetId,
    ) -> Result<Option<AssetRecord>, IndexError> {
        conn.query_row(SELECT_ASSET_BY_ID, [asset_id.as_str()], decode_asset_record)
            .optional()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "get asset by id".to_string(),
            })
    }

    /// Fetches one asset row by current vault-relative path.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_asset_by_path(
        conn: &Connection,
        path: &RelativePath,
    ) -> Result<Option<AssetRecord>, IndexError> {
        conn.query_row(SELECT_ASSET_BY_PATH, [path.as_str()], decode_asset_record)
            .optional()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "get asset by path".to_string(),
            })
    }

    /// Fetches one asset row by content hash using deterministic path ordering.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn find_asset_by_content_hash(
        conn: &Connection,
        hash: &ContentHash,
    ) -> Result<Option<AssetRecord>, IndexError> {
        conn.query_row(
            SELECT_ASSET_BY_CONTENT_HASH,
            [hash.as_str()],
            decode_asset_record,
        )
        .optional()
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "find asset by content hash".to_string(),
        })
    }

    /// Lists all asset rows in deterministic path order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn list_assets(conn: &Connection) -> Result<Vec<AssetRecord>, IndexError> {
        let mut statement = conn
            .prepare(LIST_ASSETS_SQL)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "prepare list assets query".to_string(),
            })?;
        let rows = statement
            .query_map([], decode_asset_record)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "list assets".to_string(),
            })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode listed assets".to_string(),
            })
    }

    /// Replaces the full asset-reference set for one source note atomically.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the savepoint, delete, or insert steps fail.
    pub fn replace_asset_references(
        conn: &Connection,
        source_note_id: &NoteId,
        refs: &[AssetReference],
    ) -> Result<(), IndexError> {
        with_savepoint(
            conn,
            SAVEPOINT_REPLACE_ASSET_REFERENCES,
            "replace asset references",
            |conn| {
                conn.execute(
                    DELETE_ASSET_REFERENCES_FOR_SOURCE,
                    [source_note_id.as_str()],
                )
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: "delete asset references".to_string(),
                })?;

                let mut statement =
                    conn.prepare(INSERT_ASSET_REFERENCE)
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "prepare insert asset reference".to_string(),
                        })?;
                for asset_reference in refs {
                    statement
                        .execute(rusqlite::params![
                            asset_reference.source_note_id.as_str(),
                            asset_reference.raw_path,
                            asset_reference
                                .target_asset_id
                                .as_ref()
                                .map(AssetId::as_str),
                        ])
                        .map_err(|source| IndexError::Sqlite {
                            source,
                            context: "insert asset reference".to_string(),
                        })?;
                }

                Ok(())
            },
        )
    }

    /// Fetches outbound asset references for one source note in insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn get_asset_references(
        conn: &Connection,
        source_note_id: &NoteId,
    ) -> Result<Vec<AssetReference>, IndexError> {
        let mut statement = conn
            .prepare(SELECT_ASSET_REFERENCES_FOR_SOURCE)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "prepare get asset references query".to_string(),
            })?;
        let rows = statement
            .query_map([source_note_id.as_str()], decode_asset_reference)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "get asset references".to_string(),
            })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode get asset references rows".to_string(),
            })
    }
}

fn decode_asset_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetRecord> {
    let asset_id = row.get::<_, String>(0)?;
    let current_path = row.get::<_, String>(1)?;
    let content_hash = row.get::<_, String>(2)?;
    let raw_size_bytes = row.get::<_, i64>(4)?;
    let server_revision = row
        .get::<_, Option<i64>>(11)?
        .unwrap_or(DEFAULT_SERVER_REVISION);

    Ok(AssetRecord {
        asset_id: AssetId::from(&asset_id),
        current_path: RelativePath::new(&current_path),
        content_hash: ContentHash::from(&content_hash),
        mime_type: row.get(3)?,
        size_bytes: decode_size_bytes(raw_size_bytes)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        trashed_at: row.get(7)?,
        deleted_at: row.get(8)?,
        is_trashed: row.get::<_, i64>(9)? != 0,
        is_deleted: row.get::<_, i64>(10)? != 0,
        server_revision: ServerRevision::new(server_revision),
    })
}

fn decode_asset_reference(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetReference> {
    let source_note_id = row.get::<_, String>(0)?;
    let target_asset_id = row.get::<_, Option<String>>(2)?;

    Ok(AssetReference {
        source_note_id: NoteId::from(&source_note_id),
        raw_path: row.get(1)?,
        target_asset_id: target_asset_id.as_deref().map(AssetId::from),
    })
}

fn encode_size_bytes(size_bytes: u64) -> Result<i64, IndexError> {
    i64::try_from(size_bytes).map_err(|source| IndexError::Sqlite {
        source: rusqlite::Error::ToSqlConversionFailure(Box::new(source)),
        context: "convert asset size for upsert".to_string(),
    })
}

fn decode_size_bytes(size_bytes: i64) -> rusqlite::Result<u64> {
    u64::try_from(size_bytes).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(4, Type::Integer, Box::new(source))
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
