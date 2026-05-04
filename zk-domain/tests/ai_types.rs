// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fmt::Debug, hash::Hash};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use zk_domain::{
    AiIdentifierError, ChunkEmbeddingRecord, ChunkId, ChunkIdParseError, ChunkRecord,
    EmbeddingModelId, EmbeddingVector, EmbeddingVectorError, ProviderId, SemanticEdgeRecord,
    id::{ContentHash, NoteId},
};

fn assert_eq_common_traits<T>()
where
    T: Debug + Clone + PartialEq + Eq + Serialize + DeserializeOwned,
{
}

fn assert_hashable_id_traits<T>()
where
    T: Debug + Clone + PartialEq + Eq + Hash + Serialize + DeserializeOwned,
{
}

fn assert_float_record_traits<T>()
where
    T: Debug + Clone + PartialEq + Serialize + DeserializeOwned,
{
}

fn assert_error_traits<T>()
where
    T: Debug + Clone + PartialEq + Eq,
{
}

#[test]
fn ai_domain_types_implement_required_common_traits() {
    assert_hashable_id_traits::<ChunkId>();
    assert_eq_common_traits::<ChunkRecord>();
    assert_eq_common_traits::<EmbeddingModelId>();
    assert_eq_common_traits::<ProviderId>();
    assert_error_traits::<AiIdentifierError>();
    assert_error_traits::<ChunkIdParseError>();
    assert_error_traits::<EmbeddingVectorError>();
    assert_float_record_traits::<EmbeddingVector>();
    assert_float_record_traits::<ChunkEmbeddingRecord>();
    assert_float_record_traits::<SemanticEdgeRecord>();
}

#[test]
fn chunk_id_generate_produces_serializable_ulid_value() {
    let chunk_id = ChunkId::generate();
    let raw = chunk_id.as_str();

    assert_eq!(raw.len(), 26);
    assert!(
        raw.chars()
            .all(|character| character.is_ascii_alphanumeric())
    );

    let json = serde_json::to_string(&chunk_id).expect("chunk id should serialize");
    let decoded: ChunkId = serde_json::from_str(&json).expect("chunk id should deserialize");

    assert_eq!(decoded, chunk_id);
    assert_eq!(decoded.to_string(), raw);
}

#[test]
fn chunk_record_round_trips_via_json() {
    let record = ChunkRecord {
        chunk_id: ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6R"),
        note_id: NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S"),
        ordinal: 7,
        content_hash: ContentHash::from_bytes(b"# Heading\n\nBody"),
        text: "# Heading\n\nBody".to_string(),
        token_count: Some(4),
        created_at: ts((2026, 5, 4, 8, 0, 0)),
    };

    let json = serde_json::to_value(&record).expect("chunk record should serialize");

    assert_eq!(json["chunk_id"], "01JAR5A3PJEZ3SKG67PN8N6S6R");
    assert_eq!(json["note_id"], "01JAR5A3PJEZ3SKG67PN8N6S6S");
    assert_eq!(json["ordinal"], 7);
    assert_eq!(json["content_hash"], record.content_hash.as_str());
    assert_eq!(json["text"], "# Heading\n\nBody");
    assert_eq!(json["token_count"], 4);
    assert_eq!(json["created_at"], "2026-05-04T08:00:00Z");

    let decoded: ChunkRecord =
        serde_json::from_value(json).expect("chunk record should deserialize");
    assert_eq!(decoded, record);
}

#[test]
fn embedding_identifiers_accept_canonical_values() {
    let model = "openai/text-embedding-3-small"
        .parse::<EmbeddingModelId>()
        .expect("model id should parse");
    let provider = "local-candle"
        .parse::<ProviderId>()
        .expect("provider id should parse");

    assert_eq!(model.as_str(), "openai/text-embedding-3-small");
    assert_eq!(model.to_string(), "openai/text-embedding-3-small");
    assert_eq!(provider.as_str(), "local-candle");
    assert_eq!(provider.to_string(), "local-candle");
}

#[test]
fn embedding_identifiers_reject_invalid_values() {
    for value in ["", "local\nmodel", "remote\u{7f}", "local/\u{43c}\u{438}"] {
        assert_eq!(
            value.parse::<EmbeddingModelId>(),
            Err(AiIdentifierError::InvalidValue)
        );
        assert_eq!(
            value.parse::<ProviderId>(),
            Err(AiIdentifierError::InvalidValue)
        );
    }

    let too_long = "a".repeat(129);

    assert_eq!(
        too_long.parse::<EmbeddingModelId>(),
        Err(AiIdentifierError::TooLong {
            max: 128,
            actual: 129,
        })
    );
    assert_eq!(
        too_long.parse::<ProviderId>(),
        Err(AiIdentifierError::TooLong {
            max: 128,
            actual: 129,
        })
    );
}

#[test]
fn embedding_vector_reports_dimension_and_dot_product() {
    let first = vector([1.0, 2.0, 3.0]);
    let second = vector([4.0, 5.0, 6.0]);

    assert_eq!(first.dim(), 3);
    let dot_product = first
        .dot_product(&second)
        .expect("same-dimension vectors should dot");

    assert!((dot_product - 32.0).abs() <= 1e-6);
}

#[test]
fn embedding_vector_identical_vectors_give_similarity_one() {
    let first = vector([1.0, 0.0, 0.0]);
    let identical = vector([1.0, 0.0, 0.0]);

    let same = first
        .cosine_similarity(&identical)
        .expect("same-dimension vectors should compare");

    assert!((same - 1.0).abs() <= 1e-6);
}

#[test]
fn embedding_vector_orthogonal_vectors_give_similarity_zero() {
    let first = vector([1.0, 0.0, 0.0]);
    let orthogonal = vector([0.0, 1.0, 0.0]);

    let perpendicular = first
        .cosine_similarity(&orthogonal)
        .expect("same-dimension vectors should compare");

    assert!(perpendicular.abs() <= 1e-6);
}

#[test]
fn embedding_vector_returns_zero_similarity_for_zero_norm_vectors() {
    let zero = vector([0.0, 0.0, 0.0]);
    let other = vector([1.0, 2.0, 3.0]);

    let similarity = zero
        .cosine_similarity(&other)
        .expect("same-dimension vectors should compare");

    assert!(similarity.abs() <= 1e-6);
}

#[test]
fn embedding_vector_dimension_mismatch_returns_error() {
    let first = vector([1.0, 2.0, 3.0]);
    let second = EmbeddingVector {
        values: vec![1.0, 2.0],
        model: model_id(),
    };

    assert_eq!(
        first.dot_product(&second),
        Err(EmbeddingVectorError::DimensionMismatch {
            expected: 3,
            actual: 2,
        })
    );
    assert_eq!(
        first.cosine_similarity(&second),
        Err(EmbeddingVectorError::DimensionMismatch {
            expected: 3,
            actual: 2,
        })
    );
}

#[test]
fn embedding_vector_normalize_in_place_scales_to_unit_length() {
    let mut embedding = vector([3.0, 4.0]);

    embedding.normalize_in_place();

    assert!((embedding.values[0] - 0.6).abs() <= 1e-6);
    assert!((embedding.values[1] - 0.8).abs() <= 1e-6);
}

#[test]
fn embedding_vector_normalize_in_place_leaves_zero_vector_unchanged() {
    let mut embedding = vector([0.0, 0.0]);

    embedding.normalize_in_place();

    assert_eq!(embedding.values, vec![0.0, 0.0]);
}

#[test]
fn embedding_vector_partial_eq_uses_float_bits() {
    let positive_zero = vector([0.0]);
    let negative_zero = vector([-0.0]);

    assert_ne!(positive_zero, negative_zero);
}

#[test]
fn chunk_embedding_record_round_trips_via_json() {
    let record = ChunkEmbeddingRecord {
        chunk_id: ChunkId::new("01JAR5A3PJEZ3SKG67PN8N6S6R"),
        model: model_id(),
        provider: provider_id(),
        vector: vector([0.1, 0.2, 0.3]),
        created_at: ts((2026, 5, 4, 9, 0, 0)),
    };

    let json = serde_json::to_value(&record).expect("chunk embedding should serialize");

    assert_eq!(json["chunk_id"], "01JAR5A3PJEZ3SKG67PN8N6S6R");
    assert_eq!(json["model"], "local/all-minilm-l6-v2");
    assert_eq!(json["provider"], "stub");
    assert_eq!(json["vector"]["values"], json!(record.vector.values));
    assert_eq!(json["vector"]["model"], "local/all-minilm-l6-v2");
    assert_eq!(json["created_at"], "2026-05-04T09:00:00Z");

    let decoded: ChunkEmbeddingRecord =
        serde_json::from_value(json).expect("chunk embedding should deserialize");
    assert_eq!(decoded, record);
}

#[test]
fn semantic_edge_record_round_trips_via_json() {
    let record = SemanticEdgeRecord {
        source_note_id: NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S"),
        target_note_id: NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6T"),
        similarity: 0.875,
        created_at: ts((2026, 5, 4, 10, 0, 0)),
    };

    let json = serde_json::to_value(&record).expect("semantic edge should serialize");

    assert_eq!(json["source_note_id"], "01JAR5A3PJEZ3SKG67PN8N6S6S");
    assert_eq!(json["target_note_id"], "01JAR5A3PJEZ3SKG67PN8N6S6T");
    assert_eq!(json["similarity"], 0.875);
    assert_eq!(json["created_at"], "2026-05-04T10:00:00Z");

    let decoded: SemanticEdgeRecord =
        serde_json::from_value(json).expect("semantic edge should deserialize");
    assert_eq!(decoded, record);
}

fn vector<const DIM: usize>(values: [f32; DIM]) -> EmbeddingVector {
    EmbeddingVector {
        values: values.to_vec(),
        model: model_id(),
    }
}

fn model_id() -> EmbeddingModelId {
    "local/all-minilm-l6-v2"
        .parse()
        .expect("test model id should parse")
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("test provider id should parse")
}

fn ts(parts: (i32, u32, u32, u32, u32, u32)) -> DateTime<Utc> {
    let (year, month, day, hour, minute, second) = parts;

    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("test timestamp is valid")
}
