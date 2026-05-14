// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_ai::{
    AiError,
    chunker::{ChunkerOptions, ParagraphChunker},
};
use zk_domain::id::{ContentHash, NoteId};

#[test]
fn empty_body_returns_no_chunks() {
    let chunker = ParagraphChunker::default();

    let chunks = chunker
        .chunk(&note_id(), " \n\n\t ")
        .expect("empty body should chunk successfully");

    assert!(chunks.is_empty());
}

#[test]
fn single_short_paragraph_returns_one_chunk() {
    let note_id = note_id();
    let chunker = ParagraphChunker::default();
    let body = "A short paragraph for embedding.";

    let chunks = chunker
        .chunk(&note_id, body)
        .expect("short paragraph should chunk successfully");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].note_id, note_id);
    assert_eq!(chunks[0].ordinal, 0);
    assert_eq!(chunks[0].text, body);
    assert_eq!(
        chunks[0].content_hash,
        ContentHash::from_bytes(body.as_bytes())
    );
    assert_eq!(chunks[0].token_count, Some(6));
}

#[test]
fn paragraph_overflow_emits_multiple_chunks_with_overlap() {
    let chunker = ParagraphChunker::new(ChunkerOptions {
        max_tokens: 20,
        overlap_tokens: 4,
        respect_headings: true,
    });
    let body = [
        "alpha beta gamma delta epsilon zeta eta theta iota",
        "kappa lambda mu nu xi omicron pi rho sigma",
        "tau upsilon phi chi psi omega alpha beta gamma",
    ]
    .join("\n\n");

    let chunks = chunker
        .chunk(&note_id(), &body)
        .expect("paragraphs should chunk successfully");

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].ordinal, 0);
    assert_eq!(chunks[1].ordinal, 1);
    assert_eq!(chunks[2].ordinal, 2);
    assert!(chunks[1].text.contains("eta theta iota"));
    assert!(chunks[1].text.contains("kappa lambda mu"));
    assert!(chunks[2].text.contains("pi rho sigma"));
    assert!(chunks[2].text.contains("tau upsilon phi"));
}

#[test]
fn heading_respecting_mode_keeps_heading_lines_at_chunk_boundaries() {
    let chunker = ParagraphChunker::new(ChunkerOptions {
        max_tokens: 12,
        overlap_tokens: 0,
        respect_headings: true,
    });
    let body = "# Title\n\nalpha beta gamma delta epsilon\n\n# Next\n\nzeta eta theta";

    let chunks = chunker
        .chunk(&note_id(), body)
        .expect("heading sections should chunk successfully");

    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].text.starts_with("# Title"));
    assert!(chunks[1].text.starts_with("# Next"));
    assert!(!chunks[0].text.contains("\n\n# Next"));
}

#[test]
fn disabled_heading_respect_treats_headings_as_normal_paragraphs() {
    let chunker = ParagraphChunker::new(ChunkerOptions {
        max_tokens: 12,
        overlap_tokens: 0,
        respect_headings: false,
    });
    let body = "# Title\n\nalpha beta gamma delta epsilon\n\n# Next\n\nzeta eta theta";

    let chunks = chunker
        .chunk(&note_id(), body)
        .expect("plain paragraph mode should chunk successfully");

    assert!(chunks[0].text.contains("\n\n# Next"));
}

#[test]
fn oversized_single_paragraph_splits_on_sentence_boundaries() {
    let chunker = ParagraphChunker::new(ChunkerOptions {
        max_tokens: 3,
        overlap_tokens: 0,
        respect_headings: true,
    });
    let body = "First sentence. Second sentence. Third sentence.";

    let chunks = chunker
        .chunk(&note_id(), body)
        .expect("oversized paragraph should split successfully");

    assert_eq!(
        chunk_texts(&chunks),
        vec!["First sentence.", "Second sentence.", "Third sentence."]
    );
}

#[test]
fn deterministic_fields_match_across_runs() {
    let chunker = ParagraphChunker::new(ChunkerOptions {
        max_tokens: 8,
        overlap_tokens: 2,
        respect_headings: true,
    });
    let note_id = note_id();
    let body = "# Title\n\nFirst sentence. Second sentence.\n\nThird paragraph here.";

    let first = chunker
        .chunk(&note_id, body)
        .expect("first run should chunk successfully");
    let second = chunker
        .chunk(&note_id, body)
        .expect("second run should chunk successfully");

    assert_eq!(deterministic_fields(&first), deterministic_fields(&second));
}

#[test]
fn zero_max_tokens_returns_invalid_config() {
    let chunker = ParagraphChunker::new(ChunkerOptions {
        max_tokens: 0,
        overlap_tokens: 0,
        respect_headings: true,
    });

    let result = chunker.chunk(&note_id(), "body");

    assert!(
        matches!(result, Err(AiError::InvalidConfig(message)) if message == "max_tokens must be greater than zero")
    );
}

fn deterministic_fields(
    chunks: &[zk_domain::ChunkRecord],
) -> Vec<(String, u32, String, Option<u32>)> {
    chunks
        .iter()
        .map(|chunk| {
            (
                chunk.text.clone(),
                chunk.ordinal,
                chunk.content_hash.to_string(),
                chunk.token_count,
            )
        })
        .collect()
}

fn chunk_texts(chunks: &[zk_domain::ChunkRecord]) -> Vec<&str> {
    chunks.iter().map(|chunk| chunk.text.as_str()).collect()
}

fn note_id() -> NoteId {
    NoteId::new("01JAR5A3PJEZ3SKG67PN8N6S6S")
}
