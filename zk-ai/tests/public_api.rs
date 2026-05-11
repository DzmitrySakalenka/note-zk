// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::any::{type_name, type_name_of_val};

use zk_ai::{
    AppendChatMessageParams, AssembleParams, ChatConversationsRepo, ChatMessageCitationsRepo,
    ChatMessagesRepo, ChatProviderResolver, ChatService, ChatServiceRepos, ChatTurnRequest,
    ChunkOutcome, Chunker, ChunkerOptions, ChunkingService, ChunksWriter, CitationFormat,
    EmbeddingsCapability, EmbeddingsProvider, EmbeddingsReader, EmbeddingsRequest,
    EmbeddingsResponse, EmbeddingsService, EmbeddingsServiceOptions, EmbeddingsWriter,
    ParagraphChunker, PromptAssembler, PromptConfig, ProviderRegistry, ProviderSelector,
    RecomputeOutcome, RetrievalParams, RetrievalService, SearchHit, SemanticSearchService,
    StubEmbeddingsProvider, embed_one,
};
use zk_domain::{
    ChunkEmbeddingRecord, ChunkId, ChunkRecord, EmbeddingModelId, EmbeddingVector, id::NoteId,
};

#[test]
fn ai_operator_public_api_is_available_from_crate_root() {
    assert!(!type_name::<ChunkerOptions>().is_empty());
    assert!(!type_name::<ParagraphChunker>().is_empty());
    assert!(!type_name::<dyn Chunker>().is_empty());
    assert!(!type_name::<dyn EmbeddingsProvider>().is_empty());
    assert!(!type_name::<EmbeddingsRequest>().is_empty());
    assert!(!type_name::<EmbeddingsResponse>().is_empty());
    assert!(!type_name::<EmbeddingsCapability>().is_empty());
    assert!(!type_name::<StubEmbeddingsProvider>().is_empty());
    assert!(!type_name::<ProviderRegistry>().is_empty());
    assert!(!type_name::<ProviderSelector>().is_empty());
    assert!(!type_name::<ChunkingService<ParagraphChunker, NoopChunksWriter>>().is_empty());
    assert!(!type_name::<dyn ChunksWriter>().is_empty());
    assert!(!type_name::<ChunkOutcome>().is_empty());
    assert!(
        !type_name::<EmbeddingsService<StubEmbeddingsProvider, NoopEmbeddingsRepo>>().is_empty()
    );
    assert!(!type_name::<dyn EmbeddingsWriter>().is_empty());
    assert!(!type_name::<EmbeddingsServiceOptions>().is_empty());
    assert!(!type_name::<RecomputeOutcome>().is_empty());
    assert!(!type_name::<RetrievalParams>().is_empty());
    assert!(!type_name::<RetrievalService>().is_empty());
    assert!(!type_name::<PromptConfig>().is_empty());
    assert!(!type_name::<CitationFormat>().is_empty());
    assert!(!type_name::<AssembleParams<'static>>().is_empty());
    assert!(!type_name::<PromptAssembler>().is_empty());
    assert!(!type_name::<ChatTurnRequest>().is_empty());
    assert!(!type_name::<AppendChatMessageParams>().is_empty());
    assert!(!type_name::<ChatService>().is_empty());
    assert!(!type_name::<ChatServiceRepos>().is_empty());
    assert!(!type_name::<dyn ChatConversationsRepo>().is_empty());
    assert!(!type_name::<dyn ChatMessagesRepo>().is_empty());
    assert!(!type_name::<dyn ChatMessageCitationsRepo>().is_empty());
    assert!(!type_name::<dyn ChatProviderResolver>().is_empty());
    assert!(
        !type_name::<SemanticSearchService<StubEmbeddingsProvider, NoopEmbeddingsRepo>>()
            .is_empty()
    );
    assert!(!type_name::<dyn EmbeddingsReader>().is_empty());
    assert!(!type_name::<SearchHit>().is_empty());

    assert!(!type_name_of_val(&embed_one::<StubEmbeddingsProvider>).is_empty());
}

#[cfg(feature = "provider-http")]
#[test]
fn remote_provider_public_api_is_available_from_crate_root() {
    use zk_ai::{RemoteEmbeddingsConfig, RemoteEmbeddingsProvider};

    assert!(!type_name::<RemoteEmbeddingsProvider>().is_empty());
    assert!(!type_name::<RemoteEmbeddingsConfig>().is_empty());
}

#[cfg(feature = "provider-local")]
#[test]
fn local_provider_public_api_is_available_from_crate_root() {
    use zk_ai::{LocalEmbeddingsConfig, LocalEmbeddingsProvider};

    assert!(!type_name::<LocalEmbeddingsProvider>().is_empty());
    assert!(!type_name::<LocalEmbeddingsConfig>().is_empty());
}

#[derive(Debug, Clone)]
struct NoopChunksWriter;

impl ChunksWriter for NoopChunksWriter {
    fn list_existing_hashes(&self, _note_id: &NoteId) -> zk_ai::AiResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn replace_for_note(&self, _note_id: &NoteId, chunks: &[ChunkRecord]) -> zk_ai::AiResult<u64> {
        Ok(chunks.len() as u64)
    }
}

#[derive(Debug, Clone)]
struct NoopEmbeddingsRepo;

impl EmbeddingsWriter for NoopEmbeddingsRepo {
    fn list_chunks_missing_embeddings(
        &self,
        _note_id: &NoteId,
        _model: &EmbeddingModelId,
    ) -> zk_ai::AiResult<Vec<ChunkRecord>> {
        Ok(Vec::new())
    }

    fn upsert_embeddings(&self, records: &[ChunkEmbeddingRecord]) -> zk_ai::AiResult<u64> {
        Ok(records.len() as u64)
    }
}

impl EmbeddingsReader for NoopEmbeddingsRepo {
    fn top_k_cosine(
        &self,
        _query: &EmbeddingVector,
        _k: usize,
        _model: &EmbeddingModelId,
    ) -> zk_ai::AiResult<Vec<zk_ai::service::semantic_search::TopHit>> {
        Ok(Vec::new())
    }

    fn list_chunk_text(&self, chunk_ids: &[ChunkId]) -> zk_ai::AiResult<Vec<(ChunkId, String)>> {
        Ok(chunk_ids
            .iter()
            .cloned()
            .map(|chunk_id| (chunk_id, String::new()))
            .collect())
    }
}
