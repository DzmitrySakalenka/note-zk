// Integration benchmarks inherit dev-dependencies used by tests in this crate.
#![allow(unused_crate_dependencies)]

use std::{fs, path::Path};

use chrono::{DateTime, TimeZone, Utc};
use divan::{AllocProfiler, Bencher, black_box, black_box_drop, counter::ItemsCount};
use rusqlite::Connection;
use tempfile::TempDir;
use ulid::Ulid;
use zk_domain::{
    SyncChange, SyncPullResponse, WireEventType, WireItemType,
    id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::SyncStatus,
    sync_state::{DeviceRecord, NoteSyncState},
};
use zk_index::{DeviceRepo, NoteRepo, SyncStateRepo, ensure_schema, open_or_create};
use zk_sync::{
    plan::{Clock, NotePlannedOperation, PlannedOp, PlannedOperation},
    pull::{PullApplyContext, apply_pull_response},
    push::{PushBuildContext, build_push_request},
};

const NOTE_COUNT: usize = 10_000;
const FIXED_ULID_TIMESTAMP_MS: u64 = 1_776_552_453_000;
const ID_SEED: u64 = 0x5e_ed_16_37_ba_5e_1d_00;

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
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

#[derive(Debug)]
struct NoteIdentity {
    id: String,
    current_path: String,
    remote_path: String,
}

struct PullFixture {
    vault: TempDir,
    database: TempDir,
    conn: Connection,
    device_id: DeviceId,
    clock: FixedClock,
    trash_dir: RelativePath,
}

struct PushFixture {
    vault: TempDir,
    device_id: DeviceId,
    planned: Vec<PlannedOperation>,
}

#[divan::bench(threads = 1, sample_count = 5, sample_size = 1)]
fn apply_pull_response_10k_notes(bencher: Bencher) {
    let identities = note_identities();
    let response = pull_response_10k_notes(&identities);

    bencher
        .counter(ItemsCount::new(NOTE_COUNT))
        .with_inputs(|| pull_fixture(&identities))
        .bench_local_refs(|fixture| {
            let PullFixture {
                vault,
                database: _database,
                conn,
                device_id,
                clock,
                trash_dir,
            } = fixture;
            let ctx = PullApplyContext {
                vault: vault.path(),
                device_id,
                clock,
                trash_dir,
            };
            let outcome = apply_pull_response(conn, &ctx, black_box(&response))
                .expect("pull response benchmark fixture should apply");

            black_box_drop(outcome);
        });
}

#[divan::bench(threads = 1, sample_count = 5, sample_size = 1)]
fn build_push_request_10k_local_changes(bencher: Bencher) {
    let identities = note_identities();
    let fixture = push_fixture(&identities);

    bencher
        .counter(ItemsCount::new(NOTE_COUNT))
        .bench_local(|| {
            let ctx = PushBuildContext {
                device_id: &fixture.device_id,
                vault: fixture.vault.path(),
            };
            let request = build_push_request(&ctx, black_box(&fixture.planned))
                .expect("push request benchmark fixture should build");

            black_box_drop(request);
        });
}

fn note_identities() -> Vec<NoteIdentity> {
    let mut rng = SplitMix64::new(ID_SEED);

    (0..NOTE_COUNT)
        .map(|index| {
            let random = ((u128::from(rng.next()) << 16) | u128::from(rng.next() & 0xffff))
                & ((1_u128 << 80) - 1);
            let id = Ulid::from_parts(FIXED_ULID_TIMESTAMP_MS + index as u64, random).to_string();
            let current_path = format!("notes/{id}.md");
            let remote_path = current_path.clone();

            NoteIdentity {
                id,
                current_path,
                remote_path,
            }
        })
        .collect()
}

fn pull_response_10k_notes(identities: &[NoteIdentity]) -> SyncPullResponse {
    let changes = identities
        .iter()
        .enumerate()
        .map(|(index, identity)| SyncChange {
            event_seq: i64::try_from(index + 1).expect("benchmark event_seq should fit i64"),
            item_id: identity.id.clone(),
            item_type: WireItemType::Note,
            event_type: WireEventType::Move,
            path: Some(identity.remote_path.clone()),
            content: None,
            asset_bytes_base64: None,
            content_hash: None,
            server_revision: Some(
                i64::try_from(NOTE_COUNT + index + 1).expect("benchmark revision should fit i64"),
            ),
            trashed_at: None,
            retention_until: None,
            deleted_at: None,
        })
        .collect();

    SyncPullResponse {
        changes,
        next_cursor: Some(i64::try_from(NOTE_COUNT).expect("benchmark cursor should fit i64")),
    }
}

fn pull_fixture(identities: &[NoteIdentity]) -> PullFixture {
    let vault = TempDir::new().expect("benchmark vault tempdir should create");
    let database = TempDir::new().expect("benchmark database tempdir should create");
    let conn =
        open_or_create(&database.path().join("sync.db")).expect("benchmark database should open");
    ensure_schema(&conn).expect("benchmark schema should apply");

    let device_id = device_id();
    DeviceRepo::upsert_device(&conn, &device_record(&device_id))
        .expect("benchmark device should insert");

    for (index, identity) in identities.iter().enumerate() {
        seed_note_row(vault.path(), &conn, &device_id, identity, index);
    }

    PullFixture {
        vault,
        database,
        conn,
        device_id,
        clock: fixed_clock(),
        trash_dir: RelativePath::new(".zk-trash"),
    }
}

fn seed_note_row(
    vault: &Path,
    conn: &Connection,
    device_id: &DeviceId,
    identity: &NoteIdentity,
    index: usize,
) {
    let note_id = NoteId::new(&identity.id);
    let path = RelativePath::new(&identity.current_path);
    let content = local_note_content(identity, index);
    let content_hash = ContentHash::from_bytes(content.as_bytes());
    let server_revision =
        ServerRevision::new(i64::try_from(index + 1).expect("benchmark revision should fit i64"));
    let title = local_title(index);
    let frontmatter_json = format!(
        r#"{{"id":"{}","title":"{}","created":"2026-04-01T00:00:00Z","updated":"2026-04-01T00:00:00Z"}}"#,
        identity.id, title
    );

    write_file(vault, &identity.current_path, content.as_bytes());
    NoteRepo::upsert_note_with_server_revision(
        conn,
        &note_id,
        &path,
        &title,
        &content_hash,
        &frontmatter_json,
        "2026-04-01T00:00:00Z",
        "2026-04-01T00:00:00Z",
        None,
        false,
        &server_revision,
    )
    .expect("benchmark note row should insert");
    SyncStateRepo::upsert_note_sync_state(
        conn,
        &NoteSyncState {
            item_id: note_id,
            device_id: device_id.clone(),
            last_local_hash: Some(content_hash.clone()),
            last_synced_hash: Some(content_hash),
            last_synced_path: Some(path),
            last_known_server_revision: Some(server_revision),
            sync_status: SyncStatus::Synced,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: None,
        },
    )
    .expect("benchmark sync state should insert");
}

fn push_fixture(identities: &[NoteIdentity]) -> PushFixture {
    let vault = TempDir::new().expect("benchmark vault tempdir should create");
    let planned = identities
        .iter()
        .enumerate()
        .map(|(index, identity)| {
            PlannedOperation::Note(NotePlannedOperation {
                note_id: NoteId::new(&identity.id),
                current_path: RelativePath::new(&identity.current_path),
                op: metadata_only_operation(index),
                local_hash: ContentHash::from_bytes(
                    format!("local hash source {index}").as_bytes(),
                ),
                base_server_revision: Some(ServerRevision::new(
                    i64::try_from(index + 1).expect("benchmark revision should fit i64"),
                )),
            })
        })
        .collect();

    PushFixture {
        vault,
        device_id: device_id(),
        planned,
    }
}

fn metadata_only_operation(index: usize) -> PlannedOp {
    match index % 3 {
        0 => PlannedOp::Trash,
        1 => PlannedOp::Restore,
        _ => PlannedOp::Delete,
    }
}

fn device_record(device_id: &DeviceId) -> DeviceRecord {
    DeviceRecord {
        device_id: device_id.clone(),
        device_name: "Benchmark laptop".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["pull".to_string()],
        created_at: "2026-04-01T00:00:00Z".to_string(),
        last_seen_at: "2026-04-22T18:07:33Z".to_string(),
        last_sync_cursor: Some(0),
    }
}

fn local_note_content(identity: &NoteIdentity, index: usize) -> String {
    note_content(
        &identity.id,
        &local_title(index),
        "2026-04-01T00:00:00Z",
        index,
    )
}

fn note_content(note_id: &str, title: &str, updated: &str, index: usize) -> String {
    format!(
        "---\nid: {note_id}\ntitle: {title}\ncreated: 2026-04-01T00:00:00Z\nupdated: {updated}\n---\n# {title}\n\nBenchmark body {index}\n"
    )
}

fn local_title(index: usize) -> String {
    format!("Benchmark note {index}")
}

fn write_file(vault: &Path, relative_path: &str, bytes: &[u8]) {
    let absolute_path = vault.join(relative_path);
    fs::create_dir_all(
        absolute_path
            .parent()
            .expect("benchmark note path should have a parent"),
    )
    .expect("benchmark note parent should create");
    fs::write(absolute_path, bytes).expect("benchmark note should write");
}

fn device_id() -> DeviceId {
    DeviceId::new("device-benchmark")
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        now: Utc
            .with_ymd_and_hms(2026, 4, 22, 18, 7, 33)
            .single()
            .expect("fixed clock timestamp should be valid"),
    }
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}
