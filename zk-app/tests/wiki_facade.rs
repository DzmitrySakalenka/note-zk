// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "ai")]

use std::{sync::Arc, time::Duration};

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};
use zk_app::{
    AiJobStatus, AppError, LocalWikiIndexHandle, WikiFacade, WikiFacadeDeps, WikiLintFindingKind,
    WikiLintScope, WikiLintSeverity,
};
use zk_domain::{
    JobId,
    id::{NoteId, RelativePath},
};
use zk_index::{SourcesRepo, open_memory};
use zk_sync::HttpTransport;

const SOURCE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6S";
const PAGE_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6P";
const JOB_ID: &str = "01JAR5A3PJEZ3SKG67PN8N6S6J";

#[tokio::test(flavor = "current_thread")]
async fn ingest_source_posts_admin_request_and_returns_job_id() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("POST"))
        .and(path("/admin/ai/wiki/ingest"))
        .and(header("authorization", "Bearer shared-token"))
        .and(|request: &wiremock::Request| {
            let body: serde_json::Value = request.body_json().expect("body should be JSON");

            body.get("source_note_id")
                .and_then(serde_json::Value::as_str)
                == Some(SOURCE_ID)
                && body
                    .get("request_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|request_id| request_id.len() == 26)
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "job_id": JOB_ID })))
        .expect(1)
        .mount(&server)
        .await;

    let job_id = facade
        .ingest_source(NoteId::new(SOURCE_ID))
        .await
        .expect("ingest source should enqueue");

    assert_eq!(job_id, JobId::new(JOB_ID).expect("job id should parse"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn ingest_retry_reuses_request_id() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);
    let retrying_mock = server
        .register_as_scoped(
            Mock::given(method("POST"))
                .and(path("/admin/ai/wiki/ingest"))
                .respond_with(ResponseTemplate::new(500).set_body_string("temporary failure"))
                .up_to_n_times(1),
        )
        .await;
    let success_mock = server
        .register_as_scoped(
            Mock::given(method("POST"))
                .and(path("/admin/ai/wiki/ingest"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "job_id": JOB_ID })))
                .expect(1),
        )
        .await;

    let job_id = facade
        .ingest_source(NoteId::new(SOURCE_ID))
        .await
        .expect("ingest source should retry once");

    assert_eq!(job_id, JobId::new(JOB_ID).expect("job id should parse"));
    let mut captured_request_ids = request_ids(retrying_mock.received_requests().await);
    captured_request_ids.extend(request_ids(success_mock.received_requests().await));
    assert_eq!(captured_request_ids.len(), 2);
    assert_eq!(captured_request_ids[0], captured_request_ids[1]);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn lint_wiki_posts_folder_scope_and_returns_job_id() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("POST"))
        .and(path("/admin/ai/wiki/lint"))
        .and(header("authorization", "Bearer shared-token"))
        .and(|request: &wiremock::Request| {
            let body: serde_json::Value = request.body_json().expect("body should be JSON");

            body.get("scope") == Some(&json!({ "folder": { "path": "wiki/entities" } }))
                && body
                    .get("request_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|request_id| request_id.len() == 26)
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "job_id": JOB_ID })))
        .expect(1)
        .mount(&server)
        .await;

    let job_id = facade
        .lint_wiki(WikiLintScope::Folder {
            path: RelativePath::new("wiki/entities"),
        })
        .await
        .expect("lint job should enqueue");

    assert_eq!(job_id, JobId::new(JOB_ID).expect("job id should parse"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn list_lint_findings_decodes_server_response() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("GET"))
        .and(path("/admin/ai/wiki/lint/findings"))
        .and(query_param("job_id", JOB_ID))
        .and(query_param("limit", "25"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "findings": [
                {
                    "finding_id": "01JAR5A3PJEZ3SKG67PN8N6F1",
                    "job_id": JOB_ID,
                    "kind": "stale_claim",
                    "page_id": PAGE_ID,
                    "source_id": SOURCE_ID,
                    "message": "Source changed after page update",
                    "severity": "warning",
                    "created_at": "2026-05-13T12:00:00Z"
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let findings = facade
        .list_lint_findings(JobId::new(JOB_ID).expect("job id should parse"), Some(25))
        .await
        .expect("findings should decode");

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, WikiLintFindingKind::StaleClaim);
    assert_eq!(findings[0].severity, WikiLintSeverity::Warning);
    assert_eq!(findings[0].page_id, Some(NoteId::new(PAGE_ID)));
    assert_eq!(findings[0].source_id, Some(NoteId::new(SOURCE_ID)));
    assert_eq!(findings[0].message, "Source changed after page update");
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn server_unauthorized_response_maps_to_auth_error() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("POST"))
        .and(path("/admin/ai/wiki/lint"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {
                "code": "unauthorized",
                "message": "admin token required"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let error = facade
        .lint_wiki(WikiLintScope::Vault)
        .await
        .expect_err("unauthorized lint should fail");

    assert!(matches!(error, AppError::Auth { message } if message == "admin token required"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn wait_for_job_polls_jobs_list_until_terminal_status() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    let _pending_mock = server
        .register_as_scoped(
            Mock::given(method("GET"))
                .and(path("/admin/ai/jobs"))
                .and(query_param("limit", "1000"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!([job_json("pending")])),
                )
                .up_to_n_times(1),
        )
        .await;
    let _done_mock = server
        .register_as_scoped(
            Mock::given(method("GET"))
                .and(path("/admin/ai/jobs"))
                .and(query_param("limit", "1000"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([job_json("done")])))
                .expect(1),
        )
        .await;

    let status = facade
        .wait_for_job(
            JobId::new(JOB_ID).expect("job id should parse"),
            Duration::from_secs(5),
        )
        .await
        .expect("wait should succeed");

    assert_eq!(status, AiJobStatus::Done);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn wait_for_job_missing_job_maps_to_not_found() {
    let server = MockServer::start().await;
    let facade = connected_facade(&server);

    Mock::given(method("GET"))
        .and(path("/admin/ai/jobs"))
        .and(query_param("limit", "1000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let error = facade
        .wait_for_job(
            JobId::new(JOB_ID).expect("job id should parse"),
            Duration::from_secs(5),
        )
        .await
        .expect_err("missing job should fail");

    assert!(matches!(error, AppError::NotFound { message } if message.contains(JOB_ID)));
    server.verify().await;
}

#[test]
fn provenance_methods_read_local_index() {
    let connection = open_memory().expect("open in-memory SQLite");
    zk_index::ensure_schema(&connection).expect("schema should be ready");
    SourcesRepo::replace_for_note(
        &connection,
        &NoteId::new(PAGE_ID),
        &[NoteId::new(SOURCE_ID)],
    )
    .expect("source row should insert");
    let facade = WikiFacade::new(WikiFacadeDeps {
        http: None,
        local_index: Some(LocalWikiIndexHandle::new(connection)),
    })
    .expect("wiki facade should construct");

    assert_eq!(
        facade
            .list_sources_for_note(NoteId::new(PAGE_ID))
            .expect("sources should query"),
        vec![NoteId::new(SOURCE_ID)]
    );
    assert_eq!(
        facade
            .list_pages_using_source(NoteId::new(SOURCE_ID))
            .expect("pages should query"),
        vec![NoteId::new(PAGE_ID)]
    );
}

fn connected_facade(server: &MockServer) -> WikiFacade {
    WikiFacade::new(WikiFacadeDeps {
        http: Some(Arc::new(http_transport(server, "shared-token"))),
        local_index: None,
    })
    .expect("wiki facade should construct")
}

fn http_transport(server: &MockServer, token: &str) -> HttpTransport {
    HttpTransport::new(
        server.uri().parse().expect("mock server URL should parse"),
        Some(token.to_string()),
    )
    .expect("HTTP transport should construct")
}

fn job_json(status: &str) -> serde_json::Value {
    json!({
        "job_id": JOB_ID,
        "kind": "lint_wiki",
        "status": status,
        "attempts": 0,
        "last_error": null,
        "enqueued_at": "2026-05-13T12:00:00Z",
        "started_at": null,
        "finished_at": null
    })
}

fn request_ids(requests: Vec<wiremock::Request>) -> Vec<String> {
    requests
        .into_iter()
        .map(|request| {
            let body: serde_json::Value = request.body_json().expect("body should be JSON");
            body.get("request_id")
                .and_then(serde_json::Value::as_str)
                .expect("request_id should be present")
                .to_string()
        })
        .collect()
}
