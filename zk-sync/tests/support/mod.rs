use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{DateTime, Days, SecondsFormat, Utc};
use pollster::block_on;
use tempfile::TempDir;
use zk_app::{
    AppError, BootstrapOutcome, Clock, PullApplyOutcome, PushApplyOutcome, RebuildOptions,
    SyncContext, WriteContext, bootstrap, create_note, plan_and_push, pull_and_apply,
    purge_expired, rebuild_index, register_device, restore_note, trash_note, update_note,
};
use zk_domain::{
    ItemKind, PurgeRequest, PurgeScope, RegisterDeviceRequest, RegisterDeviceResponse,
    RestoreRequest, SyncChange, SyncPullRequest, SyncPullResponse, SyncPushRequest,
    SyncPushResponse, TargetSelector, TrashRequest, WireEventType, WireItemType, WireOperation,
    asset::AssetRecord,
    config::EffectiveConfig,
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath},
    note::{NoteFrontmatter, NoteRecord},
    sync_state::{DeleteRecord, DeleteRecordTarget, NoteSyncState},
    write::{CreateNoteRequest, UpdateNoteRequest},
};
use zk_index::{
    AssetRepo, DeleteRecordRepo, DeviceRepo, NoteRepo, SyncStateRepo, ensure_schema, open_memory,
};
use zk_sync::{SyncResult, bootstrap::RemoteBootstrapProbe, transport::SyncTransport};

const SERVER_TRASH_RETENTION_DAYS: u32 = 14;
const SERVER_MAX_BATCH_SIZE: u32 = 100;

#[derive(Debug, Clone)]
pub struct FakeClock {
    current: Arc<Mutex<DateTime<Utc>>>,
}

impl FakeClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            current: Arc::new(Mutex::new(start)),
        }
    }

    pub fn advance_days(&self, days: u64) {
        let mut current = self
            .current
            .lock()
            .expect("fake clock mutex should not be poisoned");
        *current = current
            .checked_add_days(Days::new(days))
            .expect("fake clock should stay within representable range");
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.current
            .lock()
            .expect("fake clock mutex should not be poisoned")
            .to_owned()
    }
}

#[derive(Debug, Clone)]
pub struct FakeServer {
    inner: Arc<Mutex<FakeServerInner>>,
    clock: FakeClock,
}

#[derive(Debug, Default)]
struct FakeServerInner {
    next_event_seq: i64,
    next_asset_seed: u32,
    registered_devices: HashMap<String, RegisterDeviceRequest>,
    served_cursors: HashMap<String, Option<i64>>,
    notes: HashMap<String, ServerNoteState>,
    assets: HashMap<String, ServerAssetState>,
    events: Vec<SyncChange>,
}

#[derive(Debug, Clone)]
struct ServerNoteState {
    path: String,
    content: String,
    content_hash: String,
    server_revision: i64,
    trashed_at: Option<String>,
    retention_until: Option<String>,
    deleted_at: Option<String>,
}

#[derive(Debug, Clone)]
struct ServerAssetState {
    path: String,
    bytes: Vec<u8>,
    content_hash: String,
    server_revision: i64,
    trashed_at: Option<String>,
    retention_until: Option<String>,
    deleted_at: Option<String>,
}

impl FakeServer {
    pub fn new(clock: FakeClock) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeServerInner {
                next_asset_seed: 1,
                ..FakeServerInner::default()
            })),
            clock,
        }
    }

    pub fn last_event_seq(&self) -> i64 {
        self.inner
            .lock()
            .expect("fake server mutex should not be poisoned")
            .events
            .last()
            .map_or(0, |event| event.event_seq)
    }

    pub fn seed_asset_create(&self, path: &str, bytes: &[u8]) -> AssetId {
        let mut inner = self.lock_inner();
        let asset_id = AssetId::new(&format!("seed-asset-{:04}", inner.next_asset_seed));
        inner.next_asset_seed += 1;

        let content_hash = ContentHash::from_bytes(bytes).to_string();
        let server_revision = 1;
        inner.assets.insert(
            asset_id.to_string(),
            ServerAssetState {
                path: path.to_string(),
                bytes: bytes.to_vec(),
                content_hash: content_hash.clone(),
                server_revision,
                trashed_at: None,
                retention_until: None,
                deleted_at: None,
            },
        );
        let event_seq = next_event_seq(&mut inner);
        push_event(
            &mut inner,
            SyncChange {
                event_seq,
                item_id: asset_id.to_string(),
                item_type: WireItemType::Asset,
                event_type: WireEventType::Create,
                path: Some(path.to_string()),
                content: None,
                asset_bytes_base64: Some(BASE64_STANDARD.encode(bytes)),
                content_hash: Some(content_hash),
                server_revision: Some(server_revision),
                trashed_at: None,
                retention_until: None,
                deleted_at: None,
            },
        );

        asset_id
    }

    pub fn seed_asset_update(&self, asset_id: &AssetId, bytes: &[u8]) {
        let mut inner = self.lock_inner();
        let (path, content_hash, server_revision) = {
            let state = inner
                .assets
                .get_mut(asset_id.as_str())
                .expect("seeded asset should exist before update");
            state.bytes = bytes.to_vec();
            state.content_hash = ContentHash::from_bytes(bytes).to_string();
            state.server_revision += 1;
            (
                state.path.clone(),
                state.content_hash.clone(),
                state.server_revision,
            )
        };
        let event_seq = next_event_seq(&mut inner);

        push_event(
            &mut inner,
            SyncChange {
                event_seq,
                item_id: asset_id.to_string(),
                item_type: WireItemType::Asset,
                event_type: WireEventType::Update,
                path: Some(path),
                content: None,
                asset_bytes_base64: Some(BASE64_STANDARD.encode(bytes)),
                content_hash: Some(content_hash),
                server_revision: Some(server_revision),
                trashed_at: None,
                retention_until: None,
                deleted_at: None,
            },
        );
    }

    pub fn seed_asset_move(&self, asset_id: &AssetId, new_path: &str) {
        let mut inner = self.lock_inner();
        let server_revision = {
            let state = inner
                .assets
                .get_mut(asset_id.as_str())
                .expect("seeded asset should exist before move");
            state.path = new_path.to_string();
            state.server_revision += 1;
            state.server_revision
        };
        let event_seq = next_event_seq(&mut inner);

        push_event(
            &mut inner,
            SyncChange {
                event_seq,
                item_id: asset_id.to_string(),
                item_type: WireItemType::Asset,
                event_type: WireEventType::Move,
                path: Some(new_path.to_string()),
                content: None,
                asset_bytes_base64: None,
                content_hash: None,
                server_revision: Some(server_revision),
                trashed_at: None,
                retention_until: None,
                deleted_at: None,
            },
        );
    }

    pub fn seed_asset_trash(&self, asset_id: &AssetId) {
        let now = format_timestamp(self.clock.now());
        let retention_until = retention_until(self.clock.now(), SERVER_TRASH_RETENTION_DAYS);
        let mut inner = self.lock_inner();
        let (original_path, server_revision) = {
            let state = inner
                .assets
                .get_mut(asset_id.as_str())
                .expect("seeded asset should exist before trash");
            let original_path = state.path.clone();
            state.trashed_at = Some(now.clone());
            state.retention_until = Some(retention_until.clone());
            state.deleted_at = None;
            state.server_revision += 1;
            (original_path, state.server_revision)
        };
        let event_seq = next_event_seq(&mut inner);

        push_event(
            &mut inner,
            SyncChange {
                event_seq,
                item_id: asset_id.to_string(),
                item_type: WireItemType::Asset,
                event_type: WireEventType::Trash,
                path: Some(original_path),
                content: None,
                asset_bytes_base64: None,
                content_hash: None,
                server_revision: Some(server_revision),
                trashed_at: Some(now),
                retention_until: Some(retention_until),
                deleted_at: None,
            },
        );
    }

    pub fn seed_asset_restore(&self, asset_id: &AssetId) {
        let mut inner = self.lock_inner();
        let (path, server_revision) = {
            let state = inner
                .assets
                .get_mut(asset_id.as_str())
                .expect("seeded asset should exist before restore");
            state.trashed_at = None;
            state.retention_until = None;
            state.deleted_at = None;
            state.server_revision += 1;
            (state.path.clone(), state.server_revision)
        };
        let event_seq = next_event_seq(&mut inner);

        push_event(
            &mut inner,
            SyncChange {
                event_seq,
                item_id: asset_id.to_string(),
                item_type: WireItemType::Asset,
                event_type: WireEventType::Restore,
                path: Some(path),
                content: None,
                asset_bytes_base64: None,
                content_hash: None,
                server_revision: Some(server_revision),
                trashed_at: None,
                retention_until: None,
                deleted_at: None,
            },
        );
    }

    pub fn seed_asset_purge(&self, asset_id: &AssetId) {
        let deleted_at = format_timestamp(self.clock.now());
        let mut inner = self.lock_inner();
        let server_revision = {
            let state = inner
                .assets
                .get_mut(asset_id.as_str())
                .expect("seeded asset should exist before purge");
            state.deleted_at = Some(deleted_at.clone());
            state.server_revision += 1;
            state.server_revision
        };
        let event_seq = next_event_seq(&mut inner);

        push_event(
            &mut inner,
            SyncChange {
                event_seq,
                item_id: asset_id.to_string(),
                item_type: WireItemType::Asset,
                event_type: WireEventType::Purge,
                path: None,
                content: None,
                asset_bytes_base64: None,
                content_hash: None,
                server_revision: Some(server_revision),
                trashed_at: None,
                retention_until: None,
                deleted_at: Some(deleted_at),
            },
        );
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, FakeServerInner> {
        self.inner
            .lock()
            .expect("fake server mutex should not be poisoned")
    }
}

#[async_trait]
impl SyncTransport for FakeServer {
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> SyncResult<RegisterDeviceResponse> {
        self.lock_inner()
            .registered_devices
            .insert(request.device_id.clone(), request.clone());

        Ok(RegisterDeviceResponse {
            device_id: request.device_id.clone(),
            server_time: format_timestamp(self.clock.now()),
            trash_retention_days: SERVER_TRASH_RETENTION_DAYS,
            max_batch_size: SERVER_MAX_BATCH_SIZE,
        })
    }

    async fn push(&self, request: &SyncPushRequest) -> SyncResult<SyncPushResponse> {
        let mut inner = self.lock_inner();
        let mut results = Vec::with_capacity(request.notes.len() + request.assets.len());

        for note in &request.notes {
            results.push(push_note_item(&mut inner, &self.clock, note));
        }

        for asset in &request.assets {
            results.push(push_asset_item(&mut inner, &self.clock, asset));
        }

        Ok(SyncPushResponse { results })
    }

    async fn pull(&self, request: &SyncPullRequest) -> SyncResult<SyncPullResponse> {
        let mut inner = self.lock_inner();
        let batch_limit = usize::try_from(request.batch_size.unwrap_or(SERVER_MAX_BATCH_SIZE))
            .expect("batch size should fit in usize");
        let changes = inner
            .events
            .iter()
            .filter(|change| {
                request
                    .cursor
                    .is_none_or(|cursor| change.event_seq > cursor)
            })
            .take(batch_limit)
            .cloned()
            .collect::<Vec<_>>();
        let next_cursor = changes
            .last()
            .map(|change| change.event_seq)
            .or(request.cursor);

        inner
            .served_cursors
            .insert(request.device_id.clone(), next_cursor);

        Ok(SyncPullResponse {
            changes,
            next_cursor,
        })
    }

    async fn probe_remote_counts(&self, _device_id: &DeviceId) -> SyncResult<RemoteBootstrapProbe> {
        let inner = self.lock_inner();
        let note_count = inner
            .notes
            .values()
            .filter(|state| state.deleted_at.is_none())
            .count();
        let asset_count = inner
            .assets
            .values()
            .filter(|state| state.deleted_at.is_none())
            .count();

        Ok(RemoteBootstrapProbe {
            note_count: u32::try_from(note_count).expect("note count should fit in u32"),
            asset_count: u32::try_from(asset_count).expect("asset count should fit in u32"),
        })
    }
}

pub struct ClientFixture {
    vault: TempDir,
    conn: rusqlite::Connection,
    config: EffectiveConfig,
    device_id: DeviceId,
    clock: FakeClock,
    trash_dir: RelativePath,
    next_note_seed: Cell<u32>,
}

impl ClientFixture {
    pub fn new(name: &str, clock: FakeClock) -> Self {
        let vault = TempDir::new().expect("temp vault should be created");
        let conn = open_memory().expect("in-memory database should open");
        ensure_schema(&conn).expect("schema should be ready");

        let config = EffectiveConfig {
            vault_path: vault.path().to_path_buf(),
            ..EffectiveConfig::default()
        };
        let trash_dir = RelativePath::new(&config.trash_dir);

        Self {
            vault,
            conn,
            config,
            device_id: DeviceId::new(&format!("{name}-device")),
            clock,
            trash_dir,
            next_note_seed: Cell::new(1),
        }
    }

    pub fn register(
        &self,
        transport: &dyn SyncTransport,
    ) -> Result<RegisterDeviceResponse, AppError> {
        block_on(register_device(
            transport,
            &self.conn,
            RegisterDeviceRequest {
                device_id: self.device_id.to_string(),
                device_name: self.device_id.to_string(),
                platform: Some("test-os".to_string()),
                app_version: Some("0.1.0".to_string()),
                capabilities: vec!["sync".to_string(), "push".to_string(), "pull".to_string()],
            },
        ))
    }

    pub fn bootstrap(
        &mut self,
        transport: &dyn SyncTransport,
    ) -> Result<BootstrapOutcome, AppError> {
        block_on(bootstrap(transport, self.sync_context()))
    }

    pub fn push(&mut self, transport: &dyn SyncTransport) -> Result<PushApplyOutcome, AppError> {
        block_on(plan_and_push(transport, self.sync_context()))
    }

    pub fn pull(&mut self, transport: &dyn SyncTransport) -> Result<PullApplyOutcome, AppError> {
        block_on(pull_and_apply(transport, self.sync_context()))
    }

    pub fn make_note(&mut self, path: &str, body: &str) -> NoteId {
        let note_id = self.next_note_id();
        create_note(
            self.vault.path(),
            &self.config,
            &mut WriteContext {
                conn: &mut self.conn,
                device_id: &self.device_id,
            },
            CreateNoteRequest {
                path: RelativePath::new(path),
                frontmatter: NoteFrontmatter {
                    id: note_id.clone(),
                    title: file_stem_title(path),
                    tags: vec!["sync".to_string()],
                    note_type: None,
                    aliases: Vec::new(),
                    created: None,
                    updated: None,
                    source: None,
                    status: None,
                    sources: Vec::new(),
                    extra: BTreeMap::default(),
                },
                body_markdown: body.to_string(),
            },
        )
        .expect("note should be created");
        self.rebuild();

        note_id
    }

    pub fn edit_note(&mut self, note_id: &NoteId, body: &str) {
        update_note(
            self.vault.path(),
            &self.config,
            &mut WriteContext {
                conn: &mut self.conn,
                device_id: &self.device_id,
            },
            UpdateNoteRequest {
                note_id: note_id.clone(),
                frontmatter: None,
                body_markdown: Some(body.to_string()),
            },
            None,
        )
        .expect("note should be updated");
        self.rebuild();
    }

    pub fn move_note(&mut self, note_id: &NoteId, new_path: &str) {
        let note = self.note_row(note_id);
        let destination = self.vault.path().join(new_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).expect("move destination parent should be created");
        }
        fs::rename(
            self.vault.path().join(note.current_path.as_str()),
            &destination,
        )
        .expect("note file should be renamed");
        self.rebuild();
    }

    pub fn trash_note(&mut self, note_id: &NoteId) {
        trash_note(
            self.vault.path(),
            &self.config,
            &mut self.conn,
            &TrashRequest {
                kind: ItemKind::Note,
                target: TargetSelector::ById(note_id.to_string()),
                reason: None,
            },
        )
        .expect("note should be trashed");
    }

    pub fn restore_note(&mut self, note_id: &NoteId) {
        restore_note(
            self.vault.path(),
            &self.config,
            &mut self.conn,
            &RestoreRequest {
                kind: ItemKind::Note,
                target: TargetSelector::ById(note_id.to_string()),
                alternate_path: None,
            },
        )
        .expect("note should be restored");
    }

    pub fn purge_expired(&mut self) {
        purge_expired(
            self.vault.path(),
            &self.config,
            &mut self.conn,
            &PurgeRequest {
                scope: PurgeScope::Expired,
                confirm: true,
            },
            &self.clock,
        )
        .expect("expired purge should succeed");
    }

    pub fn note_row(&self, note_id: &NoteId) -> NoteRecord {
        NoteRepo::get_note(&self.conn, note_id)
            .expect("note query should succeed")
            .expect("note row should exist")
    }

    pub fn asset_row(&self, asset_id: &AssetId) -> AssetRecord {
        AssetRepo::get_asset(&self.conn, asset_id)
            .expect("asset query should succeed")
            .expect("asset row should exist")
    }

    pub fn note_state(&self, note_id: &NoteId) -> NoteSyncState {
        SyncStateRepo::get_note_sync_state(&self.conn, note_id)
            .expect("note sync-state query should succeed")
            .expect("note sync-state should exist")
    }

    pub fn note_delete_record(&self, note_id: &NoteId) -> Option<DeleteRecord> {
        DeleteRecordRepo::get_delete_record(&self.conn, &DeleteRecordTarget::Note(note_id.clone()))
            .expect("note delete-record query should succeed")
    }

    pub fn asset_delete_record(&self, asset_id: &AssetId) -> Option<DeleteRecord> {
        DeleteRecordRepo::get_delete_record(
            &self.conn,
            &DeleteRecordTarget::Asset(asset_id.clone()),
        )
        .expect("asset delete-record query should succeed")
    }

    pub fn device_cursor(&self) -> Option<i64> {
        DeviceRepo::get_device(&self.conn, &self.device_id)
            .expect("device query should succeed")
            .map(|record| record.last_sync_cursor)
            .expect("device should exist after registration")
    }

    pub fn note_bytes(&self, path: &str) -> Vec<u8> {
        fs::read(self.vault.path().join(path)).expect("note bytes should be readable")
    }

    pub fn asset_bytes(&self, path: &str) -> Vec<u8> {
        fs::read(self.vault.path().join(path)).expect("asset bytes should be readable")
    }

    pub fn vault_snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        collect_vault_files(self.vault.path(), self.vault.path(), &mut files);
        files
    }

    pub fn file_exists(&self, path: &str) -> bool {
        self.vault.path().join(path).exists()
    }

    pub fn file_modified(&self, path: &str) -> SystemTime {
        fs::metadata(self.vault.path().join(path))
            .expect("file metadata should be readable")
            .modified()
            .expect("file modified time should be readable")
    }

    pub fn note_count(&self) -> u32 {
        NoteRepo::count(&self.conn).expect("note count should succeed")
    }

    fn next_note_id(&self) -> NoteId {
        let next_seed = self.next_note_seed.get();
        self.next_note_seed.set(next_seed + 1);
        NoteId::new(&format!("{}-note-{next_seed:04}", self.device_id.as_str()))
    }

    fn rebuild(&mut self) {
        rebuild_index(
            self.vault.path(),
            &self.config,
            &mut self.conn,
            RebuildOptions::default(),
        )
        .expect("rebuild should succeed");
    }

    fn sync_context(&mut self) -> SyncContext<'_> {
        SyncContext {
            conn: &mut self.conn,
            vault: self.vault.path(),
            device_id: &self.device_id,
            clock: &self.clock,
            trash_dir: &self.trash_dir,
        }
    }
}

fn collect_vault_files(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    let entries = fs::read_dir(current).expect("vault directory should be readable");
    let mut paths = entries
        .map(|entry| entry.expect("vault entry should be readable").path())
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            collect_vault_files(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("vault file should live under the vault root")
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(
                relative,
                fs::read(&path).expect("vault file should be readable"),
            );
        }
    }
}

fn file_stem_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("note")
        .to_string()
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn retention_until(now: DateTime<Utc>, retention_days: u32) -> String {
    format_timestamp(
        now.checked_add_days(Days::new(u64::from(retention_days)))
            .expect("retention window should stay within representable range"),
    )
}

fn push_event(inner: &mut FakeServerInner, change: SyncChange) {
    inner.events.push(change);
}

fn next_event_seq(inner: &mut FakeServerInner) -> i64 {
    inner.next_event_seq += 1;
    inner.next_event_seq
}

#[allow(clippy::too_many_lines)]
fn push_note_item(
    inner: &mut FakeServerInner,
    clock: &FakeClock,
    note: &zk_domain::PushNoteItem,
) -> zk_domain::PushItemResult {
    match note.operation {
        WireOperation::Create => {
            if inner.notes.contains_key(note.note_id.as_str()) {
                return conflict_result(&note.note_id, "item already exists");
            }
            let Some(content) = note.content.clone() else {
                return rejected_result(&note.note_id, "note create is missing content");
            };
            let server_revision = 1;
            inner.notes.insert(
                note.note_id.clone(),
                ServerNoteState {
                    path: note.path.clone(),
                    content: content.clone(),
                    content_hash: note.content_hash.clone(),
                    server_revision,
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: None,
                },
            );
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: note.note_id.clone(),
                    item_type: WireItemType::Note,
                    event_type: WireEventType::Create,
                    path: Some(note.path.clone()),
                    content: Some(content),
                    asset_bytes_base64: None,
                    content_hash: Some(note.content_hash.clone()),
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: None,
                },
            );
            accepted_result(&note.note_id, server_revision)
        }
        WireOperation::Update => {
            let Some(_) = inner.notes.get(note.note_id.as_str()) else {
                return rejected_result(&note.note_id, "note update is missing remote state");
            };
            if note.base_server_revision
                != Some(
                    inner
                        .notes
                        .get(note.note_id.as_str())
                        .expect("checked note should exist")
                        .server_revision,
                )
            {
                return conflict_result(&note.note_id, "server revision conflict");
            }
            let Some(content) = note.content.clone() else {
                return rejected_result(&note.note_id, "note update is missing content");
            };
            let server_revision = {
                let state = inner
                    .notes
                    .get_mut(note.note_id.as_str())
                    .expect("checked note should exist");
                state.path.clone_from(&note.path);
                state.content.clone_from(&content);
                state.content_hash.clone_from(&note.content_hash);
                state.trashed_at = None;
                state.retention_until = None;
                state.deleted_at = None;
                state.server_revision += 1;
                state.server_revision
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: note.note_id.clone(),
                    item_type: WireItemType::Note,
                    event_type: WireEventType::Update,
                    path: Some(note.path.clone()),
                    content: Some(content),
                    asset_bytes_base64: None,
                    content_hash: Some(note.content_hash.clone()),
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: None,
                },
            );
            accepted_result(&note.note_id, server_revision)
        }
        WireOperation::Move => {
            let Some(_) = inner.notes.get(note.note_id.as_str()) else {
                return rejected_result(&note.note_id, "note move is missing remote state");
            };
            if note.base_server_revision
                != Some(
                    inner
                        .notes
                        .get(note.note_id.as_str())
                        .expect("checked note should exist")
                        .server_revision,
                )
            {
                return conflict_result(&note.note_id, "server revision conflict");
            }
            let Some(content) = note.content.clone() else {
                return rejected_result(&note.note_id, "note move is missing content");
            };
            let server_revision = {
                let state = inner
                    .notes
                    .get_mut(note.note_id.as_str())
                    .expect("checked note should exist");
                state.path.clone_from(&note.path);
                state.content = content;
                state.content_hash.clone_from(&note.content_hash);
                state.server_revision += 1;
                state.server_revision
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: note.note_id.clone(),
                    item_type: WireItemType::Note,
                    event_type: WireEventType::Move,
                    path: Some(note.path.clone()),
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: None,
                },
            );
            accepted_result(&note.note_id, server_revision)
        }
        WireOperation::Trash => {
            let now = format_timestamp(clock.now());
            let retention_until = retention_until(clock.now(), SERVER_TRASH_RETENTION_DAYS);
            let Some(_) = inner.notes.get(note.note_id.as_str()) else {
                return rejected_result(&note.note_id, "note trash is missing remote state");
            };
            if note.base_server_revision
                != Some(
                    inner
                        .notes
                        .get(note.note_id.as_str())
                        .expect("checked note should exist")
                        .server_revision,
                )
            {
                return conflict_result(&note.note_id, "server revision conflict");
            }
            let (original_path, server_revision) = {
                let state = inner
                    .notes
                    .get_mut(note.note_id.as_str())
                    .expect("checked note should exist");
                let original_path = state.path.clone();
                state.trashed_at = Some(now.clone());
                state.retention_until = Some(retention_until.clone());
                state.deleted_at = None;
                state.server_revision += 1;
                (original_path, state.server_revision)
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: note.note_id.clone(),
                    item_type: WireItemType::Note,
                    event_type: WireEventType::Trash,
                    path: Some(original_path),
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: Some(now),
                    retention_until: Some(retention_until),
                    deleted_at: None,
                },
            );
            accepted_result(&note.note_id, server_revision)
        }
        WireOperation::Restore => {
            let Some(_) = inner.notes.get(note.note_id.as_str()) else {
                return rejected_result(&note.note_id, "note restore is missing remote state");
            };
            if note.base_server_revision
                != Some(
                    inner
                        .notes
                        .get(note.note_id.as_str())
                        .expect("checked note should exist")
                        .server_revision,
                )
            {
                return conflict_result(&note.note_id, "server revision conflict");
            }
            let server_revision = {
                let state = inner
                    .notes
                    .get_mut(note.note_id.as_str())
                    .expect("checked note should exist");
                state.path.clone_from(&note.path);
                state.trashed_at = None;
                state.retention_until = None;
                state.deleted_at = None;
                state.server_revision += 1;
                state.server_revision
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: note.note_id.clone(),
                    item_type: WireItemType::Note,
                    event_type: WireEventType::Restore,
                    path: Some(note.path.clone()),
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: None,
                },
            );
            accepted_result(&note.note_id, server_revision)
        }
        WireOperation::Delete => {
            let deleted_at = format_timestamp(clock.now());
            let Some(_) = inner.notes.get(note.note_id.as_str()) else {
                return rejected_result(&note.note_id, "note delete is missing remote state");
            };
            if note.base_server_revision
                != Some(
                    inner
                        .notes
                        .get(note.note_id.as_str())
                        .expect("checked note should exist")
                        .server_revision,
                )
            {
                return conflict_result(&note.note_id, "server revision conflict");
            }
            let server_revision = {
                let state = inner
                    .notes
                    .get_mut(note.note_id.as_str())
                    .expect("checked note should exist");
                state.deleted_at = Some(deleted_at.clone());
                state.server_revision += 1;
                state.server_revision
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: note.note_id.clone(),
                    item_type: WireItemType::Note,
                    event_type: WireEventType::Purge,
                    path: None,
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: Some(deleted_at),
                },
            );
            accepted_result(&note.note_id, server_revision)
        }
    }
}

#[allow(clippy::too_many_lines)]
fn push_asset_item(
    inner: &mut FakeServerInner,
    clock: &FakeClock,
    asset: &zk_domain::PushAssetItem,
) -> zk_domain::PushItemResult {
    match asset.operation {
        WireOperation::Create | WireOperation::Update => rejected_result(
            &asset.asset_id,
            "asset binary push bytes are not implemented in the V1 simulation",
        ),
        WireOperation::Move => {
            let Some(_) = inner.assets.get(asset.asset_id.as_str()) else {
                return rejected_result(&asset.asset_id, "asset move is missing remote state");
            };
            if asset.base_server_revision
                != Some(
                    inner
                        .assets
                        .get(asset.asset_id.as_str())
                        .expect("checked asset should exist")
                        .server_revision,
                )
            {
                return conflict_result(&asset.asset_id, "server revision conflict");
            }
            let server_revision = {
                let state = inner
                    .assets
                    .get_mut(asset.asset_id.as_str())
                    .expect("checked asset should exist");
                state.path.clone_from(&asset.path);
                state.content_hash.clone_from(&asset.content_hash);
                state.server_revision += 1;
                state.server_revision
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: asset.asset_id.clone(),
                    item_type: WireItemType::Asset,
                    event_type: WireEventType::Move,
                    path: Some(asset.path.clone()),
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: None,
                },
            );
            accepted_result(&asset.asset_id, server_revision)
        }
        WireOperation::Trash => {
            let now = format_timestamp(clock.now());
            let retention_until = retention_until(clock.now(), SERVER_TRASH_RETENTION_DAYS);
            let Some(_) = inner.assets.get(asset.asset_id.as_str()) else {
                return rejected_result(&asset.asset_id, "asset trash is missing remote state");
            };
            if asset.base_server_revision
                != Some(
                    inner
                        .assets
                        .get(asset.asset_id.as_str())
                        .expect("checked asset should exist")
                        .server_revision,
                )
            {
                return conflict_result(&asset.asset_id, "server revision conflict");
            }
            let (original_path, server_revision) = {
                let state = inner
                    .assets
                    .get_mut(asset.asset_id.as_str())
                    .expect("checked asset should exist");
                let original_path = state.path.clone();
                state.trashed_at = Some(now.clone());
                state.retention_until = Some(retention_until.clone());
                state.deleted_at = None;
                state.server_revision += 1;
                (original_path, state.server_revision)
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: asset.asset_id.clone(),
                    item_type: WireItemType::Asset,
                    event_type: WireEventType::Trash,
                    path: Some(original_path),
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: Some(now),
                    retention_until: Some(retention_until),
                    deleted_at: None,
                },
            );
            accepted_result(&asset.asset_id, server_revision)
        }
        WireOperation::Restore => {
            let Some(_) = inner.assets.get(asset.asset_id.as_str()) else {
                return rejected_result(&asset.asset_id, "asset restore is missing remote state");
            };
            if asset.base_server_revision
                != Some(
                    inner
                        .assets
                        .get(asset.asset_id.as_str())
                        .expect("checked asset should exist")
                        .server_revision,
                )
            {
                return conflict_result(&asset.asset_id, "server revision conflict");
            }
            let server_revision = {
                let state = inner
                    .assets
                    .get_mut(asset.asset_id.as_str())
                    .expect("checked asset should exist");
                state.path.clone_from(&asset.path);
                state.trashed_at = None;
                state.retention_until = None;
                state.deleted_at = None;
                state.server_revision += 1;
                state.server_revision
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: asset.asset_id.clone(),
                    item_type: WireItemType::Asset,
                    event_type: WireEventType::Restore,
                    path: Some(asset.path.clone()),
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: None,
                },
            );
            accepted_result(&asset.asset_id, server_revision)
        }
        WireOperation::Delete => {
            let deleted_at = format_timestamp(clock.now());
            let Some(_) = inner.assets.get(asset.asset_id.as_str()) else {
                return rejected_result(&asset.asset_id, "asset delete is missing remote state");
            };
            if asset.base_server_revision
                != Some(
                    inner
                        .assets
                        .get(asset.asset_id.as_str())
                        .expect("checked asset should exist")
                        .server_revision,
                )
            {
                return conflict_result(&asset.asset_id, "server revision conflict");
            }
            let server_revision = {
                let state = inner
                    .assets
                    .get_mut(asset.asset_id.as_str())
                    .expect("checked asset should exist");
                state.deleted_at = Some(deleted_at.clone());
                state.server_revision += 1;
                state.server_revision
            };
            let event_seq = next_event_seq(inner);
            push_event(
                inner,
                SyncChange {
                    event_seq,
                    item_id: asset.asset_id.clone(),
                    item_type: WireItemType::Asset,
                    event_type: WireEventType::Purge,
                    path: None,
                    content: None,
                    asset_bytes_base64: None,
                    content_hash: None,
                    server_revision: Some(server_revision),
                    trashed_at: None,
                    retention_until: None,
                    deleted_at: Some(deleted_at),
                },
            );
            accepted_result(&asset.asset_id, server_revision)
        }
    }
}

fn accepted_result(item_id: &str, server_revision: i64) -> zk_domain::PushItemResult {
    zk_domain::PushItemResult {
        item_id: item_id.to_string(),
        accepted: true,
        conflict: false,
        new_server_revision: Some(server_revision),
        canonical_path: None,
        message: None,
    }
}

fn conflict_result(item_id: &str, message: &str) -> zk_domain::PushItemResult {
    zk_domain::PushItemResult {
        item_id: item_id.to_string(),
        accepted: false,
        conflict: true,
        new_server_revision: None,
        canonical_path: None,
        message: Some(message.to_string()),
    }
}

fn rejected_result(item_id: &str, message: &str) -> zk_domain::PushItemResult {
    zk_domain::PushItemResult {
        item_id: item_id.to_string(),
        accepted: false,
        conflict: false,
        new_server_revision: None,
        canonical_path: None,
        message: Some(message.to_string()),
    }
}
