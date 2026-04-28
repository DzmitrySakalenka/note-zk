//! Transport abstraction for remote sync APIs.

use std::sync::atomic::{AtomicI64, Ordering};

use async_trait::async_trait;
use zk_domain::{
    PushItemResult, RegisterDeviceRequest, RegisterDeviceResponse, ResolveConflictRequest,
    ResolveConflictResponse, SyncPullRequest, SyncPullResponse, SyncPushRequest, SyncPushResponse,
    id::DeviceId,
};

use crate::{SyncError, SyncResult, bootstrap::RemoteBootstrapProbe};

const NOOP_SERVER_TIME: &str = "2026-04-22T18:07:33Z";
const NOOP_TRASH_RETENTION_DAYS: u32 = 14;
const NOOP_MAX_BATCH_SIZE: u32 = 100;

/// Runtime-agnostic sync transport used by the app-layer sync facade.
#[async_trait]
pub trait SyncTransport: Send + Sync {
    /// Registers or refreshes one device against the remote side.
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> SyncResult<RegisterDeviceResponse>;

    /// Sends one local push batch to the remote side.
    async fn push(&self, request: &SyncPushRequest) -> SyncResult<SyncPushResponse>;

    /// Pulls one ordered remote event batch from the current cursor.
    async fn pull(&self, request: &SyncPullRequest) -> SyncResult<SyncPullResponse>;

    /// Sends one explicit conflict-resolution request to the remote side.
    async fn resolve_conflict(
        &self,
        _request: &ResolveConflictRequest,
    ) -> SyncResult<ResolveConflictResponse> {
        Err(SyncError::Unsupported("resolve_conflict"))
    }

    /// Returns remote note and asset counts for initial-sync bootstrap detection.
    async fn probe_remote_counts(&self, device_id: &DeviceId) -> SyncResult<RemoteBootstrapProbe>;
}

/// Deterministic transport double for local sync composition tests.
#[derive(Debug, Default)]
pub struct NoopTransport {
    next_server_revision: AtomicI64,
}

impl NoopTransport {
    /// Creates a new no-op transport with deterministic responses.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn next_server_revision(&self) -> i64 {
        self.next_server_revision.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[async_trait]
impl SyncTransport for NoopTransport {
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> SyncResult<RegisterDeviceResponse> {
        Ok(RegisterDeviceResponse {
            device_id: request.device_id.clone(),
            server_time: NOOP_SERVER_TIME.to_string(),
            trash_retention_days: NOOP_TRASH_RETENTION_DAYS,
            max_batch_size: NOOP_MAX_BATCH_SIZE,
        })
    }

    async fn push(&self, request: &SyncPushRequest) -> SyncResult<SyncPushResponse> {
        let mut results =
            Vec::with_capacity(request.notes.len().saturating_add(request.assets.len()));

        for note in &request.notes {
            results.push(PushItemResult {
                item_id: note.note_id.clone(),
                accepted: true,
                conflict: false,
                new_server_revision: Some(self.next_server_revision()),
                canonical_path: None,
                message: None,
            });
        }

        for asset in &request.assets {
            results.push(PushItemResult {
                item_id: asset.asset_id.clone(),
                accepted: true,
                conflict: false,
                new_server_revision: Some(self.next_server_revision()),
                canonical_path: None,
                message: None,
            });
        }

        Ok(SyncPushResponse { results })
    }

    async fn pull(&self, request: &SyncPullRequest) -> SyncResult<SyncPullResponse> {
        Ok(SyncPullResponse {
            changes: Vec::new(),
            next_cursor: request.cursor,
        })
    }

    async fn probe_remote_counts(&self, _device_id: &DeviceId) -> SyncResult<RemoteBootstrapProbe> {
        Ok(RemoteBootstrapProbe {
            note_count: 0,
            asset_count: 0,
        })
    }
}
