// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};
use zk_ai::{
    provider::StubEmbeddingsProvider,
    selector::{ProviderRegistry, ProviderSelector},
    service::semantic_search::SearchHit,
};
use zk_app::{AiFacade, AiFacadeDeps, LocalEmbeddingsRepoHandle, SemanticSearchRequest};
use zk_domain::{ExecutionMode, ProviderId, ProviderSelectorPolicy};
use zk_sync::HttpTransport;

const EMBEDDING_DIM: usize = 128;
const SHARED_TOKEN: &str = "shared-token";
const QUERY_STRINGS: &[&str] = &[
    "ANSI terminal control",
    "ANSI terminal",
    "vector indexing",
    "approximate nearest neighbor metadata",
    "garlic pasta recipe",
    "rust async runtimes",
    "tokio async executor",
    "synthwave chord progressions",
    "neon arpeggiated pads",
    "markdown chunk embeddings",
];

#[tokio::test(flavor = "current_thread")]
async fn hybrid_connected_and_offline_top_one_match_for_fixture_queries() {
    let local_repo = seeded_fixture_repo().await;

    for query in QUERY_STRINGS {
        let offline_hits = search(&offline_facade(local_repo.clone()), query).await;
        let expected_top_hit = top_hit_identity(&offline_hits);

        let connected_server = MockServer::start().await;
        mount_remote_search(&connected_server, query, &offline_hits, 1).await;
        let connected_hits = search(&connected_facade(&connected_server), query).await;
        connected_server.verify().await;

        // Compare only top-1 identity. Full ordering and scores are more brittle
        // around equal f32 similarities, while this still checks path equivalence.
        assert_eq!(
            top_hit_identity(&connected_hits),
            expected_top_hit,
            "connected top-1 should match offline for query {query:?}"
        );

        let hybrid_success_server = MockServer::start().await;
        mount_health(&hybrid_success_server, 200, 1).await;
        mount_remote_search(&hybrid_success_server, query, &offline_hits, 1).await;
        let hybrid_success_hits = search(
            &hybrid_facade(&hybrid_success_server, local_repo.clone()),
            query,
        )
        .await;
        hybrid_success_server.verify().await;
        assert_eq!(
            top_hit_identity(&hybrid_success_hits),
            top_hit_identity(&connected_hits),
            "hybrid probe success should match connected for query {query:?}"
        );

        let hybrid_failure_server = MockServer::start().await;
        mount_health(&hybrid_failure_server, 503, 1).await;
        mount_unexpected_remote_search(&hybrid_failure_server).await;
        let hybrid_failure_hits = search(
            &hybrid_facade(&hybrid_failure_server, local_repo.clone()),
            query,
        )
        .await;
        hybrid_failure_server.verify().await;
        assert_eq!(
            top_hit_identity(&hybrid_failure_hits),
            expected_top_hit,
            "hybrid probe failure should match offline for query {query:?}"
        );
    }
}

async fn seeded_fixture_repo() -> LocalEmbeddingsRepoHandle {
    let repo = LocalEmbeddingsRepoHandle::open_memory().expect("local repo should open");
    let facade = AiFacade::new(AiFacadeDeps {
        http: None,
        local_selector: Some(stub_selector()),
        local_repo: Some(repo.clone()),
        mode: ExecutionMode::Offline,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct");

    let summary = facade
        .recompute_local(&fixture_root())
        .await
        .expect("fixture recompute should succeed");
    assert_eq!(summary.notes, 5);
    assert!(summary.chunks > 0);
    assert!(summary.embeddings > 0);
    assert!(summary.errors.is_empty());

    repo
}

async fn search(facade: &AiFacade, query: &str) -> Vec<SearchHit> {
    let hits = facade
        .semantic_search(SemanticSearchRequest {
            query: query.to_string(),
            top_k: 1,
            mode_override: None,
        })
        .await
        .expect("semantic search should succeed");
    assert_eq!(hits.len(), 1, "query {query:?} should return one hit");

    hits
}

async fn mount_health(server: &MockServer, status: u16, expected_calls: u64) {
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(status))
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_remote_search(
    server: &MockServer,
    query: &str,
    hits: &[SearchHit],
    expected_calls: u64,
) {
    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .and(header("authorization", format!("Bearer {SHARED_TOKEN}")))
        .and(body_json(json!({ "query": query, "top_k": 1 })))
        .respond_with(remote_search_response(hits))
        .expect(expected_calls)
        .mount(server)
        .await;
}

async fn mount_unexpected_remote_search(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/search/semantic"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(server)
        .await;
}

fn remote_search_response(hits: &[SearchHit]) -> ResponseTemplate {
    let hits = hits
        .iter()
        .map(|hit| {
            json!({
                "note_id": hit.note_id.to_string(),
                "chunk_id": hit.chunk_id.to_string(),
                "similarity": hit.similarity,
                "snippet": hit.snippet,
            })
        })
        .collect::<Vec<_>>();

    ResponseTemplate::new(200).set_body_json(json!({
        "hits": hits,
        "query_dim": EMBEDDING_DIM,
        "model": format!("stub/sha256-{EMBEDDING_DIM}"),
    }))
}

fn top_hit_identity(hits: &[SearchHit]) -> (String, String) {
    let hit = hits.first().expect("top hit should exist");

    (hit.note_id.to_string(), hit.chunk_id.to_string())
}

fn connected_facade(server: &MockServer) -> AiFacade {
    AiFacade::new(AiFacadeDeps {
        http: Some(Arc::new(http_transport(server))),
        local_selector: None,
        local_repo: None,
        mode: ExecutionMode::Connected,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct")
}

fn offline_facade(local_repo: LocalEmbeddingsRepoHandle) -> AiFacade {
    AiFacade::new(AiFacadeDeps {
        http: None,
        local_selector: Some(stub_selector()),
        local_repo: Some(local_repo),
        mode: ExecutionMode::Offline,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct")
}

fn hybrid_facade(server: &MockServer, local_repo: LocalEmbeddingsRepoHandle) -> AiFacade {
    AiFacade::new(AiFacadeDeps {
        http: Some(Arc::new(http_transport(server))),
        local_selector: Some(stub_selector()),
        local_repo: Some(local_repo),
        mode: ExecutionMode::Hybrid,
        ..AiFacadeDeps::default()
    })
    .expect("AI facade should construct")
}

fn stub_selector() -> ProviderSelector {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(EMBEDDING_DIM)));

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

fn http_transport(server: &MockServer) -> HttpTransport {
    HttpTransport::new(
        server.uri().parse().expect("mock server URL should parse"),
        Some(SHARED_TOKEN.to_string()),
    )
    .expect("HTTP transport should construct")
}

fn provider_id(value: &str) -> ProviderId {
    value.parse().expect("provider id should parse")
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("zk-app should have a workspace parent")
        .join("tests")
        .join("fixtures")
        .join("ai-vault")
}
