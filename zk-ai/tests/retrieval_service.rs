// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use zk_ai::{
    AiResult,
    provider::{EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse},
    service::{
        retrieval::{RetrievalParams, RetrievalService},
        semantic_search::{EmbeddingsReader, TopHit},
    },
};
use zk_domain::{ChunkId, EmbeddingModelId, EmbeddingVector, ProviderId, id::NoteId};

#[tokio::test]
async fn retrieve_limits_chunks_per_note_and_preserves_similarity_order() {
    let model = model();
    let provider = Arc::new(RecordingProvider::new(model.clone()));
    let reader = Arc::new(FakeEmbeddingsReader::new(
        vec![
            hit(0, 0, 0.99),
            hit(0, 1, 0.98),
            hit(0, 2, 0.97),
            hit(1, 3, 0.90),
            hit(1, 4, 0.89),
            hit(2, 5, 0.88),
        ],
        text_by_chunk(&[
            (0, "note zero first chunk"),
            (1, "note zero second chunk"),
            (3, "note one first chunk"),
            (4, "note one second chunk"),
            (5, "note two first chunk"),
        ]),
    ));
    let service = RetrievalService::new(reader.clone(), provider.clone());

    let context = service
        .retrieve(
            "vault query",
            RetrievalParams {
                top_k: 4,
                max_chunks_per_note: 2,
                token_budget: 128,
                model: model.clone(),
            },
        )
        .await
        .expect("retrieval should succeed");

    assert_eq!(provider.requested_models(), vec![model.clone()]);
    assert_eq!(
        provider.requested_texts(),
        vec![vec!["vault query".to_string()]]
    );
    assert_eq!(reader.requested_limits(), vec![16]);
    assert_eq!(reader.requested_models(), vec![model.clone()]);
    assert_eq!(
        context
            .chunks
            .iter()
            .map(|chunk| chunk.chunk_id.clone())
            .collect::<Vec<_>>(),
        vec![chunk_id(0), chunk_id(1), chunk_id(3), chunk_id(4)]
    );
    assert!(
        context
            .chunks
            .iter()
            .all(|chunk| chunk.note_id != note_id(2))
    );
    assert_eq!(context.model, model);
}

#[tokio::test]
async fn retrieve_truncates_chunks_when_token_budget_would_be_exceeded() {
    let model = model();
    let provider = Arc::new(RecordingProvider::new(model.clone()));
    let reader = Arc::new(FakeEmbeddingsReader::new(
        vec![hit(0, 0, 0.99), hit(1, 1, 0.98), hit(2, 2, 0.97)],
        text_by_chunk(&[
            (0, "abcdefghijklmnop"),
            (1, "qrstuvwxyzabcdef"),
            (2, "ghijklmnopqrstuv"),
        ]),
    ));
    let service = RetrievalService::new(reader, provider);

    let context = service
        .retrieve(
            "budgeted query",
            RetrievalParams {
                top_k: 8,
                max_chunks_per_note: 2,
                token_budget: 9,
                model: model.clone(),
            },
        )
        .await
        .expect("retrieval should succeed");

    assert_eq!(
        context
            .chunks
            .iter()
            .map(|chunk| (chunk.chunk_id.clone(), chunk.token_estimate))
            .collect::<Vec<_>>(),
        vec![(chunk_id(0), 4), (chunk_id(1), 4)]
    );
    assert_eq!(context.total_token_estimate, 8);
    assert_eq!(context.model, model);
}

#[tokio::test]
async fn retrieve_uses_default_limits_from_params_constructor() {
    let model = model();
    let provider = Arc::new(RecordingProvider::new(model.clone()));
    let reader = Arc::new(FakeEmbeddingsReader::new(
        vec![hit(0, 0, 0.99), hit(0, 1, 0.98), hit(0, 2, 0.97)],
        text_by_chunk(&[
            (0, "first default chunk"),
            (1, "second default chunk"),
            (2, "third default chunk"),
        ]),
    ));
    let service = RetrievalService::new(reader.clone(), provider);

    let context = service
        .retrieve("default query", RetrievalParams::new(model))
        .await
        .expect("retrieval should succeed");

    assert_eq!(reader.requested_limits(), vec![32]);
    assert_eq!(
        context
            .chunks
            .iter()
            .map(|chunk| chunk.chunk_id.clone())
            .collect::<Vec<_>>(),
        vec![chunk_id(0), chunk_id(1)]
    );
}

#[derive(Clone)]
struct FakeEmbeddingsReader {
    hits: Arc<Vec<TopHit>>,
    text_by_chunk: Arc<HashMap<ChunkId, String>>,
    calls: Arc<Mutex<Vec<ReaderCall>>>,
}

impl FakeEmbeddingsReader {
    fn new(hits: Vec<TopHit>, text_by_chunk: HashMap<ChunkId, String>) -> Self {
        Self {
            hits: Arc::new(hits),
            text_by_chunk: Arc::new(text_by_chunk),
            calls: Arc::default(),
        }
    }

    fn requested_limits(&self) -> Vec<usize> {
        self.calls
            .lock()
            .expect("reader calls lock should not be poisoned")
            .iter()
            .map(|call| call.limit)
            .collect()
    }

    fn requested_models(&self) -> Vec<EmbeddingModelId> {
        self.calls
            .lock()
            .expect("reader calls lock should not be poisoned")
            .iter()
            .map(|call| call.model.clone())
            .collect()
    }
}

impl EmbeddingsReader for FakeEmbeddingsReader {
    fn top_k_cosine(
        &self,
        _query: &EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<TopHit>> {
        self.calls
            .lock()
            .expect("reader calls lock should not be poisoned")
            .push(ReaderCall {
                limit: k,
                model: model.clone(),
            });

        Ok(self.hits.iter().take(k).cloned().collect())
    }

    fn list_chunk_text(&self, chunk_ids: &[ChunkId]) -> AiResult<Vec<(ChunkId, String)>> {
        Ok(chunk_ids
            .iter()
            .filter_map(|chunk_id| {
                self.text_by_chunk
                    .get(chunk_id)
                    .map(|text| (chunk_id.clone(), text.clone()))
            })
            .collect())
    }
}

#[derive(Debug, Clone)]
struct ReaderCall {
    limit: usize,
    model: EmbeddingModelId,
}

struct RecordingProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
    request_count: AtomicUsize,
    requests: Mutex<Vec<EmbeddingsRequest>>,
}

impl RecordingProvider {
    fn new(model: EmbeddingModelId) -> Self {
        Self {
            id: provider_id(),
            capability: EmbeddingsCapability {
                model,
                dim: 2,
                max_batch_size: 8,
                max_input_tokens: 128,
            },
            request_count: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested_models(&self) -> Vec<EmbeddingModelId> {
        self.requests
            .lock()
            .expect("provider requests lock should not be poisoned")
            .iter()
            .map(|request| request.model.clone())
            .collect()
    }

    fn requested_texts(&self) -> Vec<Vec<String>> {
        self.requests
            .lock()
            .expect("provider requests lock should not be poisoned")
            .iter()
            .map(|request| request.texts.clone())
            .collect()
    }
}

#[async_trait]
impl EmbeddingsProvider for RecordingProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capability(&self) -> &EmbeddingsCapability {
        &self.capability
    }

    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        self.request_count.fetch_add(1, Ordering::SeqCst);
        self.requests
            .lock()
            .expect("provider requests lock should not be poisoned")
            .push(request.clone());

        Ok(EmbeddingsResponse {
            vectors: request
                .texts
                .iter()
                .map(|_| EmbeddingVector {
                    values: vec![1.0, 0.0],
                    model: request.model.clone(),
                })
                .collect(),
        })
    }
}

fn hit(note_index: u32, chunk_index: u32, similarity: f32) -> TopHit {
    TopHit {
        note_id: note_id(note_index),
        chunk_id: chunk_id(chunk_index),
        similarity,
    }
}

fn text_by_chunk(entries: &[(u32, &str)]) -> HashMap<ChunkId, String> {
    entries
        .iter()
        .map(|(index, text)| (chunk_id(*index), (*text).to_string()))
        .collect()
}

fn note_id(index: u32) -> NoteId {
    NoteId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
}

fn chunk_id(index: u32) -> ChunkId {
    ChunkId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
}

fn model() -> EmbeddingModelId {
    "test/retrieval-model"
        .parse()
        .expect("embedding model fixture should be valid")
}

fn provider_id() -> ProviderId {
    "retrieval-test-provider"
        .parse()
        .expect("provider fixture should be valid")
}
