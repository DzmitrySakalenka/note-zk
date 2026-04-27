// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fs;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use pollster::block_on;
use tempfile::TempDir;
use zk_app::{
    AppError, BootstrapOutcome, Clock, SyncContext, bootstrap, plan_and_push, pull_and_apply,
    register_device,
};
use zk_domain::{
    RegisterDeviceRequest, RegisterDeviceResponse,
    id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::NoteRecord,
    sync::SyncStatus,
    sync_state::DeviceRecord,
};
use zk_index::{DeviceRepo, NoteRepo, SyncStateRepo, ensure_schema, open_memory};
use zk_sync::{
    SyncError,
    bootstrap::RemoteBootstrapProbe,
    transport::{NoopTransport, SyncTransport},
};

fn device_id() -> DeviceId {
    DeviceId::new("device-alpha")
}

fn trash_dir() -> RelativePath {
    RelativePath::new("99_trash")
}

fn open_sync_connection() -> rusqlite::Connection {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    connection
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        now: Utc
            .with_ymd_and_hms(2026, 5, 1, 0, 0, 0)
            .single()
            .expect("fixed clock timestamp should be valid"),
    }
}

fn register_request() -> RegisterDeviceRequest {
    RegisterDeviceRequest {
        device_id: device_id().to_string(),
        device_name: "Laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["sync".to_string(), "pull".to_string()],
    }
}

fn device_record(cursor: Option<i64>) -> DeviceRecord {
    DeviceRecord {
        device_id: device_id(),
        device_name: "Laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["sync".to_string()],
        created_at: "2026-04-01T08:00:00Z".to_string(),
        last_seen_at: "2026-04-22T18:07:33Z".to_string(),
        last_sync_cursor: cursor,
    }
}

fn base_note(note_id: &NoteId, path: &str, body: &[u8]) -> NoteRecord {
    NoteRecord {
        note_id: note_id.clone(),
        current_path: RelativePath::new(path),
        content_hash: ContentHash::from_bytes(body),
        created_at: "2026-04-01T00:00:00Z".to_string(),
        updated_at: "2026-04-02T00:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: Some(device_id()),
        server_revision: ServerRevision::new(0),
    }
}

fn insert_note(conn: &rusqlite::Connection, record: &NoteRecord) {
    NoteRepo::upsert_note(
        conn,
        &record.note_id,
        &record.current_path,
        "TG11 note",
        &record.content_hash,
        r#"{"id":"01TG11NOTE","title":"TG11 note"}"#,
        &record.created_at,
        &record.updated_at,
        record.trashed_at.as_deref(),
        record.is_trashed,
    )
    .expect("note should be inserted");
}

fn write_vault_file(vault_root: &std::path::Path, relative_path: &str, bytes: &[u8]) {
    let absolute_path = vault_root.join(relative_path);
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent).expect("parent directory should create");
    }
    fs::write(absolute_path, bytes).expect("vault file should write");
}

#[test]
fn register_device_persists_local_device_record() {
    let connection = open_sync_connection();
    let transport = NoopTransport::new();

    let response = block_on(register_device(&transport, &connection, register_request()))
        .expect("register device should succeed");

    let device = DeviceRepo::get_device(&connection, &device_id())
        .expect("device query should succeed")
        .expect("device should exist");
    assert_eq!(response.device_id, device_id().to_string());
    assert_eq!(device.device_name, "Laptop");
    assert_eq!(device.platform.as_deref(), Some("macos"));
    assert_eq!(device.app_version.as_deref(), Some("0.1.0"));
    assert_eq!(
        device.capabilities,
        vec!["sync".to_string(), "pull".to_string()]
    );
    assert_eq!(device.created_at, response.server_time);
    assert_eq!(device.last_seen_at, response.server_time);
    assert_eq!(device.last_sync_cursor, None);
}

#[test]
fn plan_and_push_with_no_items_returns_empty_outcome_and_preserves_cursor() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    DeviceRepo::upsert_device(&connection, &device_record(Some(7))).expect("device should insert");

    let outcome = block_on(plan_and_push(
        &NoopTransport::new(),
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
        },
    ))
    .expect("plan and push should succeed");

    assert_eq!(outcome.accepted, 0);
    assert!(outcome.conflicts.is_empty());
    assert!(outcome.errors.is_empty());
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(7)
    );
}

#[test]
fn pull_and_apply_with_empty_transport_returns_zero_outcome_and_preserves_cursor() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    DeviceRepo::upsert_device(&connection, &device_record(Some(11))).expect("device should insert");

    let outcome = block_on(pull_and_apply(
        &NoopTransport::new(),
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
        },
    ))
    .expect("pull and apply should succeed");

    assert_eq!(outcome.applied, 0);
    assert!(outcome.conflicts.is_empty());
    assert_eq!(outcome.next_cursor, Some(11));
    assert_eq!(
        DeviceRepo::get_device(&connection, &device_id())
            .expect("device query should succeed")
            .expect("device should exist")
            .last_sync_cursor,
        Some(11)
    );
}

#[test]
fn bootstrap_mode_a_uses_pull_path() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    DeviceRepo::upsert_device(&connection, &device_record(None)).expect("device should insert");
    let transport = ProbeTransport::new(1, 0);

    let outcome = block_on(bootstrap(
        &transport,
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
        },
    ))
    .expect("bootstrap mode a should succeed");

    assert!(matches!(
        outcome,
        BootstrapOutcome::Pulled(zk_app::PullApplyOutcome {
            applied: 0,
            ref conflicts,
            next_cursor: None,
        }) if conflicts.is_empty()
    ));
}

#[test]
fn bootstrap_mode_b_uses_push_path_and_creates_sync_state() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let note_id = NoteId::new("01TG11011MODEB000000000001");
    insert_note(
        &connection,
        &base_note(&note_id, "notes/bootstrap.md", b"# Bootstrap\n"),
    );
    write_vault_file(vault.path(), "notes/bootstrap.md", b"# Bootstrap\n");

    let outcome = block_on(bootstrap(
        &NoopTransport::new(),
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
        },
    ))
    .expect("bootstrap mode b should succeed");

    assert!(matches!(
        outcome,
        BootstrapOutcome::Pushed(zk_app::PushApplyOutcome {
            accepted: 1,
            ref conflicts,
            ref errors,
        }) if conflicts.is_empty() && errors.is_empty()
    ));

    let state = SyncStateRepo::get_note_sync_state(&connection, &note_id)
        .expect("sync-state query should succeed")
        .expect("sync-state should exist");
    assert_eq!(state.device_id, device_id());
    assert_eq!(state.sync_status, SyncStatus::Synced);
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.last_known_server_revision,
        Some(ServerRevision::new(1))
    );
}

#[test]
fn bootstrap_mode_c_surfaces_ambiguous_bootstrap() {
    let mut connection = open_sync_connection();
    let vault = TempDir::new().expect("temp vault should create");
    let note_id = NoteId::new("01TG11011MODEC000000000001");
    insert_note(
        &connection,
        &base_note(&note_id, "notes/ambiguous.md", b"# Ambiguous\n"),
    );
    write_vault_file(vault.path(), "notes/ambiguous.md", b"# Ambiguous\n");
    let transport = ProbeTransport::new(1, 0);

    let error = block_on(bootstrap(
        &transport,
        SyncContext {
            conn: &mut connection,
            vault: vault.path(),
            device_id: &device_id(),
            clock: &fixed_clock(),
            trash_dir: &trash_dir(),
        },
    ))
    .expect_err("bootstrap mode c should fail");

    assert!(matches!(
        error,
        AppError::Sync(SyncError::AmbiguousBootstrap)
    ));
}

#[derive(Debug, Clone, Copy)]
struct FixedClock {
    now: DateTime<Utc>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

struct ProbeTransport {
    inner: NoopTransport,
    counts: RemoteBootstrapProbe,
}

impl ProbeTransport {
    fn new(note_count: u32, asset_count: u32) -> Self {
        Self {
            inner: NoopTransport::new(),
            counts: RemoteBootstrapProbe {
                note_count,
                asset_count,
            },
        }
    }
}

#[async_trait]
impl SyncTransport for ProbeTransport {
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> zk_sync::SyncResult<RegisterDeviceResponse> {
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
        self.inner.pull(request).await
    }

    async fn probe_remote_counts(
        &self,
        device_id: &DeviceId,
    ) -> zk_sync::SyncResult<RemoteBootstrapProbe> {
        let _ = device_id;
        Ok(self.counts)
    }
}
