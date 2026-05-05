// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use zk_ai::{
    AiResult,
    chunker::{ChunkerOptions, ParagraphChunker},
    service::chunking::{ChunkOutcome, ChunkingService, ChunksWriter},
};
use zk_domain::{ChunkRecord, id::NoteId};

#[test]
fn first_call_replaces_chunks() {
    let writer = MockChunksWriter::default();
    let observer = writer.clone();
    let service = ChunkingService::new(chunker(), writer);
    let note_id = note_id();

    let outcome = service
        .chunk_note(&note_id, "First paragraph.")
        .expect("first chunking pass should succeed");

    assert_eq!(outcome, ChunkOutcome::Replaced { chunk_count: 1 });
    assert_eq!(observer.replace_count(), 1);
    assert_eq!(observer.stored_texts(&note_id), vec!["First paragraph."]);
}

#[test]
fn second_call_with_same_body_is_unchanged() {
    let writer = MockChunksWriter::default();
    let observer = writer.clone();
    let service = ChunkingService::new(chunker(), writer);
    let note_id = note_id();

    service
        .chunk_note(&note_id, "Stable body.")
        .expect("first chunking pass should succeed");
    let outcome = service
        .chunk_note(&note_id, "Stable body.")
        .expect("second chunking pass should succeed");

    assert_eq!(outcome, ChunkOutcome::Unchanged { chunk_count: 1 });
    assert_eq!(observer.replace_count(), 1);
    assert_eq!(observer.stored_texts(&note_id), vec!["Stable body."]);
}

#[test]
fn edited_body_replaces_old_chunks() {
    let writer = MockChunksWriter::default();
    let observer = writer.clone();
    let service = ChunkingService::new(chunker(), writer);
    let note_id = note_id();

    service
        .chunk_note(&note_id, "Original body.")
        .expect("first chunking pass should succeed");
    let outcome = service
        .chunk_note(&note_id, "Edited body.")
        .expect("edited chunking pass should succeed");

    assert_eq!(outcome, ChunkOutcome::Replaced { chunk_count: 1 });
    assert_eq!(observer.replace_count(), 2);
    assert_eq!(observer.stored_texts(&note_id), vec!["Edited body."]);
}

#[derive(Clone, Default)]
struct MockChunksWriter {
    chunks: Arc<Mutex<Vec<ChunkRecord>>>,
    replace_count: Arc<AtomicUsize>,
}

impl MockChunksWriter {
    fn stored_texts(&self, note_id: &NoteId) -> Vec<String> {
        self.chunks
            .lock()
            .expect("mock chunks lock should not be poisoned")
            .iter()
            .filter(|chunk| chunk.note_id == *note_id)
            .map(|chunk| chunk.text.clone())
            .collect()
    }

    fn replace_count(&self) -> usize {
        self.replace_count.load(Ordering::SeqCst)
    }
}

impl ChunksWriter for MockChunksWriter {
    fn list_existing_hashes(&self, note_id: &NoteId) -> AiResult<Vec<String>> {
        Ok(self
            .chunks
            .lock()
            .expect("mock chunks lock should not be poisoned")
            .iter()
            .filter(|chunk| chunk.note_id == *note_id)
            .map(|chunk| chunk.content_hash.to_string())
            .collect())
    }

    fn replace_for_note(&self, note_id: &NoteId, chunks: &[ChunkRecord]) -> AiResult<u64> {
        let mut stored_chunks = self
            .chunks
            .lock()
            .expect("mock chunks lock should not be poisoned");
        stored_chunks.retain(|chunk| chunk.note_id != *note_id);
        stored_chunks.extend_from_slice(chunks);
        self.replace_count.fetch_add(1, Ordering::SeqCst);

        Ok(u64::try_from(chunks.len()).expect("chunk count should fit u64"))
    }
}

fn chunker() -> ParagraphChunker {
    ParagraphChunker::new(ChunkerOptions {
        max_tokens: 20,
        overlap_tokens: 0,
        respect_headings: true,
    })
}

fn note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S")
}
