// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::any::type_name;

use zk_domain::{
    Capability, ChunkEmbeddingRecord, ChunkId, ChunkRecord, Citation, ConversationId,
    ConversationIdParseError, ConversationRecord, EmbeddingModelId, EmbeddingVector, ExecutionMode,
    JobId, MessageId, MessageIdParseError, MessageRecord, MessageRole, MessageRoleParseError,
    ProviderId, ProviderSelectorPolicy, RetrievalChunk, RetrievalContext, SemanticEdgeRecord,
};

#[test]
fn ai_domain_types_are_available_from_crate_root() {
    assert!(!type_name::<ChunkId>().is_empty());
    assert!(!type_name::<ChunkRecord>().is_empty());
    assert!(!type_name::<EmbeddingModelId>().is_empty());
    assert!(!type_name::<ProviderId>().is_empty());
    assert!(!type_name::<EmbeddingVector>().is_empty());
    assert!(!type_name::<ChunkEmbeddingRecord>().is_empty());
    assert!(!type_name::<SemanticEdgeRecord>().is_empty());
    assert!(!type_name::<ExecutionMode>().is_empty());
    assert!(!type_name::<Capability>().is_empty());
    assert!(!type_name::<ProviderSelectorPolicy>().is_empty());
    assert!(!type_name::<JobId>().is_empty());
    assert!(!type_name::<ConversationId>().is_empty());
    assert!(!type_name::<ConversationIdParseError>().is_empty());
    assert!(!type_name::<MessageId>().is_empty());
    assert!(!type_name::<MessageIdParseError>().is_empty());
    assert!(!type_name::<MessageRole>().is_empty());
    assert!(!type_name::<MessageRoleParseError>().is_empty());
    assert!(!type_name::<MessageRecord>().is_empty());
    assert!(!type_name::<ConversationRecord>().is_empty());
    assert!(!type_name::<Citation>().is_empty());
    assert!(!type_name::<RetrievalChunk>().is_empty());
    assert!(!type_name::<RetrievalContext>().is_empty());
}
