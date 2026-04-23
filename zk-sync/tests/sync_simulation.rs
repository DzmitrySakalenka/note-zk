use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use zk_app::AppError;
use zk_domain::{
    id::{RelativePath, ServerRevision},
    sync::SyncStatus,
};
use zk_sync::{SyncError, transport::SyncTransport};

mod support;

use support::{ClientFixture, FakeClock, FakeServer};

fn fixed_clock() -> FakeClock {
    FakeClock::new(
        Utc.with_ymd_and_hms(2026, 4, 23, 12, 0, 0)
            .single()
            .expect("fixed clock timestamp should be valid"),
    )
}

fn sync_origin_to_both(
    origin: &mut ClientFixture,
    peer: &mut ClientFixture,
    transport: &dyn SyncTransport,
) {
    let push_outcome = origin.push(transport).expect("push should succeed");
    assert!(push_outcome.errors.is_empty());
    assert!(push_outcome.conflicts.is_empty());
    origin.pull(transport).expect("origin pull should succeed");
    peer.pull(transport).expect("peer pull should succeed");
}

#[test]
fn bootstrap_mode_b_then_mode_a_converges_note_vaults_and_cursors() {
    let clock = fixed_clock();
    let server = FakeServer::new(clock.clone());
    let mut client_a = ClientFixture::new("client-a", clock.clone());
    let mut client_b = ClientFixture::new("client-b", clock);

    client_a
        .register(&server)
        .expect("client A should register");
    client_b
        .register(&server)
        .expect("client B should register");

    let note_a = client_a.make_note("notes/alpha.md", "# Alpha\n");
    let note_b = client_a.make_note("notes/beta.md", "# Beta\n");
    let note_c = client_a.make_note("notes/nested/gamma.md", "# Gamma\n");

    let bootstrap_a = client_a
        .bootstrap(&server)
        .expect("client A bootstrap should succeed");
    assert!(matches!(bootstrap_a, zk_app::BootstrapOutcome::Pushed(_)));

    client_a
        .pull(&server)
        .expect("client A follow-up pull should succeed");

    let bootstrap_b = client_b
        .bootstrap(&server)
        .expect("client B bootstrap should succeed");
    assert!(matches!(bootstrap_b, zk_app::BootstrapOutcome::Pulled(_)));

    assert_eq!(client_a.vault_snapshot(), client_b.vault_snapshot());
    assert_eq!(client_a.device_cursor(), Some(server.last_event_seq()));
    assert_eq!(client_b.device_cursor(), Some(server.last_event_seq()));

    for note_id in [&note_a, &note_b, &note_c] {
        let left = client_a.note_row(note_id);
        let right = client_b.note_row(note_id);
        assert_eq!(left.current_path, right.current_path);
        assert_eq!(left.content_hash, right.content_hash);
        assert_eq!(left.is_trashed, right.is_trashed);
        assert_eq!(left.trashed_at, right.trashed_at);
        assert_eq!(left.server_revision, right.server_revision);
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn note_lifecycle_round_trip_and_asset_pull_smoke() {
    let clock = fixed_clock();
    let server = FakeServer::new(clock.clone());
    let mut client_a = ClientFixture::new("client-a", clock.clone());
    let mut client_b = ClientFixture::new("client-b", clock.clone());

    client_a
        .register(&server)
        .expect("client A should register");
    client_b
        .register(&server)
        .expect("client B should register");

    let note_id = client_a.make_note("notes/lifecycle.md", "# Lifecycle v1\n");
    sync_origin_to_both(&mut client_a, &mut client_b, &server);
    assert_eq!(
        client_b.note_bytes("notes/lifecycle.md"),
        client_a.note_bytes("notes/lifecycle.md")
    );
    assert_eq!(
        client_b.note_row(&note_id).server_revision,
        ServerRevision::new(1)
    );
    assert_eq!(
        client_b.note_state(&note_id).sync_status,
        SyncStatus::Synced
    );

    client_a.edit_note(&note_id, "# Lifecycle v2\n");
    sync_origin_to_both(&mut client_a, &mut client_b, &server);
    assert_eq!(
        client_b.note_bytes("notes/lifecycle.md"),
        client_a.note_bytes("notes/lifecycle.md")
    );
    assert_eq!(
        client_b.note_row(&note_id).server_revision,
        ServerRevision::new(2)
    );

    client_a.move_note(&note_id, "notes/archive/lifecycle.md");
    sync_origin_to_both(&mut client_a, &mut client_b, &server);
    assert!(!client_b.file_exists("notes/lifecycle.md"));
    assert!(client_b.file_exists("notes/archive/lifecycle.md"));
    assert_eq!(
        client_b.note_row(&note_id).current_path.as_str(),
        "notes/archive/lifecycle.md"
    );
    assert_eq!(
        client_b
            .note_state(&note_id)
            .last_synced_path
            .as_ref()
            .map(RelativePath::as_str),
        Some("notes/archive/lifecycle.md")
    );

    client_a.trash_note(&note_id);
    sync_origin_to_both(&mut client_a, &mut client_b, &server);
    let trashed_note = client_b.note_row(&note_id);
    assert!(trashed_note.is_trashed);
    assert!(
        trashed_note
            .current_path
            .as_str()
            .starts_with("99_trash/2026-04-23/")
    );
    let note_delete_record = client_b
        .note_delete_record(&note_id)
        .expect("note delete record should exist");
    assert_eq!(
        note_delete_record.original_path.as_str(),
        "notes/archive/lifecycle.md"
    );

    client_a.restore_note(&note_id);
    sync_origin_to_both(&mut client_a, &mut client_b, &server);
    let restored_note = client_b.note_row(&note_id);
    assert!(!restored_note.is_trashed);
    assert_eq!(
        restored_note.current_path.as_str(),
        "notes/archive/lifecycle.md"
    );
    assert!(client_b.note_delete_record(&note_id).is_none());

    client_a.trash_note(&note_id);
    sync_origin_to_both(&mut client_a, &mut client_b, &server);
    clock.advance_days(20);
    client_a.purge_expired();
    let purge_push = client_a.push(&server).expect("purge push should succeed");
    assert_eq!(purge_push.accepted, 1);
    assert!(
        client_a
            .note_delete_record(&note_id)
            .expect("origin delete record should remain")
            .delete_revision
            .is_some()
    );
    client_a
        .pull(&server)
        .expect("origin purge pull should succeed");
    client_b
        .pull(&server)
        .expect("peer purge pull should succeed");
    assert!(!client_b.file_exists(trashed_note.current_path.as_str()));
    let purged_note = client_b.note_row(&note_id);
    assert!(purged_note.is_deleted);
    let peer_note_delete_record = client_b
        .note_delete_record(&note_id)
        .expect("peer delete record should remain after purge");
    assert!(peer_note_delete_record.deleted_at.is_some());
    assert!(peer_note_delete_record.delete_revision.is_some());

    let asset_id = server.seed_asset_create("assets/diagram.bin", b"asset-v1");
    client_b
        .pull(&server)
        .expect("asset create pull should succeed");
    let asset_v1 = client_b.asset_row(&asset_id);
    assert_eq!(asset_v1.current_path.as_str(), "assets/diagram.bin");
    assert_eq!(asset_v1.server_revision, ServerRevision::new(1));
    assert_eq!(client_b.asset_bytes("assets/diagram.bin"), b"asset-v1");

    server.seed_asset_update(&asset_id, b"asset-v2");
    client_b
        .pull(&server)
        .expect("asset update pull should succeed");
    let asset_v2 = client_b.asset_row(&asset_id);
    assert_eq!(asset_v2.server_revision, ServerRevision::new(2));
    assert_eq!(client_b.asset_bytes("assets/diagram.bin"), b"asset-v2");

    server.seed_asset_move(&asset_id, "assets/archive/diagram.bin");
    client_b
        .pull(&server)
        .expect("asset move pull should succeed");
    let moved_asset = client_b.asset_row(&asset_id);
    assert_eq!(
        moved_asset.current_path.as_str(),
        "assets/archive/diagram.bin"
    );
    assert!(client_b.file_exists("assets/archive/diagram.bin"));
    assert!(!client_b.file_exists("assets/diagram.bin"));

    server.seed_asset_trash(&asset_id);
    client_b
        .pull(&server)
        .expect("asset trash pull should succeed");
    let trashed_asset = client_b.asset_row(&asset_id);
    assert!(trashed_asset.is_trashed);
    assert!(trashed_asset.current_path.as_str().starts_with("99_trash/"));
    let asset_delete_record = client_b
        .asset_delete_record(&asset_id)
        .expect("asset delete record should exist");
    assert_eq!(
        asset_delete_record.original_path.as_str(),
        "assets/archive/diagram.bin"
    );

    server.seed_asset_restore(&asset_id);
    client_b
        .pull(&server)
        .expect("asset restore pull should succeed");
    let restored_asset = client_b.asset_row(&asset_id);
    assert!(!restored_asset.is_trashed);
    assert_eq!(
        restored_asset.current_path.as_str(),
        "assets/archive/diagram.bin"
    );
    assert!(client_b.asset_delete_record(&asset_id).is_none());

    server.seed_asset_trash(&asset_id);
    client_b
        .pull(&server)
        .expect("asset retrash pull should succeed");
    server.seed_asset_purge(&asset_id);
    client_b
        .pull(&server)
        .expect("asset purge pull should succeed");
    let purged_asset = client_b.asset_row(&asset_id);
    assert!(purged_asset.is_deleted);
    let peer_asset_delete_record = client_b
        .asset_delete_record(&asset_id)
        .expect("asset delete record should remain after purge");
    assert!(peer_asset_delete_record.deleted_at.is_some());
    assert!(peer_asset_delete_record.delete_revision.is_some());
}

#[test]
fn concurrent_edit_push_conflict_then_pull_overwrites_local_copy() {
    let clock = fixed_clock();
    let server = FakeServer::new(clock.clone());
    let mut client_a = ClientFixture::new("client-a", clock.clone());
    let mut client_b = ClientFixture::new("client-b", clock.clone());

    client_a
        .register(&server)
        .expect("client A should register");
    client_b
        .register(&server)
        .expect("client B should register");

    let note_id = client_a.make_note("notes/shared.md", "# Shared v1\n");
    let bootstrap_a = client_a
        .bootstrap(&server)
        .expect("client A bootstrap should succeed");
    assert!(matches!(bootstrap_a, zk_app::BootstrapOutcome::Pushed(_)));
    client_a
        .pull(&server)
        .expect("client A pull should succeed");
    let bootstrap_b = client_b
        .bootstrap(&server)
        .expect("client B bootstrap should succeed");
    assert!(matches!(bootstrap_b, zk_app::BootstrapOutcome::Pulled(_)));

    client_a.edit_note(&note_id, "# Shared from A\n");
    client_b.edit_note(&note_id, "# Shared from B\n");

    let push_a = client_a
        .push(&server)
        .expect("client A push should succeed");
    assert_eq!(push_a.accepted, 1);
    client_a
        .pull(&server)
        .expect("client A pull should succeed");

    let push_b = client_b
        .push(&server)
        .expect("client B push should succeed");
    assert_eq!(push_b.accepted, 0);
    assert_eq!(push_b.conflicts.len(), 1);
    assert!(push_b.errors.is_empty());
    assert_eq!(
        client_b.note_state(&note_id).sync_status,
        SyncStatus::Conflict
    );

    let pull_b = client_b
        .pull(&server)
        .expect("client B pull should succeed");
    assert_eq!(pull_b.applied, 1);
    assert!(pull_b.conflicts.is_empty());

    assert_eq!(
        client_b.note_bytes("notes/shared.md"),
        client_a.note_bytes("notes/shared.md")
    );

    let expected_conflict_copy = "notes/shared.conflict-remote-20260423T120000Z.md".to_string();
    assert!(!client_b.file_exists(&expected_conflict_copy));
    assert_eq!(
        client_b.note_state(&note_id).sync_status,
        SyncStatus::Synced
    );
    assert_eq!(client_b.device_cursor(), Some(server.last_event_seq()));
}

#[test]
fn failed_third_pull_event_rolls_back_cursor_and_replays_idempotently() {
    let clock = fixed_clock();
    let server = FakeServer::new(clock.clone());
    let corrupting_transport = CorruptingTransport::new(server.clone());
    let mut client_a = ClientFixture::new("client-a", clock.clone());
    let mut client_b = ClientFixture::new("client-b", clock);

    client_a
        .register(&server)
        .expect("client A should register");
    client_b
        .register(&server)
        .expect("client B should register");

    client_a.make_note("notes/one.md", "# One\n");
    let push_one = client_a.push(&server).expect("first push should succeed");
    assert_eq!(push_one.accepted, 1);

    client_a.make_note("notes/two.md", "# Two\n");
    let push_two = client_a.push(&server).expect("second push should succeed");
    assert_eq!(push_two.accepted, 1);

    client_a.make_note("notes/three.md", "# Three\n");
    let push_three = client_a.push(&server).expect("third push should succeed");
    assert_eq!(push_three.accepted, 1);

    let pull_error = client_b
        .pull(&corrupting_transport)
        .expect_err("first pull should fail on the third event");
    assert!(matches!(
        pull_error,
        AppError::Sync(SyncError::Transport(_))
    ));

    assert_eq!(client_b.device_cursor(), None);
    assert_eq!(client_b.note_count(), 0);
    assert!(client_b.file_exists("notes/one.md"));
    assert!(client_b.file_exists("notes/two.md"));
    assert!(!client_b.file_exists("notes/three.md"));

    let one_modified_before = client_b.file_modified("notes/one.md");
    let two_modified_before = client_b.file_modified("notes/two.md");

    let replay = client_b
        .pull(&server)
        .expect("replayed pull should succeed");
    assert_eq!(replay.applied, 3);
    assert_eq!(client_b.device_cursor(), Some(server.last_event_seq()));
    assert_eq!(client_b.note_count(), 3);
    assert_eq!(client_b.file_modified("notes/one.md"), one_modified_before);
    assert_eq!(client_b.file_modified("notes/two.md"), two_modified_before);
    assert!(client_b.file_exists("notes/three.md"));
}

#[test]
fn purge_propagation_keeps_delete_record_and_delete_revision() {
    let clock = fixed_clock();
    let server = FakeServer::new(clock.clone());
    let mut client_a = ClientFixture::new("client-a", clock.clone());
    let mut client_b = ClientFixture::new("client-b", clock.clone());

    client_a
        .register(&server)
        .expect("client A should register");
    client_b
        .register(&server)
        .expect("client B should register");

    let note_id = client_a.make_note("notes/purge.md", "# Purge\n");
    sync_origin_to_both(&mut client_a, &mut client_b, &server);

    client_a.trash_note(&note_id);
    sync_origin_to_both(&mut client_a, &mut client_b, &server);
    let trashed_path = client_b.note_row(&note_id).current_path;

    clock.advance_days(20);
    client_a.purge_expired();
    let purge_push = client_a.push(&server).expect("purge push should succeed");
    assert_eq!(purge_push.accepted, 1);

    let origin_delete_record = client_a
        .note_delete_record(&note_id)
        .expect("origin delete record should remain");
    assert!(origin_delete_record.delete_revision.is_some());

    client_a
        .pull(&server)
        .expect("origin purge pull should succeed");
    client_b
        .pull(&server)
        .expect("peer purge pull should succeed");

    assert!(!client_b.file_exists(trashed_path.as_str()));
    let peer_row = client_b.note_row(&note_id);
    assert!(peer_row.is_deleted);
    let peer_delete_record = client_b
        .note_delete_record(&note_id)
        .expect("peer delete record should remain");
    assert!(peer_delete_record.deleted_at.is_some());
    assert!(peer_delete_record.delete_revision.is_some());
    assert_eq!(client_b.device_cursor(), Some(server.last_event_seq()));
}

#[derive(Clone)]
struct CorruptingTransport {
    inner: FakeServer,
    corrupted_once: Arc<Mutex<bool>>,
}

impl CorruptingTransport {
    fn new(inner: FakeServer) -> Self {
        Self {
            inner,
            corrupted_once: Arc::new(Mutex::new(false)),
        }
    }
}

#[async_trait]
impl SyncTransport for CorruptingTransport {
    async fn register_device(
        &self,
        request: &zk_domain::RegisterDeviceRequest,
    ) -> zk_sync::SyncResult<zk_domain::RegisterDeviceResponse> {
        self.inner.register_device(request).await
    }

    async fn push(
        &self,
        request: &zk_domain::SyncPushRequest,
    ) -> zk_sync::SyncResult<zk_domain::SyncPushResponse> {
        self.inner.push(request).await
    }

    async fn pull(
        &self,
        request: &zk_domain::SyncPullRequest,
    ) -> zk_sync::SyncResult<zk_domain::SyncPullResponse> {
        let mut response = self.inner.pull(request).await?;
        let mut corrupted_once = self
            .corrupted_once
            .lock()
            .expect("corrupting transport mutex should not be poisoned");

        if !*corrupted_once && response.changes.len() >= 3 {
            response.changes[2].content = None;
            *corrupted_once = true;
        }

        Ok(response)
    }

    async fn probe_remote_counts(
        &self,
        device_id: &zk_domain::id::DeviceId,
    ) -> zk_sync::SyncResult<zk_sync::bootstrap::RemoteBootstrapProbe> {
        self.inner.probe_remote_counts(device_id).await
    }
}
