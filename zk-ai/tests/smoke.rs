// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::io;

use zk_ai::{AiError, AiResult};

#[test]
fn ai_error_variants_display_and_are_send_sync_static() {
    let provider_unavailable = AiError::ProviderUnavailable("remote-http".to_string());
    assert_eq!(
        provider_unavailable.to_string(),
        "provider unavailable: remote-http"
    );

    let provider_request = AiError::ProviderRequest("429 rate limited".to_string());
    assert_eq!(
        provider_request.to_string(),
        "provider request failed: 429 rate limited"
    );

    let chunking = AiError::Chunking("invalid paragraph boundary".to_string());
    assert_eq!(
        chunking.to_string(),
        "chunking failed: invalid paragraph boundary"
    );

    let dimension_mismatch = AiError::DimensionMismatch {
        expected: 384,
        actual: 1536,
    };
    assert_eq!(
        dimension_mismatch.to_string(),
        "embedding dimension mismatch: expected 384, got 1536"
    );

    let invalid_embedding_blob = AiError::InvalidEmbeddingBlob { len: 7 };
    assert_eq!(
        invalid_embedding_blob.to_string(),
        "invalid embedding blob length 7; expected a non-empty multiple of 4 bytes"
    );

    let invalid_config = AiError::InvalidConfig("dim must be positive".to_string());
    assert_eq!(
        invalid_config.to_string(),
        "invalid configuration: dim must be positive"
    );

    let io_error = AiError::from(io::Error::other("model file missing"));
    assert_eq!(io_error.to_string(), "io error: model file missing");

    let internal = AiError::Internal("provider registry empty".to_string());
    assert_eq!(
        internal.to_string(),
        "internal error: provider registry empty"
    );

    let alias_round_trip: AiResult<()> = Err(AiError::ProviderUnavailable("stub".to_string()));
    assert!(matches!(
        alias_round_trip,
        Err(AiError::ProviderUnavailable(provider)) if provider == "stub"
    ));

    assert_send_sync_static::<AiError>();
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}
