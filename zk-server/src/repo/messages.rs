use std::str::FromStr;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, types::Type};
use zk_domain::{ConversationId, MessageId, MessageRecord, MessageRole};

use super::timestamps::{decode_timestamp, encode_timestamp};
use crate::{ServerError, ServerResult};

const INSERT_MESSAGE_SQL: &str = r"
INSERT INTO messages (
    message_id,
    conversation_id,
    role,
    content,
    token_count,
    created_at,
    server_revision
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, NULL
)
";
const GET_MESSAGE_SQL: &str = r"
SELECT
    message_id,
    conversation_id,
    role,
    content,
    token_count,
    created_at,
    server_revision
FROM messages
WHERE message_id = ?1
LIMIT 1
";
const LIST_MESSAGES_SQL: &str = r"
SELECT
    message_id,
    conversation_id,
    role,
    content,
    token_count,
    created_at,
    server_revision
FROM messages
WHERE conversation_id = ?1
  AND (?2 IS NULL OR created_at < ?2)
  AND (?3 OR role <> 'system')
ORDER BY created_at ASC, message_id ASC
LIMIT ?4
";

const DEFAULT_LIST_LIMIT: u32 = 200;
const MAX_LIST_LIMIT: u32 = 1_000;

/// Parameters for appending one immutable chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendMessageParams {
    /// Parent conversation identifier.
    pub conversation_id: ConversationId,
    /// Message role.
    pub role: MessageRole,
    /// Full message content.
    pub content: String,
    /// Optional provider-reported token count.
    pub token_count: Option<u32>,
    /// Message creation timestamp.
    pub now: DateTime<Utc>,
}

/// Parameters for listing chat messages for one conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListMessagesParams {
    /// Maximum number of rows to return. Must be in `[1, 1000]`.
    pub limit: u32,
    /// Optional exclusive timestamp cursor for paging backward through history.
    pub before: Option<DateTime<Utc>>,
    /// Include system messages when true.
    pub include_system: bool,
}

impl Default for ListMessagesParams {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIST_LIMIT,
            before: None,
            include_system: true,
        }
    }
}

/// Repository over the server `messages` table.
pub struct MessagesRepo;

impl MessagesRepo {
    /// Appends one immutable message and returns the persisted row.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the insert fails.
    pub fn append(conn: &Connection, params: AppendMessageParams) -> ServerResult<MessageRecord> {
        let message_id = MessageId::generate();
        let created_at = encode_timestamp(params.now);

        conn.execute(
            INSERT_MESSAGE_SQL,
            rusqlite::params![
                message_id.as_str(),
                params.conversation_id.as_str(),
                params.role.as_str(),
                &params.content,
                params.token_count,
                created_at,
            ],
        )
        .map_err(ServerError::Database)?;

        Ok(MessageRecord {
            message_id,
            conversation_id: params.conversation_id,
            role: params.role,
            content: params.content,
            token_count: params.token_count,
            created_at: params.now,
            server_revision: None,
        })
    }

    /// Fetches one message by id.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the query fails or row decoding is invalid.
    pub fn get(conn: &Connection, message_id: &MessageId) -> ServerResult<Option<MessageRecord>> {
        let mut statement = conn
            .prepare_cached(GET_MESSAGE_SQL)
            .map_err(ServerError::Database)?;

        statement
            .query_row([message_id.as_str()], decode_message_row)
            .optional()
            .map_err(ServerError::Database)
    }

    /// Lists messages in chronological conversation order.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::InvalidRequest`] for an invalid limit, or
    /// [`ServerError`] when the query fails or row decoding is invalid.
    pub fn list_for_conversation(
        conn: &Connection,
        conversation_id: &ConversationId,
        params: ListMessagesParams,
    ) -> ServerResult<Vec<MessageRecord>> {
        let limit = decode_limit(params.limit)?;
        let before = params.before.map(encode_timestamp);
        let mut statement = conn
            .prepare_cached(LIST_MESSAGES_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map(
                rusqlite::params![
                    conversation_id.as_str(),
                    before.as_deref(),
                    params.include_system,
                    limit,
                ],
                decode_message_row,
            )
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }
}

fn decode_message_row(row: &Row<'_>) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        message_id: MessageId::from(row.get::<_, String>(0)?),
        conversation_id: ConversationId::from(row.get::<_, String>(1)?),
        role: decode_role(row, 2)?,
        content: row.get(3)?,
        token_count: decode_optional_u32(row, 4)?,
        created_at: decode_timestamp(row, 5)?,
        server_revision: row.get(6)?,
    })
}

fn decode_role(row: &Row<'_>, index: usize) -> rusqlite::Result<MessageRole> {
    let value = row.get::<_, String>(index)?;
    MessageRole::from_str(&value).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(source))
    })
}

fn decode_optional_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u32>> {
    let value = row.get::<_, Option<i64>>(index)?;
    value
        .map(|number| {
            u32::try_from(number)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, number))
        })
        .transpose()
}

fn decode_limit(limit: u32) -> ServerResult<i64> {
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(ServerError::InvalidRequest(
            "limit must be between 1 and 1000".to_string(),
        ));
    }

    Ok(i64::from(limit))
}
