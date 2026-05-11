use chrono::{DateTime, Utc};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, ffi};
use zk_domain::{ConversationId, ConversationRecord, auth::UserId};

use super::timestamps::{decode_optional_timestamp, decode_timestamp, encode_timestamp};
use crate::{ServerError, ServerResult};

const INSERT_CONVERSATION_SQL: &str = r"
INSERT INTO conversations (
    conversation_id,
    user_id,
    title,
    created_at,
    updated_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5
)
";
const GET_CONVERSATION_SQL: &str = r"
SELECT
    conversation_id,
    user_id,
    title,
    created_at,
    updated_at,
    last_message_at,
    archived_at
FROM conversations
WHERE conversation_id = ?1
LIMIT 1
";
const LIST_LIVE_CONVERSATIONS_SQL: &str = r"
SELECT
    conversation_id,
    user_id,
    title,
    created_at,
    updated_at,
    last_message_at,
    archived_at
FROM conversations
WHERE user_id = ?1 AND archived_at IS NULL
ORDER BY last_message_at IS NULL ASC, last_message_at DESC, created_at DESC, conversation_id ASC
LIMIT ?2 OFFSET ?3
";
const LIST_ALL_CONVERSATIONS_SQL: &str = r"
SELECT
    conversation_id,
    user_id,
    title,
    created_at,
    updated_at,
    last_message_at,
    archived_at
FROM conversations
WHERE user_id = ?1
ORDER BY last_message_at IS NULL ASC, last_message_at DESC, created_at DESC, conversation_id ASC
LIMIT ?2 OFFSET ?3
";
const RENAME_CONVERSATION_SQL: &str = r"
UPDATE conversations
SET title = ?2, updated_at = ?3
WHERE conversation_id = ?1
";
const ARCHIVE_CONVERSATION_SQL: &str =
    "UPDATE conversations SET archived_at = ?2 WHERE conversation_id = ?1";
const UNARCHIVE_CONVERSATION_SQL: &str =
    "UPDATE conversations SET archived_at = NULL WHERE conversation_id = ?1";
const DELETE_CONVERSATION_SQL: &str = "DELETE FROM conversations WHERE conversation_id = ?1";
const TOUCH_LAST_MESSAGE_AT_SQL: &str =
    "UPDATE conversations SET last_message_at = ?2 WHERE conversation_id = ?1";

const DEFAULT_LIST_LIMIT: u32 = 50;
const MAX_LIST_LIMIT: u32 = 200;
const CONVERSATION_NOT_FOUND: &str = "conversation not found";

/// Parameters for listing chat conversations for one user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListConversationsParams {
    /// Include archived conversations when true.
    pub include_archived: bool,
    /// Maximum number of rows to return. Must be in `[1, 200]`.
    pub limit: u32,
    /// Number of matching rows to skip.
    pub offset: u32,
}

impl Default for ListConversationsParams {
    fn default() -> Self {
        Self {
            include_archived: false,
            limit: DEFAULT_LIST_LIMIT,
            offset: 0,
        }
    }
}

/// Repository over the server `conversations` table.
pub struct ConversationsRepo;

impl ConversationsRepo {
    /// Creates a conversation for one user and returns the persisted row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] on generated id collision,
    /// [`ServerError::Internal`] on foreign-key failure, or [`ServerError`]
    /// when the insert fails.
    pub fn create(
        conn: &Connection,
        user_id: &UserId,
        title: Option<&str>,
    ) -> ServerResult<ConversationRecord> {
        let conversation_id = ConversationId::generate();
        let now = Utc::now();
        let created_at = encode_timestamp(now);

        conn.execute(
            INSERT_CONVERSATION_SQL,
            rusqlite::params![
                conversation_id.as_str(),
                user_id.as_str(),
                title,
                created_at,
                created_at,
            ],
        )
        .map_err(map_create_conversation_error)?;

        Ok(ConversationRecord {
            conversation_id,
            title: title.map(str::to_string),
            user_id: user_id.clone(),
            created_at: now,
            updated_at: now,
            last_message_at: None,
            archived_at: None,
        })
    }

    /// Fetches one conversation by id.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the query fails or row decoding is invalid.
    pub fn get(
        conn: &Connection,
        conversation_id: &ConversationId,
    ) -> ServerResult<Option<ConversationRecord>> {
        let mut statement = conn
            .prepare_cached(GET_CONVERSATION_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([conversation_id.as_str()], decode_conversation_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Lists conversations for one user in recent-conversation order.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::InvalidRequest`] for an invalid limit, or
    /// [`ServerError`] when the query fails or row decoding is invalid.
    pub fn list_for_user(
        conn: &Connection,
        user_id: &UserId,
        params: ListConversationsParams,
    ) -> ServerResult<Vec<ConversationRecord>> {
        let limit = decode_limit(params.limit)?;
        let sql = if params.include_archived {
            LIST_ALL_CONVERSATIONS_SQL
        } else {
            LIST_LIVE_CONVERSATIONS_SQL
        };
        let mut statement = conn.prepare_cached(sql).map_err(ServerError::Database)?;
        let rows = statement
            .query_map(
                rusqlite::params![user_id.as_str(), limit, i64::from(params.offset)],
                decode_conversation_row,
            )
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Renames a conversation and updates its metadata timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] when the conversation does not exist,
    /// or [`ServerError`] when the update fails.
    pub fn rename(
        conn: &Connection,
        conversation_id: &ConversationId,
        title: &str,
    ) -> ServerResult<()> {
        let updated_at = encode_timestamp(Utc::now());
        execute_required(
            conn,
            RENAME_CONVERSATION_SQL,
            rusqlite::params![conversation_id.as_str(), title, updated_at],
        )
    }

    /// Marks one conversation as archived.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] when the conversation does not exist,
    /// or [`ServerError`] when the update fails.
    pub fn mark_archived(
        conn: &Connection,
        conversation_id: &ConversationId,
        now: DateTime<Utc>,
    ) -> ServerResult<()> {
        let archived_at = encode_timestamp(now);
        execute_required(
            conn,
            ARCHIVE_CONVERSATION_SQL,
            rusqlite::params![conversation_id.as_str(), archived_at],
        )
    }

    /// Clears one conversation's archive timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] when the conversation does not exist,
    /// or [`ServerError`] when the update fails.
    pub fn unarchive(conn: &Connection, conversation_id: &ConversationId) -> ServerResult<()> {
        execute_required(conn, UNARCHIVE_CONVERSATION_SQL, [conversation_id.as_str()])
    }

    /// Permanently deletes one conversation.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] when the conversation does not exist,
    /// or [`ServerError`] when the delete fails.
    pub fn delete(conn: &Connection, conversation_id: &ConversationId) -> ServerResult<()> {
        execute_required(conn, DELETE_CONVERSATION_SQL, [conversation_id.as_str()])
    }

    /// Updates only the recent-message timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] when the conversation does not exist,
    /// or [`ServerError`] when the update fails.
    pub fn touch_last_message_at(
        conn: &Connection,
        conversation_id: &ConversationId,
        now: DateTime<Utc>,
    ) -> ServerResult<()> {
        let last_message_at = encode_timestamp(now);
        execute_required(
            conn,
            TOUCH_LAST_MESSAGE_AT_SQL,
            rusqlite::params![conversation_id.as_str(), last_message_at],
        )
    }
}

fn execute_required<P>(conn: &Connection, sql: &str, params: P) -> ServerResult<()>
where
    P: rusqlite::Params,
{
    let rows_changed = conn.execute(sql, params).map_err(ServerError::Database)?;
    if rows_changed == 0 {
        return Err(ServerError::NotFound(CONVERSATION_NOT_FOUND.to_string()));
    }

    Ok(())
}

fn decode_conversation_row(row: &Row<'_>) -> rusqlite::Result<ConversationRecord> {
    Ok(ConversationRecord {
        conversation_id: ConversationId::from(row.get::<_, String>(0)?),
        user_id: UserId::from(row.get::<_, String>(1)?),
        title: row.get(2)?,
        created_at: decode_timestamp(row, 3)?,
        updated_at: decode_timestamp(row, 4)?,
        last_message_at: decode_optional_timestamp(row, 5)?,
        archived_at: decode_optional_timestamp(row, 6)?,
    })
}

fn decode_limit(limit: u32) -> ServerResult<i64> {
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(ServerError::InvalidRequest(
            "limit must be between 1 and 200".to_string(),
        ));
    }

    Ok(i64::from(limit))
}

fn map_create_conversation_error(source: rusqlite::Error) -> ServerError {
    if is_conversation_id_collision(&source) {
        ServerError::Conflict("conversation already exists".to_string())
    } else if is_foreign_key_violation(&source) {
        tracing::warn!(error = %source, "conversation create failed foreign key validation");
        ServerError::Internal(Box::new(source))
    } else {
        ServerError::Database(source)
    }
}

fn is_conversation_id_collision(source: &rusqlite::Error) -> bool {
    matches!(
        source,
        rusqlite::Error::SqliteFailure(error, Some(message))
            if error.code == ErrorCode::ConstraintViolation
                && matches!(
                    error.extended_code,
                    ffi::SQLITE_CONSTRAINT_PRIMARYKEY | ffi::SQLITE_CONSTRAINT_UNIQUE
                )
                && message.contains("conversations.conversation_id")
    )
}

fn is_foreign_key_violation(source: &rusqlite::Error) -> bool {
    matches!(
        source,
        rusqlite::Error::SqliteFailure(error, _)
            if error.code == ErrorCode::ConstraintViolation
                && error.extended_code == ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}
