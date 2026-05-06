// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::net::TcpListener;

use serde_json::json;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};
use zk_domain::{
    PushItemResult, PushNoteItem, RegisterDeviceRequest, ResolveConflictRequest,
    ResolveConflictResponse, SyncChange, SyncPullRequest, SyncPullResponse, SyncPushRequest,
    SyncPushResponse, WireConflictResolution, WireEventType, WireItemType, WireOperation,
    id::DeviceId,
};
use zk_sync::{
    HttpTransport, SyncError, bootstrap::RemoteBootstrapProbe, transport::SyncTransport,
};

fn transport(server: &MockServer, bearer_token: Option<&str>) -> HttpTransport {
    HttpTransport::new(
        Url::parse(&server.uri()).expect("mock server URL should parse"),
        bearer_token.map(str::to_string),
    )
    .expect("transport should be constructed")
}

fn register_request() -> RegisterDeviceRequest {
    RegisterDeviceRequest {
        device_id: "device-a".to_string(),
        device_name: "Laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("1.0.0".to_string()),
        capabilities: vec!["pull".to_string(), "push".to_string()],
    }
}

fn push_request() -> SyncPushRequest {
    SyncPushRequest {
        device_id: "device-a".to_string(),
        notes: vec![PushNoteItem {
            note_id: "note-a".to_string(),
            path: "notes/a.md".to_string(),
            content: Some("# A\n".to_string()),
            content_hash: "hash-a".to_string(),
            base_server_revision: None,
            operation: WireOperation::Create,
        }],
        assets: Vec::new(),
    }
}

fn pull_request() -> SyncPullRequest {
    SyncPullRequest {
        device_id: "device-a".to_string(),
        cursor: Some(41),
        batch_size: Some(100),
    }
}

fn pull_change() -> SyncChange {
    SyncChange {
        event_seq: 42,
        item_id: "note-a".to_string(),
        item_type: WireItemType::Note,
        event_type: WireEventType::Create,
        path: Some("notes/a.md".to_string()),
        content: Some("# A\n".to_string()),
        asset_bytes_base64: None,
        content_hash: Some("hash-a".to_string()),
        server_revision: Some(7),
        trashed_at: None,
        retention_until: None,
        deleted_at: None,
    }
}

fn conflict_request() -> ResolveConflictRequest {
    ResolveConflictRequest {
        device_id: "device-a".to_string(),
        item_id: "note-a".to_string(),
        item_type: WireItemType::Note,
        resolution: WireConflictResolution::KeepRemote,
    }
}

fn assert_transport_error(error: SyncError) -> String {
    match error {
        SyncError::Transport(source) => source.to_string(),
        other => panic!("expected transport error, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn get_requests_health_path_with_bearer_token() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/health"))
        .and(header("authorization", "Bearer shared-secret"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let status = transport(&server, Some("shared-secret"))
        .get("/health")
        .await
        .expect("health probe should succeed");

    assert!(status.is_success());
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn register_device_posts_to_register_path_with_bearer_token() {
    let server = MockServer::start().await;
    let request = register_request();

    Mock::given(method("POST"))
        .and(path("/devices/register"))
        .and(header("authorization", "Bearer shared-secret"))
        .and(body_json(&request))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_id": "device-a",
            "server_time": "2026-04-22T18:07:33Z",
            "trash_retention_days": 14,
            "max_batch_size": 100,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let response = transport(&server, Some("shared-secret"))
        .register_device(&request)
        .await
        .expect("register request should succeed");

    assert_eq!(response.device_id, "device-a");
    assert_eq!(response.trash_retention_days, 14);
    assert_eq!(response.max_batch_size, 100);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn push_posts_to_sync_push_path() {
    let server = MockServer::start().await;
    let request = push_request();
    let response = SyncPushResponse {
        results: vec![PushItemResult {
            item_id: "note-a".to_string(),
            accepted: true,
            conflict: false,
            new_server_revision: Some(7),
            canonical_path: None,
            message: None,
        }],
    };

    Mock::given(method("POST"))
        .and(path("/sync/push"))
        .and(body_json(&request))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let result = transport(&server, None)
        .push(&request)
        .await
        .expect("push request should succeed");

    assert_eq!(result, response);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn pull_posts_to_sync_pull_path() {
    let server = MockServer::start().await;
    let request = pull_request();
    let response = SyncPullResponse {
        changes: vec![pull_change()],
        next_cursor: Some(42),
    };

    Mock::given(method("POST"))
        .and(path("/sync/pull"))
        .and(body_json(&request))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let result = transport(&server, None)
        .pull(&request)
        .await
        .expect("pull request should succeed");

    assert_eq!(result, response);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn resolve_conflict_posts_to_resolve_conflict_path() {
    let server = MockServer::start().await;
    let request = conflict_request();
    let response = ResolveConflictResponse {
        accepted: true,
        new_server_revision: Some(7),
    };

    Mock::given(method("POST"))
        .and(path("/sync/resolve-conflict"))
        .and(body_json(&request))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let transport = transport(&server, None);
    let transport: &dyn SyncTransport = &transport;

    let result = transport
        .resolve_conflict(&request)
        .await
        .expect("resolve request should succeed");

    assert_eq!(result, response);
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn probe_remote_counts_posts_pull_sentinel_request_for_empty_remote() {
    let server = MockServer::start().await;
    let request = SyncPullRequest {
        device_id: "device-a".to_string(),
        cursor: None,
        batch_size: Some(1),
    };
    let response = SyncPullResponse {
        changes: Vec::new(),
        next_cursor: None,
    };

    Mock::given(method("POST"))
        .and(path("/sync/pull"))
        .and(body_json(&request))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let result = transport(&server, None)
        .probe_remote_counts(&DeviceId::new("device-a"))
        .await
        .expect("probe request should succeed");

    assert_eq!(
        result,
        RemoteBootstrapProbe {
            note_count: 0,
            asset_count: 0
        }
    );
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn probe_remote_counts_returns_non_empty_sentinel_for_remote_content() {
    let server = MockServer::start().await;
    let response = SyncPullResponse {
        changes: vec![pull_change()],
        next_cursor: Some(42),
    };

    Mock::given(method("POST"))
        .and(path("/sync/pull"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let result = transport(&server, None)
        .probe_remote_counts(&DeviceId::new("device-a"))
        .await
        .expect("probe request should succeed");

    assert_eq!(
        result,
        RemoteBootstrapProbe {
            note_count: 1,
            asset_count: 0
        }
    );
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn http_error_surfaces_status_path_and_response_body() {
    let server = MockServer::start().await;
    let request = push_request();

    Mock::given(method("POST"))
        .and(path("/sync/push"))
        .respond_with(ResponseTemplate::new(409).set_body_string("duplicate create"))
        .expect(1)
        .mount(&server)
        .await;

    let error = transport(&server, None)
        .push(&request)
        .await
        .expect_err("push request should fail");
    let message = assert_transport_error(error);

    assert!(message.contains("POST /sync/push -> 409 Conflict: duplicate create"));
    server.verify().await;
}

#[tokio::test(flavor = "current_thread")]
async fn connection_error_surfaces_as_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral TCP listener should bind");
    let url = Url::parse(&format!(
        "http://{}/",
        listener
            .local_addr()
            .expect("listener local address should be available")
    ))
    .expect("connection-error URL should parse");
    drop(listener);
    let request = register_request();

    let error = HttpTransport::new(url, None)
        .expect("transport should be constructed")
        .register_device(&request)
        .await
        .expect_err("register request should fail");

    assert_transport_error(error);
}
