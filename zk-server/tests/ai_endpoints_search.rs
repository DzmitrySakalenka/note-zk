// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf};

mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use chrono::{DateTime, Utc};
use common::auth_fixtures::{RequestAuth, assert_json_error, json_request, response_body};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_ai::provider::{EmbeddingsProvider, StubEmbeddingsProvider, embed_one};
use zk_domain::{
    AuthScope, ChunkEmbeddingRecord, ChunkId, ChunkRecord, EmbeddingModelId, EmbeddingVector,
    ExecutionMode, ProviderId,
    id::{ContentHash, NoteId},
};
use zk_server::{
    config::{AiConfig, AiProviderKind, ServerConfig, StubEmbeddingsConfig},
    http::build_router,
    repo::{ChunksRepo, EmbeddingsRepo, NoteRow, NotesRepo},
    service::{self, IssueTokenSpec},
    state::{ServerState, bootstrap_with_config},
};

const SEARCH_PATH: &str = "/search/semantic";
const QUERY_TEXT: &str = "semantic topic";
const NOTE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const FIRST_CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6R";
const SECOND_CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6Q";
const THIRD_CHUNK_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6P";

#[derive(Debug, Deserialize)]
struct SemanticSearchResponse {
    hits: Vec<SearchHitDto>,
    query_dim: usize,
    model: String,
}

#[derive(Debug, Deserialize)]
struct SearchHitDto {
    note_id: String,
    chunk_id: String,
    similarity: f32,
    snippet: String,
}

struct SearchFixture {
    _database_file: NamedTempFile,
    state: ServerState,
}

impl SearchFixture {
    fn enabled() -> Self {
        Self::new(Some(ai_config(ExecutionMode::Hybrid)))
    }

    fn offline_without_local() -> Self {
        Self::new(Some(ai_config(ExecutionMode::Offline)))
    }

    fn disabled() -> Self {
        Self::new(None)
    }

    fn issue_token(&self, scopes: Vec<AuthScope>) -> String {
        self.state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "Semantic search test token".to_string(),
                        device_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued")
            .plaintext
    }

    fn new(ai: Option<AiConfig>) -> Self {
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let state = bootstrap_with_config(test_config(database_file.path().to_path_buf(), ai))
            .expect("server state should bootstrap");

        Self {
            _database_file: database_file,
            state,
        }
    }
}

#[tokio::test]
async fn semantic_search_auth_accepts_sync_and_admin_scopes() {
    let fixture = SearchFixture::enabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);
    let admin_token = fixture.issue_token(vec![AuthScope::Admin]);
    let read_token = fixture.issue_token(vec![AuthScope::Read]);
    let app = build_router(fixture.state.clone());

    let sync_response = search(&app, &sync_token, &json!({ "query": QUERY_TEXT })).await;
    let admin_response = search(&app, &admin_token, &json!({ "query": QUERY_TEXT })).await;

    assert!(sync_response.hits.is_empty());
    assert!(admin_response.hits.is_empty());

    let read_response = app
        .clone()
        .oneshot(search_request(&read_token, &json!({ "query": QUERY_TEXT })))
        .await
        .expect("semantic search request should complete");
    assert_json_error(read_response, StatusCode::FORBIDDEN, "forbidden", "sync").await;

    let no_auth_response = app
        .oneshot(json_request(
            Method::POST,
            SEARCH_PATH,
            &json!({ "query": QUERY_TEXT }),
            RequestAuth::default(),
        ))
        .await
        .expect("semantic search request should complete");
    assert_json_error(
        no_auth_response,
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn semantic_search_rejects_empty_query() {
    let fixture = SearchFixture::enabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(search_request(&sync_token, &json!({ "query": "  " })))
        .await
        .expect("semantic search request should complete");

    assert_json_error(
        response,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "query must not be empty",
    )
    .await;
}

#[tokio::test]
async fn semantic_search_rejects_missing_json_body() {
    let fixture = SearchFixture::enabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(empty_search_request(&sync_token))
        .await
        .expect("semantic search request should complete");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn semantic_search_returns_ordered_hits_with_model_metadata() {
    let fixture = SearchFixture::enabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);
    seed_ordered_hits(&fixture.state, QUERY_TEXT).await;

    let response = search(
        &build_router(fixture.state.clone()),
        &sync_token,
        &json!({ "query": QUERY_TEXT, "top_k": 3 }),
    )
    .await;

    assert_eq!(response.model, "stub/sha256-2");
    assert_eq!(response.query_dim, 2);
    assert_eq!(response.hits.len(), 3);
    assert!(
        response
            .hits
            .windows(2)
            .all(|pair| pair[0].similarity >= pair[1].similarity)
    );
    assert_eq!(response.hits[0].note_id, NOTE_ID);
    assert_eq!(response.hits[0].chunk_id, FIRST_CHUNK_ID);
    assert_eq!(response.hits[0].snippet, "Exact semantic topic sentence.");
}

#[tokio::test]
async fn semantic_search_returns_422_for_malformed_embedding_blob() {
    let fixture = SearchFixture::enabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);
    seed_malformed_embedding_blob(&fixture.state, &[1, 2, 3, 4, 5]);

    let response = build_router(fixture.state.clone())
        .oneshot(search_request(
            &sync_token,
            &json!({ "query": QUERY_TEXT, "top_k": 1 }),
        ))
        .await
        .expect("semantic search request should complete");

    assert_json_error(
        response,
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_embedding_blob",
        "invalid embedding blob length 5; expected a non-empty multiple of 4 bytes",
    )
    .await;
}

#[tokio::test]
async fn semantic_search_caps_large_top_k_at_one_hundred() {
    let fixture = SearchFixture::enabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);
    seed_many_hits(&fixture.state, QUERY_TEXT, 101).await;

    let response = search(
        &build_router(fixture.state.clone()),
        &sync_token,
        &json!({ "query": QUERY_TEXT, "top_k": 500 }),
    )
    .await;

    assert_eq!(response.hits.len(), 100);
}

#[tokio::test]
async fn semantic_search_returns_404_when_ai_config_is_disabled() {
    let fixture = SearchFixture::disabled();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(search_request(&sync_token, &json!({ "query": QUERY_TEXT })))
        .await
        .expect("semantic search request should complete");

    assert_json_error(response, StatusCode::NOT_FOUND, "not_found", "not found").await;
}

#[tokio::test]
async fn semantic_search_returns_503_when_provider_is_unavailable() {
    let fixture = SearchFixture::offline_without_local();
    let sync_token = fixture.issue_token(vec![AuthScope::Sync]);

    let response = build_router(fixture.state.clone())
        .oneshot(search_request(&sync_token, &json!({ "query": QUERY_TEXT })))
        .await
        .expect("semantic search request should complete");

    assert_json_error(
        response,
        StatusCode::SERVICE_UNAVAILABLE,
        "service_unavailable",
        "ai service unavailable",
    )
    .await;
}

async fn search(app: &axum::Router, token: &str, body: &Value) -> SemanticSearchResponse {
    let response = app
        .clone()
        .oneshot(search_request(token, body))
        .await
        .expect("semantic search request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    response_body(response).await
}

fn search_request(token: &str, body: &Value) -> axum::http::Request<axum::body::Body> {
    json_request(
        Method::POST,
        SEARCH_PATH,
        body,
        RequestAuth {
            cookie_header: None,
            csrf_token: None,
            bearer_token: Some(token),
        },
    )
}

fn empty_search_request(token: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(SEARCH_PATH)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::empty())
        .expect("empty semantic search request should build")
}

async fn seed_ordered_hits(state: &ServerState, query: &str) {
    let provider = StubEmbeddingsProvider::new(2);
    let model = provider.capability().model.clone();
    let query_vector = embed_one(&provider, query.to_string(), model.clone())
        .await
        .expect("stub provider should embed query");
    let orthogonal = orthogonal_vector(&query_vector);
    let opposite = opposite_vector(&query_vector);
    let note_id = NoteId::new(NOTE_ID);
    let chunks = vec![
        chunk(
            &note_id,
            FIRST_CHUNK_ID,
            0,
            "Exact semantic topic sentence.",
        ),
        chunk(&note_id, SECOND_CHUNK_ID, 1, "Orthogonal topic sentence."),
        chunk(&note_id, THIRD_CHUNK_ID, 2, "Opposite topic sentence."),
    ];
    let records = vec![
        embedding(&chunks[0].chunk_id, &model, query_vector),
        embedding(&chunks[1].chunk_id, &model, orthogonal),
        embedding(&chunks[2].chunk_id, &model, opposite),
    ];

    seed_chunks_and_embeddings(state, &note_id, &chunks, &records);
}

async fn seed_many_hits(state: &ServerState, query: &str, count: u32) {
    let provider = StubEmbeddingsProvider::new(2);
    let model = provider.capability().model.clone();
    let query_vector = embed_one(&provider, query.to_string(), model.clone())
        .await
        .expect("stub provider should embed query");
    let note_id = NoteId::new(NOTE_ID);
    let chunks = (0..count)
        .map(|ordinal| {
            let chunk_id = ChunkId::generate();
            ChunkRecord {
                chunk_id,
                note_id: note_id.clone(),
                ordinal,
                content_hash: ContentHash::from_bytes(format!("chunk {ordinal}").as_bytes()),
                text: format!("Chunk {ordinal}"),
                token_count: Some(2),
                created_at: parse_timestamp("2026-05-05T10:10:00Z"),
            }
        })
        .collect::<Vec<_>>();
    let records = chunks
        .iter()
        .map(|chunk| embedding(&chunk.chunk_id, &model, query_vector.clone()))
        .collect::<Vec<_>>();

    seed_chunks_and_embeddings(state, &note_id, &chunks, &records);
}

fn seed_chunks_and_embeddings(
    state: &ServerState,
    note_id: &NoteId,
    chunks: &[ChunkRecord],
    records: &[ChunkEmbeddingRecord],
) {
    state
        .db()
        .with_conn(|connection| {
            NotesRepo::create(connection, &note_row(note_id))?;
            ChunksRepo::new(connection).replace_for_note(note_id, chunks)?;
            let repo = EmbeddingsRepo::new(connection);
            for record in records {
                repo.upsert(record)?;
            }

            Ok(())
        })
        .expect("semantic search fixtures should seed");
}

fn seed_malformed_embedding_blob(state: &ServerState, bytes: &[u8]) {
    let provider = StubEmbeddingsProvider::new(2);
    let model = provider.capability().model.clone();
    let note_id = NoteId::new(NOTE_ID);
    let chunk = chunk(
        &note_id,
        FIRST_CHUNK_ID,
        0,
        "Malformed semantic topic sentence.",
    );

    state
        .db()
        .with_conn(|connection| {
            NotesRepo::create(connection, &note_row(&note_id))?;
            ChunksRepo::new(connection).replace_for_note(&note_id, std::slice::from_ref(&chunk))?;
            connection
                .execute(
                    r"
                    INSERT INTO chunk_embeddings (
                        chunk_id,
                        embedding_model,
                        provider_id,
                        dim,
                        vector,
                        created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ",
                    rusqlite::params![
                        chunk.chunk_id.as_str(),
                        model.as_str(),
                        provider_id().as_str(),
                        2_i64,
                        bytes,
                        "2026-05-05T11:00:00Z",
                    ],
                )
                .expect("malformed embedding fixture should insert");

            Ok(())
        })
        .expect("malformed semantic search fixture should seed");
}

fn note_row(note_id: &NoteId) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}"),
        body_markdown: format!("# {note_id}\nbody"),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"Title {note_id}"}}"#),
        created_at: parse_timestamp("2026-05-05T10:00:00Z"),
        updated_at: parse_timestamp("2026-05-05T10:05:00Z"),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn chunk(note_id: &NoteId, chunk_id: &str, ordinal: u32, text: &str) -> ChunkRecord {
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

fn embedding(
    chunk_id: &ChunkId,
    model: &EmbeddingModelId,
    vector: EmbeddingVector,
) -> ChunkEmbeddingRecord {
    ChunkEmbeddingRecord {
        chunk_id: chunk_id.clone(),
        model: model.clone(),
        provider: provider_id(),
        vector,
        created_at: parse_timestamp("2026-05-05T11:00:00Z"),
    }
}

fn orthogonal_vector(vector: &EmbeddingVector) -> EmbeddingVector {
    let mut values = vec![vector.values[1], -vector.values[0]];
    let mut orthogonal = EmbeddingVector {
        values: std::mem::take(&mut values),
        model: vector.model.clone(),
    };
    orthogonal.normalize_in_place();
    orthogonal
}

fn opposite_vector(vector: &EmbeddingVector) -> EmbeddingVector {
    EmbeddingVector {
        values: vector.values.iter().map(|value| -*value).collect(),
        model: vector.model.clone(),
    }
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("stub provider id should parse")
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn ai_config(mode: ExecutionMode) -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: Some(StubEmbeddingsConfig { dim: 2 }),
        chat: None,
        default_mode: mode,
        recompute_concurrency: 1,
        max_attempts: 3,
    }
}

fn test_config(database_path: PathBuf, ai: Option<AiConfig>) -> ServerConfig {
    ServerConfig {
        vault_path: PathBuf::from("."),
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai,
    }
}
