// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use serde_json::json;
use zk_domain::{
    ChunkId, Citation, ConversationId, ConversationRecord, EmbeddingModelId, MessageId,
    MessageRecord, MessageRole, RetrievalChunk, RetrievalContext, UserId, id::NoteId,
};

const CONVERSATION_ULID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";
const MESSAGE_ULID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";

#[test]
fn message_role_round_trips_through_serde_and_from_str() {
    for (role, serialized) in [
        (MessageRole::System, "system"),
        (MessageRole::User, "user"),
        (MessageRole::Assistant, "assistant"),
    ] {
        let encoded = serde_json::to_value(role).expect("message role should serialize");
        assert_eq!(encoded, json!(serialized));

        let decoded: MessageRole =
            serde_json::from_value(encoded).expect("message role should deserialize");
        assert_eq!(decoded, role);
        assert_eq!(
            MessageRole::from_str(serialized).expect("message role should parse"),
            role
        );
        assert_eq!(role.to_string(), serialized);
    }
}

#[test]
fn message_role_from_str_rejects_unknown_values() {
    let error = MessageRole::from_str("tool").expect_err("unknown role should fail");

    assert_eq!(error.value(), "tool");
    assert_eq!(error.to_string(), "unknown message role: tool");
}

#[test]
fn conversation_and_message_ids_parse_display_round_trip() {
    let conversation_id =
        ConversationId::from_str(CONVERSATION_ULID).expect("conversation id should parse");
    let message_id = MessageId::from_str(MESSAGE_ULID).expect("message id should parse");

    assert_eq!(conversation_id.as_str(), CONVERSATION_ULID);
    assert_eq!(conversation_id.to_string(), CONVERSATION_ULID);
    assert_eq!(message_id.as_str(), MESSAGE_ULID);
    assert_eq!(message_id.to_string(), MESSAGE_ULID);

    let encoded_conversation =
        serde_json::to_string(&conversation_id).expect("conversation id should serialize");
    let encoded_message = serde_json::to_string(&message_id).expect("message id should serialize");

    assert_eq!(encoded_conversation, format!("\"{CONVERSATION_ULID}\""));
    assert_eq!(encoded_message, format!("\"{MESSAGE_ULID}\""));
    assert_eq!(
        serde_json::from_str::<ConversationId>(&encoded_conversation)
            .expect("conversation id should deserialize"),
        conversation_id
    );
    assert_eq!(
        serde_json::from_str::<MessageId>(&encoded_message).expect("message id should deserialize"),
        message_id
    );
}

#[test]
fn citation_round_trips_with_deterministic_field_order() {
    let citation = Citation {
        note_id: NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T"),
        chunk_id: ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6V"),
        similarity: 0.875,
        snippet: "Relevant paragraph.".to_string(),
        ordinal: 2,
    };

    let encoded = serde_json::to_string(&citation).expect("citation should serialize");
    assert_eq!(
        encoded,
        r#"{"note_id":"01JAR5A3PJEZ3SKG67PN8N6S6T","chunk_id":"01JAR5A3PJEZ3SKG67PN8N6S6V","similarity":0.875,"snippet":"Relevant paragraph.","ordinal":2}"#
    );

    let decoded: Citation = serde_json::from_str(&encoded).expect("citation should deserialize");
    assert_eq!(decoded, citation);
}

#[test]
fn chat_records_and_retrieval_context_round_trip() {
    let conversation_id =
        ConversationId::from_str(CONVERSATION_ULID).expect("conversation id should parse");
    let message_id = MessageId::from_str(MESSAGE_ULID).expect("message id should parse");
    let created_at = Utc
        .with_ymd_and_hms(2026, 5, 7, 12, 0, 0)
        .single()
        .expect("timestamp should be valid");
    let updated_at = Utc
        .with_ymd_and_hms(2026, 5, 7, 12, 1, 0)
        .single()
        .expect("timestamp should be valid");
    let conversation = ConversationRecord {
        conversation_id: conversation_id.clone(),
        title: Some("Sync design".to_string()),
        user_id: UserId::new("user-1"),
        created_at,
        updated_at,
        last_message_at: Some(updated_at),
        archived_at: None,
    };
    let message = MessageRecord {
        message_id,
        conversation_id,
        role: MessageRole::Assistant,
        content: "Answer text.".to_string(),
        token_count: Some(3),
        created_at: updated_at,
        server_revision: Some(42),
    };
    let retrieval = RetrievalContext {
        chunks: vec![RetrievalChunk {
            chunk_id: ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6V"),
            note_id: NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T"),
            text: "Retrieved text.".to_string(),
            similarity: 0.5,
            token_estimate: 4,
        }],
        total_token_estimate: 4,
        model: EmbeddingModelId::new("stub/sha256-128").expect("model id should be valid"),
    };

    let conversation_json =
        serde_json::to_string(&conversation).expect("conversation should serialize");
    let message_json = serde_json::to_string(&message).expect("message should serialize");
    let retrieval_json = serde_json::to_string(&retrieval).expect("retrieval should serialize");

    assert_eq!(
        serde_json::from_str::<ConversationRecord>(&conversation_json)
            .expect("conversation should deserialize"),
        conversation
    );
    assert_eq!(
        serde_json::from_str::<MessageRecord>(&message_json).expect("message should deserialize"),
        message
    );
    assert_eq!(
        serde_json::from_str::<RetrievalContext>(&retrieval_json)
            .expect("retrieval should deserialize"),
        retrieval
    );
}
