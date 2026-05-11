// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{TimeZone, Utc};
use insta::assert_snapshot;
use zk_ai::{
    AiError,
    service::prompt::{AssembleParams, CitationFormat, PromptAssembler, PromptConfig},
};
use zk_domain::{
    ChunkId, ConversationId, EmbeddingModelId, MessageId, MessageRecord, MessageRole,
    RetrievalChunk, RetrievalContext, id::NoteId,
};

#[test]
fn empty_history_with_retrieved_chunks_matches_snapshot() {
    let assembler = PromptAssembler::new(PromptConfig {
        system_prompt: "Use the vault context before answering.".to_string(),
        citation_format: CitationFormat::EndOfMessage,
        max_input_tokens: 4096,
    });

    let messages = assembler
        .assemble(AssembleParams {
            history: &[],
            user_query: "How should I connect alpha and beta?",
            retrieval: &retrieval_context(&[
                (
                    0,
                    0,
                    "Alpha note explains the local-first sync boundary.",
                    0.91,
                ),
                (1, 1, "Beta note covers prompt determinism.", 0.87),
                (2, 2, "Gamma note lists open follow-up work.", 0.76),
            ]),
        })
        .expect("prompt assembly should succeed");

    assert_snapshot!(
        "empty_history_with_retrieved_chunks",
        render_messages(&messages)
    );
}

#[test]
fn history_truncation_matches_snapshot() {
    let assembler = PromptAssembler::new(PromptConfig {
        system_prompt: "Use retrieved notes and keep the answer grounded.".to_string(),
        citation_format: CitationFormat::EndOfMessage,
        max_input_tokens: 115,
    });
    let history = vec![
        history_message(
            0,
            MessageRole::User,
            "First old user message that should be dropped.",
        ),
        history_message(
            1,
            MessageRole::Assistant,
            "First old assistant answer that should also be dropped.",
        ),
        history_message(
            2,
            MessageRole::User,
            "Second user message stays if budget allows.",
        ),
        history_message(
            3,
            MessageRole::Assistant,
            "Second assistant answer stays if budget allows.",
        ),
        history_message(
            4,
            MessageRole::User,
            "Most recent user clarification stays.",
        ),
    ];
    let retrieval = retrieval_context(&[
        (0, 0, "Delta note gives the decisive detail.", 0.93),
        (1, 1, "Epsilon note provides supporting context.", 0.82),
    ]);

    let messages = assembler
        .assemble(AssembleParams {
            history: &history,
            user_query: "Answer using the remaining useful context.",
            retrieval: &retrieval,
        })
        .expect("prompt assembly should truncate history");

    assert_snapshot!("history_truncation", render_messages(&messages));
}

#[test]
fn over_budget_query_returns_typed_error() {
    let assembler = PromptAssembler::new(PromptConfig {
        system_prompt: "Use context.".to_string(),
        citation_format: CitationFormat::EndOfMessage,
        max_input_tokens: 8,
    });

    let error = assembler
        .assemble(AssembleParams {
            history: &[],
            user_query: "This query is intentionally too long to fit after history truncation.",
            retrieval: &retrieval_context(&[(0, 0, "Short context.", 0.5)]),
        })
        .expect_err("over-budget prompt should fail");

    assert!(matches!(
        error,
        AiError::InvalidConfig(message)
            if message == "prompt exceeds max_input_tokens after history truncation"
    ));
}

#[test]
fn inline_citation_format_is_reserved_for_post_v1() {
    let assembler = PromptAssembler::new(PromptConfig {
        system_prompt: "Use context.".to_string(),
        citation_format: CitationFormat::Inline,
        max_input_tokens: 4096,
    });

    let error = assembler
        .assemble(AssembleParams {
            history: &[],
            user_query: "Question?",
            retrieval: &retrieval_context(&[]),
        })
        .expect_err("inline citation format should be reserved");

    assert!(matches!(
        error,
        AiError::InvalidConfig(message)
            if message == "inline citation format is reserved for post-V1"
    ));
}

fn render_messages(messages: &[zk_ai::ChatMessage]) -> String {
    serde_json::to_string_pretty(messages).expect("messages should serialize")
}

fn history_message(index: u32, role: MessageRole, content: &str) -> MessageRecord {
    MessageRecord {
        message_id: MessageId::new(&format!("message-{index}")),
        conversation_id: ConversationId::new("conversation-1"),
        role,
        content: content.to_string(),
        token_count: None,
        created_at: Utc
            .with_ymd_and_hms(2026, 5, 8, 12, index, 0)
            .single()
            .expect("fixture timestamp should be valid"),
        server_revision: None,
    }
}

fn retrieval_context(entries: &[(u32, u32, &str, f32)]) -> RetrievalContext {
    let model = model();
    let chunks = entries
        .iter()
        .map(
            |(note_index, chunk_index, text, similarity)| RetrievalChunk {
                chunk_id: chunk_id(*chunk_index),
                note_id: note_id(*note_index),
                text: (*text).to_string(),
                similarity: *similarity,
                token_estimate: u32::try_from(text.len() / 4)
                    .expect("fixture token estimate should fit u32"),
            },
        )
        .collect::<Vec<_>>();
    let total_token_estimate = chunks.iter().map(|chunk| chunk.token_estimate).sum();

    RetrievalContext {
        chunks,
        total_token_estimate,
        model,
    }
}

fn note_id(index: u32) -> NoteId {
    NoteId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
}

fn chunk_id(index: u32) -> ChunkId {
    ChunkId::new(&format!("01JAR5A3PJEZ3SKG67PN8N6S6{index}"))
}

fn model() -> EmbeddingModelId {
    "test/prompt-model"
        .parse()
        .expect("embedding model fixture should be valid")
}
