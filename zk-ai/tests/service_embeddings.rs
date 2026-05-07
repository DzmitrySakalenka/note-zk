// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use zk_ai::{
    AiError, AiResult,
    provider::{EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse},
    service::embeddings::{
        EmbeddingsService, EmbeddingsServiceOptions, EmbeddingsWriter, RecomputeOutcome,
    },
};
use zk_domain::{
    ChunkEmbeddingRecord, ChunkId, ChunkRecord, EmbeddingModelId, EmbeddingVector, ProviderId,
    id::{ContentHash, NoteId},
};

#[tokio::test]
async fn recompute_embeds_missing_chunks_in_provider_batches() {
    let note_id = note_id();
    let chunks = vec![
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First"),
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second"),
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6P", 2, "Third"),
    ];
    let writer = MockEmbeddingsWriter::new(chunks.clone());
    let observer = writer.clone();
    let provider = Arc::new(MockProvider::new(2));
    let service = EmbeddingsService::new(Arc::clone(&provider), writer, no_backoff_options());

    let outcome = service
        .recompute_note(&note_id)
        .await
        .expect("recompute should succeed");

    assert_eq!(
        outcome,
        RecomputeOutcome {
            note_id: note_id.clone(),
            embedded_count: 3,
            skipped_count: 0,
        }
    );
    assert_eq!(provider.request_count(), 2);
    assert_eq!(
        provider.request_text_batches(),
        vec![vec!["First", "Second"], vec!["Third"]]
    );
    assert_eq!(observer.upsert_count(), 2);

    let records = observer.records();
    assert_eq!(records.len(), 3);
    for (record, chunk) in records.iter().zip(chunks.iter()) {
        assert_eq!(record.chunk_id, chunk.chunk_id);
        assert_eq!(record.model, model());
        assert_eq!(record.provider, provider_id());
        assert_eq!(
            record.vector,
            EmbeddingVector {
                values: vec![text_len_value(&chunk.text)],
                model: model(),
            }
        );
    }
}

#[tokio::test]
async fn provider_request_failure_retries_then_succeeds() {
    let note_id = note_id();
    let writer = MockEmbeddingsWriter::new(vec![chunk_fixture(
        &note_id,
        "01JAR5A3PJEZ3SKG67PN8N6S6R",
        0,
        "Retry",
    )]);
    let observer = writer.clone();
    let provider = Arc::new(MockProvider::new(8).with_failures_before_success(1));
    let service = EmbeddingsService::new(Arc::clone(&provider), writer, no_backoff_options());

    let outcome = service
        .recompute_note(&note_id)
        .await
        .expect("second provider attempt should succeed");

    assert_eq!(outcome.embedded_count, 1);
    assert_eq!(outcome.skipped_count, 0);
    assert_eq!(provider.request_count(), 2);
    assert_eq!(observer.records().len(), 1);
}

#[tokio::test]
async fn provider_request_failure_exhausts_attempts() {
    let note_id = note_id();
    let writer = MockEmbeddingsWriter::new(vec![chunk_fixture(
        &note_id,
        "01JAR5A3PJEZ3SKG67PN8N6S6R",
        0,
        "Fails",
    )]);
    let observer = writer.clone();
    let provider = Arc::new(MockProvider::new(8).with_failures_before_success(3));
    let service = EmbeddingsService::new(Arc::clone(&provider), writer, no_backoff_options());

    let error = service
        .recompute_note(&note_id)
        .await
        .expect_err("third provider request failure should be returned");

    assert!(matches!(error, AiError::ProviderRequest(_)));
    assert_eq!(provider.request_count(), 3);
    assert!(observer.records().is_empty());
}

#[tokio::test]
async fn empty_missing_chunk_list_returns_without_provider_call() {
    let note_id = note_id();
    let writer = MockEmbeddingsWriter::default();
    let provider = Arc::new(MockProvider::new(8));
    let service = EmbeddingsService::new(Arc::clone(&provider), writer, no_backoff_options());

    let outcome = service
        .recompute_note(&note_id)
        .await
        .expect("empty recompute should succeed");

    assert_eq!(outcome.embedded_count, 0);
    assert_eq!(outcome.skipped_count, 0);
    assert_eq!(provider.request_count(), 0);
}

#[tokio::test]
async fn provider_response_length_mismatch_is_internal_error() {
    let note_id = note_id();
    let writer = MockEmbeddingsWriter::new(vec![
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6R", 0, "First"),
        chunk_fixture(&note_id, "01JAR5A3PJEZ3SKG67PN8N6S6Q", 1, "Second"),
    ]);
    let observer = writer.clone();
    let provider = Arc::new(MockProvider::new(8).with_response_vector_count(1));
    let service = EmbeddingsService::new(provider, writer, no_backoff_options());

    let error = service
        .recompute_note(&note_id)
        .await
        .expect_err("short provider response should be rejected");

    assert!(matches!(error, AiError::Internal(_)));
    assert!(observer.records().is_empty());
}

#[derive(Clone, Default)]
struct MockEmbeddingsWriter {
    missing_chunks: Arc<Mutex<Vec<ChunkRecord>>>,
    records: Arc<Mutex<Vec<ChunkEmbeddingRecord>>>,
    upsert_count: Arc<AtomicUsize>,
}

impl MockEmbeddingsWriter {
    fn new(missing_chunks: Vec<ChunkRecord>) -> Self {
        Self {
            missing_chunks: Arc::new(Mutex::new(missing_chunks)),
            records: Arc::default(),
            upsert_count: Arc::default(),
        }
    }

    fn records(&self) -> Vec<ChunkEmbeddingRecord> {
        self.records
            .lock()
            .expect("mock records lock should not be poisoned")
            .clone()
    }

    fn upsert_count(&self) -> usize {
        self.upsert_count.load(Ordering::SeqCst)
    }
}

impl EmbeddingsWriter for MockEmbeddingsWriter {
    fn list_chunks_missing_embeddings(
        &self,
        _note_id: &NoteId,
        _model: &EmbeddingModelId,
    ) -> AiResult<Vec<ChunkRecord>> {
        Ok(self
            .missing_chunks
            .lock()
            .expect("mock missing chunks lock should not be poisoned")
            .clone())
    }

    fn upsert_embeddings(&self, records: &[ChunkEmbeddingRecord]) -> AiResult<u64> {
        self.records
            .lock()
            .expect("mock records lock should not be poisoned")
            .extend_from_slice(records);
        self.upsert_count.fetch_add(1, Ordering::SeqCst);

        Ok(u64::try_from(records.len()).expect("record count should fit u64"))
    }
}

struct MockProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
    failures_before_success: Mutex<u32>,
    response_vector_count: Mutex<Option<usize>>,
    requests: Mutex<Vec<EmbeddingsRequest>>,
}

impl MockProvider {
    fn new(max_batch_size: usize) -> Self {
        Self {
            id: provider_id(),
            capability: EmbeddingsCapability {
                model: model(),
                dim: 1,
                max_batch_size,
                max_input_tokens: 128,
            },
            failures_before_success: Mutex::new(0),
            response_vector_count: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn with_failures_before_success(self, failures_before_success: u32) -> Self {
        *self
            .failures_before_success
            .lock()
            .expect("mock failure lock should not be poisoned") = failures_before_success;
        self
    }

    fn with_response_vector_count(self, response_vector_count: usize) -> Self {
        *self
            .response_vector_count
            .lock()
            .expect("mock response count lock should not be poisoned") =
            Some(response_vector_count);
        self
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("mock requests lock should not be poisoned")
            .len()
    }

    fn request_text_batches(&self) -> Vec<Vec<String>> {
        self.requests
            .lock()
            .expect("mock requests lock should not be poisoned")
            .iter()
            .map(|request| request.texts.clone())
            .collect()
    }
}

#[async_trait]
impl EmbeddingsProvider for MockProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capability(&self) -> &EmbeddingsCapability {
        &self.capability
    }

    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        self.requests
            .lock()
            .expect("mock requests lock should not be poisoned")
            .push(request.clone());

        {
            let mut failures = self
                .failures_before_success
                .lock()
                .expect("mock failure lock should not be poisoned");
            if *failures > 0 {
                *failures -= 1;
                return Err(AiError::ProviderRequest("transient failure".to_string()));
            }
        }

        let vector_count = self
            .response_vector_count
            .lock()
            .expect("mock response count lock should not be poisoned")
            .unwrap_or(request.texts.len());
        let vectors = request
            .texts
            .iter()
            .take(vector_count)
            .map(|text| EmbeddingVector {
                values: vec![text_len_value(text)],
                model: request.model.clone(),
            })
            .collect();

        Ok(EmbeddingsResponse { vectors })
    }
}

fn no_backoff_options() -> EmbeddingsServiceOptions {
    EmbeddingsServiceOptions {
        max_attempts: 3,
        initial_backoff: Duration::ZERO,
        backoff_max: Duration::ZERO,
    }
}

fn chunk_fixture(note_id: &NoteId, chunk_id: &str, ordinal: u32, text: &str) -> ChunkRecord {
    ChunkRecord {
        chunk_id: ChunkId::new(chunk_id),
        note_id: note_id.clone(),
        ordinal,
        content_hash: ContentHash::from_bytes(text.as_bytes()),
        text: text.to_string(),
        token_count: Some(ordinal + 3),
        created_at: parse_timestamp("2026-05-05T10:10:00Z"),
    }
}

fn note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S")
}

fn model() -> EmbeddingModelId {
    "test/model-a"
        .parse()
        .expect("embedding model fixture should be valid")
}

fn provider_id() -> ProviderId {
    "test-provider"
        .parse()
        .expect("provider fixture should be valid")
}

fn text_len_value(text: &str) -> f32 {
    f32::from(u16::try_from(text.len()).expect("fixture text length should fit u16"))
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
