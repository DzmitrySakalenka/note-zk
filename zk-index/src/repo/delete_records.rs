use rusqlite::{Connection, OptionalExtension};
use zk_domain::{
    id::{RelativePath, ServerRevision},
    sync::ItemType,
    sync_state::{DeleteRecord, DeleteRecordTarget},
};

use crate::error::IndexError;

const DELETE_LOGICAL_DELETE_RECORD: &str =
    "DELETE FROM delete_records WHERE item_id = ?1 AND item_type = ?2";
const INSERT_DELETE_RECORD: &str = r"
INSERT INTO delete_records (
    item_id,
    item_type,
    original_path,
    trashed_at,
    retention_until,
    deleted_at,
    delete_revision
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
";
const SELECT_DELETE_RECORD: &str = r"
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
const LIST_EXPIRED_DELETE_RECORDS: &str = r"
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
";
const MARK_DELETE_RECORD_DELETED: &str = r"
UPDATE delete_records
SET deleted_at = ?3, delete_revision = ?4
WHERE item_id = ?1 AND item_type = ?2
";
const LIST_ALL_DELETE_RECORDS: &str = r"
SELECT
    item_id,
    item_type,
    original_path,
    trashed_at,
    retention_until,
    deleted_at,
    delete_revision
FROM delete_records
ORDER BY retention_until ASC, item_type ASC, item_id ASC
";
const SELECT_NEXT_DELETE_REVISION: &str =
    "SELECT COALESCE(MAX(delete_revision), 0) + 1 FROM delete_records";
const SAVEPOINT_INSERT_DELETE_RECORD: &str = "delete_record_repo_insert";

/// Repository for logical delete records that track trash retention state.
pub struct DeleteRecordRepo;

impl DeleteRecordRepo {
    /// Inserts one canonical delete record for the logical key `(item_id, item_type)`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion, savepoint, delete, or insert fails.
    pub fn insert_delete_record(
        conn: &Connection,
        record: &DeleteRecord,
    ) -> Result<(), IndexError> {
        let item_type = encode_item_type(record.item_type());

        with_savepoint(
            conn,
            SAVEPOINT_INSERT_DELETE_RECORD,
            "insert delete record",
            |conn| {
                conn.execute(
                    DELETE_LOGICAL_DELETE_RECORD,
                    rusqlite::params![record.item_id_str(), item_type],
                )
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: "delete existing delete record".to_string(),
                })?;

                conn.execute(
                    INSERT_DELETE_RECORD,
                    rusqlite::params![
                        record.item_id_str(),
                        item_type,
                        record.original_path.as_str(),
                        record.trashed_at,
                        record.retention_until,
                        record.deleted_at,
                        record.delete_revision.as_ref().map(ServerRevision::value),
                    ],
                )
                .map(|_| ())
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: "insert delete record".to_string(),
                })
            },
        )
    }

    /// Fetches one delete record by logical key `(item_id, item_type)`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion, query execution, or row decoding fails.
    pub fn get_delete_record(
        conn: &Connection,
        target: &DeleteRecordTarget,
    ) -> Result<Option<DeleteRecord>, IndexError> {
        let item_type = encode_item_type(match target {
            DeleteRecordTarget::Note(_) => ItemType::Note,
            DeleteRecordTarget::Asset(_) => ItemType::Asset,
        });
        let item_id = match target {
            DeleteRecordTarget::Note(note_id) => note_id.as_str(),
            DeleteRecordTarget::Asset(asset_id) => asset_id.as_str(),
        };

        let raw_row = conn
            .query_row(
                SELECT_DELETE_RECORD,
                rusqlite::params![item_id, item_type],
                decode_raw_delete_record,
            )
            .optional()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "get delete record".to_string(),
            })?;

        raw_row.map(raw_delete_record_into_domain).transpose()
    }

    /// Deletes one delete record by logical key `(item_id, item_type)`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion or delete execution fails.
    pub fn delete_delete_record(
        conn: &Connection,
        target: &DeleteRecordTarget,
    ) -> Result<(), IndexError> {
        let item_type = encode_item_type(match target {
            DeleteRecordTarget::Note(_) => ItemType::Note,
            DeleteRecordTarget::Asset(_) => ItemType::Asset,
        });
        let item_id = match target {
            DeleteRecordTarget::Note(note_id) => note_id.as_str(),
            DeleteRecordTarget::Asset(asset_id) => asset_id.as_str(),
        };

        conn.execute(
            DELETE_LOGICAL_DELETE_RECORD,
            rusqlite::params![item_id, item_type],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "delete delete record".to_string(),
        })
    }

    /// Lists delete records whose retention window has expired and were not yet hard-deleted.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn list_expired_records(
        conn: &Connection,
        now: &str,
    ) -> Result<Vec<DeleteRecord>, IndexError> {
        let mut statement = conn
            .prepare(LIST_EXPIRED_DELETE_RECORDS)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "prepare list expired delete records query".to_string(),
            })?;
        let rows = statement
            .query_map([now], decode_raw_delete_record)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "list expired delete records".to_string(),
            })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode expired delete records".to_string(),
            })?
            .into_iter()
            .map(raw_delete_record_into_domain)
            .collect()
    }

    /// Marks one delete record as hard-deleted and updates its optional delete revision.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if enum conversion or update execution fails.
    pub fn mark_deleted(
        conn: &Connection,
        item_id: &str,
        item_type: ItemType,
        deleted_at: &str,
        revision: Option<&ServerRevision>,
    ) -> Result<(), IndexError> {
        let item_type = encode_item_type(item_type);

        conn.execute(
            MARK_DELETE_RECORD_DELETED,
            rusqlite::params![
                item_id,
                item_type,
                deleted_at,
                revision.map(ServerRevision::value)
            ],
        )
        .map(|_| ())
        .map_err(|source| IndexError::Sqlite {
            source,
            context: "mark delete record deleted".to_string(),
        })
    }

    /// Lists all delete records in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the query fails or row decoding is invalid.
    pub fn list_all(conn: &Connection) -> Result<Vec<DeleteRecord>, IndexError> {
        let mut statement =
            conn.prepare(LIST_ALL_DELETE_RECORDS)
                .map_err(|source| IndexError::Sqlite {
                    source,
                    context: "prepare list all delete records query".to_string(),
                })?;
        let rows = statement
            .query_map([], decode_raw_delete_record)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "list all delete records".to_string(),
            })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "decode all delete records".to_string(),
            })?
            .into_iter()
            .map(raw_delete_record_into_domain)
            .collect()
    }

    /// Returns the next local delete revision for a newly purged item.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if the scalar query fails.
    pub fn next_delete_revision(conn: &Connection) -> Result<ServerRevision, IndexError> {
        conn.query_row(SELECT_NEXT_DELETE_REVISION, [], |row| row.get(0))
            .map(ServerRevision::new)
            .map_err(|source| IndexError::Sqlite {
                source,
                context: "get next delete revision".to_string(),
            })
    }
}

struct RawDeleteRecord {
    item_id: String,
    item_type: String,
    original_path: String,
    trashed_at: String,
    retention_until: String,
    deleted_at: Option<String>,
    delete_revision: Option<i64>,
}

fn decode_raw_delete_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawDeleteRecord> {
    Ok(RawDeleteRecord {
        item_id: row.get(0)?,
        item_type: row.get(1)?,
        original_path: row.get(2)?,
        trashed_at: row.get(3)?,
        retention_until: row.get(4)?,
        deleted_at: row.get(5)?,
        delete_revision: row.get(6)?,
    })
}

fn raw_delete_record_into_domain(raw: RawDeleteRecord) -> Result<DeleteRecord, IndexError> {
    Ok(DeleteRecord {
        target: decode_target(&raw.item_id, &raw.item_type)?,
        original_path: RelativePath::new(&raw.original_path),
        trashed_at: raw.trashed_at,
        retention_until: raw.retention_until,
        deleted_at: raw.deleted_at,
        delete_revision: raw.delete_revision.map(ServerRevision::new),
    })
}

fn encode_item_type(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Note => "note",
        ItemType::Asset => "asset",
    }
}

fn decode_target(item_id: &str, item_type: &str) -> Result<DeleteRecordTarget, IndexError> {
    match item_type {
        "note" => Ok(DeleteRecordTarget::Note(item_id.to_string().into())),
        "asset" => Ok(DeleteRecordTarget::Asset(item_id.to_string().into())),
        _ => Err(IndexError::SchemaCorruption {
            table: "delete_records",
            column: "item_type",
            value: item_type.to_string(),
        }),
    }
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
