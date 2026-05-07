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
    AiError, AiResult,
    provider::{
        EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse,
        StubEmbeddingsProvider, embed_one,
    },
    service::semantic_search::{
        EmbeddingsReader, SemanticSearchService, TopHit, build_search_hits,
    },
};
use zk_domain::{ChunkId, EmbeddingModelId, EmbeddingVector, ProviderId, id::NoteId};

#[tokio::test]
async fn query_returns_hits_in_similarity_order() {
    let provider = Arc::new(StubEmbeddingsProvider::new(2));
    let model = provider.capability().model.clone();
    let query_vector = embed_one(
        provider.as_ref(),
        "semantic topic".to_string(),
        model.clone(),
    )
    .await
    .expect("stub provider should embed query");
    let repo = InMemoryEmbeddingsReader::new(known_vectors(&model, &query_vector), known_texts());
    let service = SemanticSearchService::new(Arc::clone(&provider), repo);

    let hits = service
        .query("semantic topic", 4)
        .await
        .expect("semantic query should succeed");

    assert_eq!(hits.len(), 4);
    assert!(
        hits.windows(2)
            .all(|pair| pair[0].similarity >= pair[1].similarity)
    );
    assert_eq!(hits[0].chunk_id, chunk_id(0));
    assert_eq!(
        hits[0].snippet,
        "Chunk zero is exactly aligned with the query."
    );
}

#[tokio::test]
async fn top_k_zero_returns_empty_without_provider_or_repo_calls() {
    let provider = Arc::new(CountingProvider::new());
    let repo = InMemoryEmbeddingsReader::default();
    let observer = repo.clone();
    let service = SemanticSearchService::new(Arc::clone(&provider), repo);

    let hits = service
        .query("ignored", 0)
        .await
        .expect("zero top-k query should succeed");

    assert!(hits.is_empty());
    assert_eq!(provider.request_count(), 0);
    assert_eq!(observer.top_k_count(), 0);
}

#[tokio::test]
async fn empty_repo_returns_empty_hits() {
    let provider = Arc::new(CountingProvider::new());
    let repo = InMemoryEmbeddingsReader::default();
    let service = SemanticSearchService::new(Arc::clone(&provider), repo);

    let hits = service
        .query("no matches", 10)
        .await
        .expect("empty repo query should succeed");

    assert!(hits.is_empty());
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn top_k_greater_than_available_returns_all_hits() {
    let provider = Arc::new(CountingProvider::new());
    let model = provider.capability().model.clone();
    let records = vec![
        StoredVector::new(
            note_id(0),
            chunk_id(0),
            embedding_vector(&model, &[1.0, 0.0]),
        ),
        StoredVector::new(
            note_id(1),
            chunk_id(1),
            embedding_vector(&model, &[0.0, 1.0]),
        ),
    ];
    let repo = InMemoryEmbeddingsReader::new(records, known_texts());
    let service = SemanticSearchService::new(provider, repo);

    let hits = service
        .query("query", 10)
        .await
        .expect("semantic query should succeed");

    assert_eq!(hits.len(), 2);
}

#[tokio::test]
async fn snippet_truncates_at_sentence_boundary_when_possible() {
    let provider = Arc::new(CountingProvider::new());
    let model = provider.capability().model.clone();
    let chunk_id = chunk_id(0);
    let records = vec![StoredVector::new(
        note_id(0),
        chunk_id.clone(),
        embedding_vector(&model, &[1.0, 0.0]),
    )];
    let mut text_by_chunk = HashMap::new();
    text_by_chunk.insert(chunk_id, format!("{}. {}", "a".repeat(80), "b".repeat(180)));
    let service = SemanticSearchService::new(
        provider,
        InMemoryEmbeddingsReader::new(records, text_by_chunk),
    );

    let hits = service
        .query("query", 1)
        .await
        .expect("semantic query should succeed");

    assert_eq!(hits[0].snippet, format!("{}.", "a".repeat(80)));
}

#[test]
fn build_search_hits_assembles_snippets_for_blocking_callers() {
    let chunk_id = chunk_id(0);
    let top_hits = vec![TopHit {
        note_id: note_id(0),
        chunk_id: chunk_id.clone(),
        similarity: 0.75,
    }];
    let mut text_by_chunk = HashMap::new();
    text_by_chunk.insert(chunk_id.clone(), "  Shared snippet text.  ".to_string());

    let hits = build_search_hits(top_hits, &text_by_chunk).expect("hits should build");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk_id, chunk_id);
    assert!((hits[0].similarity - 0.75).abs() < f32::EPSILON);
    assert_eq!(hits[0].snippet, "Shared snippet text.");
}

#[test]
fn build_search_hits_rejects_missing_chunk_text() {
    let missing_chunk_id = chunk_id(0);
    let error = build_search_hits(
        vec![TopHit {
            note_id: note_id(0),
            chunk_id: missing_chunk_id.clone(),
            similarity: 0.75,
        }],
        &HashMap::new(),
    )
    .expect_err("missing chunk text should fail");

    assert!(matches!(
        error,
        AiError::Internal(message) if message.contains(&missing_chunk_id.to_string())
    ));
}

#[derive(Clone, Default)]
struct InMemoryEmbeddingsReader {
    records: Arc<Vec<StoredVector>>,
    text_by_chunk: Arc<HashMap<ChunkId, String>>,
    top_k_count: Arc<AtomicUsize>,
}

impl InMemoryEmbeddingsReader {
    fn new(records: Vec<StoredVector>, text_by_chunk: HashMap<ChunkId, String>) -> Self {
        Self {
            records: Arc::new(records),
            text_by_chunk: Arc::new(text_by_chunk),
            top_k_count: Arc::default(),
        }
    }

    fn top_k_count(&self) -> usize {
        self.top_k_count.load(Ordering::SeqCst)
    }
}

impl EmbeddingsReader for InMemoryEmbeddingsReader {
    fn top_k_cosine(
        &self,
        query: &EmbeddingVector,
        k: usize,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<TopHit>> {
        self.top_k_count.fetch_add(1, Ordering::SeqCst);
        let mut hits = self
            .records
            .iter()
            .filter(|record| record.vector.model == *model && record.vector.dim() == query.dim())
            .map(|record| TopHit {
                note_id: record.note_id.clone(),
                chunk_id: record.chunk_id.clone(),
                similarity: record
                    .vector
                    .cosine_similarity(query)
                    .expect("fixture vector dimensions should match"),
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| right.similarity.total_cmp(&left.similarity));
        hits.truncate(k);

        Ok(hits)
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
struct StoredVector {
    note_id: NoteId,
    chunk_id: ChunkId,
    vector: EmbeddingVector,
}

impl StoredVector {
    fn new(note_id: NoteId, chunk_id: ChunkId, vector: EmbeddingVector) -> Self {
        Self {
            note_id,
            chunk_id,
            vector,
        }
    }
}

struct CountingProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
    request_count: AtomicUsize,
    requests: Mutex<Vec<EmbeddingsRequest>>,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            id: provider_id(),
            capability: EmbeddingsCapability {
                model: model(),
                dim: 2,
                max_batch_size: 8,
                max_input_tokens: 128,
            },
            request_count: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EmbeddingsProvider for CountingProvider {
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
            .expect("mock requests lock should not be poisoned")
            .push(request.clone());

        Ok(EmbeddingsResponse {
            vectors: request
                .texts
                .iter()
                .map(|_| embedding_vector(&request.model, &[2.0, 0.0]))
                .collect(),
        })
    }
}

fn known_vectors(model: &EmbeddingModelId, query: &EmbeddingVector) -> Vec<StoredVector> {
    let mut opposite = query.clone();
    for value in &mut opposite.values {
        *value = -*value;
    }

    let mut records = vec![StoredVector::new(note_id(0), chunk_id(0), query.clone())];
    records.extend((1_u32..10).map(|ordinal| {
        let ordinal_value = f32::from(u16::try_from(ordinal).expect("ordinal should fit u16"));
        let vector = if ordinal == 9 {
            opposite.clone()
        } else {
            embedding_vector(model, &[10.0 - ordinal_value, ordinal_value])
        };
        StoredVector::new(note_id(ordinal), chunk_id(ordinal), vector)
    }));
    records
}

fn known_texts() -> HashMap<ChunkId, String> {
    (0_u32..10)
        .map(|ordinal| {
            let text = if ordinal == 0 {
                "Chunk zero is exactly aligned with the query.".to_string()
            } else {
                format!("Chunk {ordinal} is exactly aligned with the query.")
            };
            (chunk_id(ordinal), text)
        })
        .collect()
}

fn embedding_vector(model: &EmbeddingModelId, values: &[f32]) -> EmbeddingVector {
    let mut vector = EmbeddingVector {
        values: values.to_vec(),
        model: model.clone(),
    };
    vector.normalize_in_place();
    vector
}

fn note_id(index: u32) -> NoteId {
    NoteId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
}

fn chunk_id(index: u32) -> ChunkId {
    ChunkId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
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
