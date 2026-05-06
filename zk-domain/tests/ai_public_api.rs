// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::any::type_name;

use zk_domain::{
    Capability, ChunkEmbeddingRecord, ChunkId, ChunkRecord, EmbeddingModelId, EmbeddingVector,
    ExecutionMode, JobId, ProviderId, ProviderSelectorPolicy, SemanticEdgeRecord,
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
}
