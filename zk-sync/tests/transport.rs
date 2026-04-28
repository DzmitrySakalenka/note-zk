// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::{ResolveConflictRequest, WireConflictResolution, WireItemType};
use zk_sync::{NoopTransport, SyncError, transport::SyncTransport};

#[tokio::test(flavor = "current_thread")]
async fn default_resolve_conflict_reports_unsupported_for_noop_transport() {
    let transport = NoopTransport::new();
    let transport: &dyn SyncTransport = &transport;
    let request = ResolveConflictRequest {
        device_id: "device-a".to_string(),
        item_id: "note-a".to_string(),
        item_type: WireItemType::Note,
        resolution: WireConflictResolution::KeepRemote,
    };

    let result = transport.resolve_conflict(&request).await;

    assert!(matches!(
        result,
        Err(SyncError::Unsupported("resolve_conflict"))
    ));
}
