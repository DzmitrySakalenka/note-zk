// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{fs, sync::Arc, time::Duration};

use serde_json::json;
use tempfile::TempDir;
use tokio::time::advance;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};
use zk_ai::{
    AiError,
    provider::{EmbeddingsProvider, StubEmbeddingsProvider, embed_one},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_app::{
    AiFacade, AiFacadeDeps, LocalEmbeddingsRepoHandle, RecomputeLocalSummary, SemanticSearchRequest,
};
use zk_domain::{
    ChunkEmbeddingRecord, ChunkId, ChunkRecord, ExecutionMode, ProviderId, ProviderSelectorPolicy,
    id::{ContentHash, NoteId, RelativePath},
};
use zk_index::{ChunksRepo, EmbeddingsRepo, NoteRepo, UpsertNoteParams};
use zk_sync::HttpTransport;

const QUERY_TEXT: &str = "semantic topic";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const FIRST_CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";

#[tokio::test(flavor = "current_thread")]
async fn connected_mode_returns_hits_from_http_transport() {
    let server = MockServer::start().await;
    let facade = AiFacade::new(AiFacadeDeps {
        http: Some(Arc::new(http_transport(&server, "shared-token"))),
        local_selector: None,
        local_repo: None,
        mode: ExecutionMode::Connected,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct");

    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .and(header("authorization", "Bearer shared-token"))
        .and(body_json(json!({ "query": QUERY_TEXT, "top_k": 3 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": [
                {
                    "note_id": NOTE_ID,
                    "chunk_id": FIRST_CHUNK_ID,
                    "similarity": 0.9,
                    "snippet": "first hit"
                },
                {
                    "note_id": "01JAR5A3PJEZ3SKG67PN8N6S6Q",
                    "chunk_id": "01JAR5A3PJEZ3SKG67PN8N6S6P",
                    "similarity": 0.8,
                    "snippet": "second hit"
                },
                {
                    "note_id": "01JAR5A3PJEZ3SKG67PN8N6S6N",
                    "chunk_id": "01JAR5A3PJEZ3SKG67PN8N6S6M",
                    "similarity": 0.7,
                    "snippet": "third hit"
                }
            ],
            "query_dim": 2,
            "model": "stub/sha256-2"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let hits = facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 3,
            mode_override: None,
        })
        .await
        .expect("connected semantic search should succeed");

    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].note_id, NoteId::new(NOTE_ID));
    assert_eq!(hits[0].chunk_id, ChunkId::new(FIRST_CHUNK_ID));
    assert!((hits[0].similarity - 0.9).abs() < f32::EPSILON);
    assert_eq!(hits[0].snippet, "first hit");
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn hybrid_mode_falls_back_to_local_repo_when_http_fails() {
    let server = MockServer::start().await;
    let local_repo = seeded_local_repo().await;
    let facade = hybrid_facade(&server, local_repo);

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(ResponseTemplate::new(500).set_body_string("provider failed"))
        .expect(1)
        .mount(&server)
        .await;

    let hits = facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect("hybrid semantic search should fall back");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].note_id, NoteId::new(NOTE_ID));
    assert_eq!(hits[0].chunk_id, ChunkId::new(FIRST_CHUNK_ID));
    assert_eq!(hits[0].snippet, "Exact semantic topic sentence.");
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn hybrid_mode_probe_success_routes_to_connected_path() {
    let server = MockServer::start().await;
    let local_repo = seeded_local_repo().await;
    let facade = hybrid_facade(&server, local_repo);

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(single_remote_hit_response("connected hit"))
        .expect(1)
        .mount(&server)
        .await;

    let hits = facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect("hybrid semantic search should use connected path");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].snippet, "connected hit");
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn hybrid_mode_probe_failure_routes_to_offline_path() {
    let server = MockServer::start().await;
    let local_repo = seeded_local_repo().await;
    let facade = hybrid_facade(&server, local_repo);

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(single_remote_hit_response("unexpected connected hit"))
        .expect(0)
        .mount(&server)
        .await;

    let hits = facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect("hybrid semantic search should use offline path");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].snippet, "Exact semantic topic sentence.");
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn hybrid_mode_reuses_probe_cache_within_ten_seconds() {
    let server = MockServer::start().await;
    let local_repo = seeded_local_repo().await;
    let facade = hybrid_facade(&server, local_repo);

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(single_remote_hit_response("connected hit"))
        .expect(2)
        .mount(&server)
        .await;

    for _ in 0..2 {
        facade
            .semantic_search(SemanticSearchRequest {
                query: QUERY_TEXT.to_string(),
                top_k: 1,
                mode_override: None,
            })
            .await
            .expect("hybrid semantic search should use connected path");
    }

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn hybrid_mode_refreshes_probe_cache_after_ten_seconds() {
    let server = MockServer::start().await;
    let local_repo = seeded_local_repo().await;
    let facade = hybrid_facade(&server, local_repo);

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(single_remote_hit_response("connected hit"))
        .expect(2)
        .mount(&server)
        .await;

    facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect("first hybrid semantic search should use connected path");

    tokio::time::pause();
    advance(Duration::from_secs(11)).await;
    tokio::time::resume();

    facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect("second hybrid semantic search should refresh probe");

    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn offline_mode_does_not_call_http_transport() {
    let server = MockServer::start().await;
    let local_repo = seeded_local_repo().await;
    let facade = AiFacade::new(AiFacadeDeps {
        http: Some(Arc::new(http_transport(&server, "unused-token"))),
        local_selector: Some(stub_selector()),
        local_repo: Some(local_repo),
        mode: ExecutionMode::Offline,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct");

    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let hits = facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect("offline semantic search should succeed");

    assert_eq!(hits.len(), 1);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn connected_mode_without_http_returns_provider_unavailable() {
    let facade = AiFacade::new(AiFacadeDeps {
        http: None,
        local_selector: Some(stub_selector()),
        local_repo: Some(LocalEmbeddingsRepoHandle::open_memory().expect("repo should open")),
        mode: ExecutionMode::Connected,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct");

    let error = facade
        .semantic_search(SemanticSearchRequest {
            query: QUERY_TEXT.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect_err("connected semantic search should require HTTP");

    assert!(matches!(error, AiError::ProviderUnavailable(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn recompute_local_populates_local_chunks_and_embeddings_for_fixture_vault() {
    let vault = fixture_vault();
    let local_repo = LocalEmbeddingsRepoHandle::open_memory().expect("repo should open");
    let facade = AiFacade::new(AiFacadeDeps {
        http: None,
        local_selector: Some(stub_selector()),
        local_repo: Some(local_repo.clone()),
        mode: ExecutionMode::Offline,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct");

    let summary = facade
        .recompute_local(vault.path())
        .await
        .expect("local recompute should succeed");

    assert_eq!(
        summary,
        RecomputeLocalSummary {
            notes: 3,
            chunks: summary.chunks,
            embeddings: summary.embeddings,
            errors: Vec::new(),
        }
    );
    assert!(summary.chunks > 0);
    assert!(summary.embeddings > 0);

    let (note_count, chunk_count, embedding_count) = local_repo
        .with_index(|connection| {
            Ok((
                NoteRepo::count(connection)?,
                count_rows(connection, "note_chunks")?,
                count_rows(connection, "chunk_embeddings")?,
            ))
        })
        .expect("local counts should query");
    assert_eq!(note_count, 3);
    assert_eq!(chunk_count, summary.chunks);
    assert_eq!(embedding_count, summary.embeddings);
}

async fn seeded_local_repo() -> LocalEmbeddingsRepoHandle {
    let repo = LocalEmbeddingsRepoHandle::open_memory().expect("repo should open");
    let provider = StubEmbeddingsProvider::new(2);
    let model = provider.capability().model.clone();
    let query_vector = embed_one(&provider, QUERY_TEXT.to_string(), model.clone())
        .await
        .expect("stub provider should embed query");
    let note_id = NoteId::new(NOTE_ID);
    let chunk = ChunkRecord {
        chunk_id: ChunkId::new(FIRST_CHUNK_ID),
        note_id: note_id.clone(),
        ordinal: 0,
        content_hash: ContentHash::from_bytes(b"Exact semantic topic sentence."),
        text: "Exact semantic topic sentence.".to_string(),
        token_count: Some(4),
        created_at: chrono::Utc::now(),
    };
    let embedding = ChunkEmbeddingRecord {
        chunk_id: chunk.chunk_id.clone(),
        model,
        provider: provider_id("stub"),
        vector: query_vector,
        created_at: chrono::Utc::now(),
    };

    repo.with_index(|connection| {
        NoteRepo::upsert_note(
            connection,
            UpsertNoteParams {
                note_id: &note_id,
                path: &RelativePath::new("notes/topic.md"),
                title: "Semantic Topic",
                content_hash: &ContentHash::from_bytes(b"topic"),
                frontmatter_json: r#"{"id":"01JAR5A3PJEZ3SKG67PN8N6S6S","title":"Semantic Topic"}"#,
                created_at: "2026-05-05T10:00:00Z",
                updated_at: "2026-05-05T10:00:00Z",
                trashed_at: None,
                is_trashed: false,
            },
        )?;
        ChunksRepo::new(connection).replace_for_note(&note_id, &[chunk])?;
        EmbeddingsRepo::new(connection).upsert(&embedding)?;

        Ok(())
    })
    .expect("local repo should seed");

    repo
}

fn fixture_vault() -> TempDir {
    let vault = TempDir::new().expect("fixture vault should create");

    for ordinal in 0..3 {
        let note_id = format!("01JAR5A3PJEZ3SKG67PN8N6S6{ordinal}");
        let path = vault.path().join(format!("note-{ordinal}.md"));
        fs::write(
            path,
            format!(
                "---\nid: {note_id}\ntitle: Note {ordinal}\n---\n# Note {ordinal}\n\nSemantic body {ordinal} with enough text for one chunk.\n"
            ),
        )
        .expect("fixture note should write");
    }

    vault
}

fn stub_selector() -> ProviderSelector {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(2)));

    ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Offline,
            remote_provider: None,
            local_provider: Some(provider_id("stub")),
            stub_provider: Some(provider_id("stub")),
        },
    )
}

fn http_transport(server: &MockServer, token: &str) -> HttpTransport {
    HttpTransport::new(
        server.uri().parse().expect("mock server URL should parse"),
        Some(token.to_string()),
    )
    .expect("HTTP transport should construct")
}

fn hybrid_facade(server: &MockServer, local_repo: LocalEmbeddingsRepoHandle) -> AiFacade {
    AiFacade::new(AiFacadeDeps {
        http: Some(Arc::new(http_transport(server, "shared-token"))),
        local_selector: Some(stub_selector()),
        local_repo: Some(local_repo),
        mode: ExecutionMode::Hybrid,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct")
}

fn single_remote_hit_response(snippet: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "hits": [
            {
                "note_id": NOTE_ID,
                "chunk_id": FIRST_CHUNK_ID,
                "similarity": 0.9,
                "snippet": snippet
            }
        ],
        "query_dim": 2,
        "model": "stub/sha256-2"
    }))
}

fn provider_id(value: &str) -> ProviderId {
    value.parse().expect("provider id should parse")
}

fn count_rows(connection: &rusqlite::Connection, table: &str) -> Result<u32, zk_index::IndexError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count = connection
        .query_row(&sql, [], |row| row.get::<_, u32>(0))
        .map_err(|source| zk_index::IndexError::Sqlite {
            source,
            context: format!("count {table} rows"),
        })?;

    Ok(count)
}
