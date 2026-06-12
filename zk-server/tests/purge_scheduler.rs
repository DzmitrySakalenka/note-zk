// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, net::SocketAddr, sync::Arc, time::Duration};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tempfile::{Builder as TempDirBuilder, TempDir};
use tokio::time::{advance, sleep, timeout};
use tokio_util::sync::CancellationToken;
use zk_server::{
    config::ServerConfig,
    repo::{
        AssetRow, AssetsRepo, DeleteRecordRow, DeleteRecordsRepo, NoteRow, NotesRepo,
        SyncEventsRepo,
    },
    scheduler::purge::{PurgeStats, run_purge_once, spawn_purge_scheduler},
    state::{ServerState, bootstrap_with_config},
};

mod common;

use common::unique_test_dir;

const PURGE_INTERVAL_FOR_TEST: Duration = Duration::from_secs(3_600);

#[test]
fn run_purge_once_purges_expired_items_and_leaves_future_items() {
    let state = fresh_state("purge-scheduler-expired");
    let now = Utc::now();
    let expired_retention = now - ChronoDuration::hours(1);
    let future_retention = now + ChronoDuration::hours(1);
    let expired_note = note_row("expired-note", 3, now);
    let expired_asset = asset_row("expired-asset", 5, now);
    let future_note = note_row("future-note", 7, now);

    state
        .db()
        .with_tx(|transaction| {
            NotesRepo::create(transaction, &expired_note)?;
            AssetsRepo::create(transaction, &expired_asset)?;
            NotesRepo::create(transaction, &future_note)?;
            DeleteRecordsRepo::upsert(
                transaction,
                &delete_record("expired-note", "note", expired_retention, now),
            )?;
            DeleteRecordsRepo::upsert(
                transaction,
                &delete_record("expired-asset", "asset", expired_retention, now),
            )?;
            DeleteRecordsRepo::upsert(
                transaction,
                &delete_record("future-note", "note", future_retention, now),
            )?;
            Ok(())
        })
        .expect("fixture rows should insert");

    let purge_stats = run_purge_once(&state).expect("purge should succeed");

    assert_eq!(
        purge_stats,
        PurgeStats {
            notes_purged: 1,
            assets_purged: 1,
            total: 2,
        }
    );
    state
        .db()
        .with_conn(|connection| {
            assert!(NotesRepo::get(connection, "expired-note")?.is_none());
            assert!(AssetsRepo::get(connection, "expired-asset")?.is_none());
            assert!(NotesRepo::get(connection, "future-note")?.is_some());

            let purged_note = DeleteRecordsRepo::get(connection, "expired-note", "note")?
                .expect("purged note delete record should remain");
            let purged_asset = DeleteRecordsRepo::get(connection, "expired-asset", "asset")?
                .expect("purged asset delete record should remain");
            let future = DeleteRecordsRepo::get(connection, "future-note", "note")?
                .expect("future delete record should remain");
            assert!(purged_note.deleted_at.is_some());
            assert_eq!(purged_note.delete_revision, Some(4));
            assert!(purged_asset.deleted_at.is_some());
            assert_eq!(purged_asset.delete_revision, Some(6));
            assert_eq!(future.deleted_at, None);
            assert_eq!(future.delete_revision, None);

            let events = SyncEventsRepo::list_since(connection, 0, 10)?;
            assert_eq!(events.len(), 2);
            assert!(events.iter().any(|event| {
                event.item_id == "expired-note"
                    && event.item_type == "note"
                    && event.event_type == "purge"
                    && event.server_revision == Some(4)
                    && event.device_id.is_none()
            }));
            assert!(events.iter().any(|event| {
                event.item_id == "expired-asset"
                    && event.item_type == "asset"
                    && event.event_type == "purge"
                    && event.server_revision == Some(6)
                    && event.device_id.is_none()
            }));
            Ok(())
        })
        .expect("purge results should be queryable");
}

#[test]
fn run_purge_once_removes_expired_note_file_from_server_vault() {
    let fixture = fresh_temp_state("purge-scheduler-vault-file");
    let state = &fixture.state;
    let now = Utc::now();
    let expired_retention = now - ChronoDuration::hours(1);
    let expired_note = note_row("expired-note-file", 3, now);
    let note_path = state.config().vault_path.join(&expired_note.current_path);

    fs::create_dir_all(
        note_path
            .parent()
            .expect("note fixture path should have a parent"),
    )
    .expect("note fixture parent should be created");
    fs::write(&note_path, &expired_note.body_markdown).expect("note fixture should be written");

    state
        .db()
        .with_tx(|transaction| {
            NotesRepo::create(transaction, &expired_note)?;
            DeleteRecordsRepo::upsert(
                transaction,
                &delete_record("expired-note-file", "note", expired_retention, now),
            )?;
            Ok(())
        })
        .expect("expired note fixture should insert");

    let purge_stats = run_purge_once(state).expect("purge should succeed");

    assert_eq!(
        purge_stats,
        PurgeStats {
            notes_purged: 1,
            assets_purged: 0,
            total: 1,
        }
    );
    assert!(!note_path.exists());
}

#[test]
fn run_purge_once_with_no_expired_records_returns_zero_stats() {
    let state = fresh_state("purge-scheduler-empty");
    let now = Utc::now();
    let future_retention = now + ChronoDuration::hours(1);
    let future_note = note_row("future-note", 1, now);

    state
        .db()
        .with_tx(|transaction| {
            NotesRepo::create(transaction, &future_note)?;
            DeleteRecordsRepo::upsert(
                transaction,
                &delete_record("future-note", "note", future_retention, now),
            )?;
            Ok(())
        })
        .expect("fixture rows should insert");

    let purge_stats = run_purge_once(&state).expect("purge should succeed");

    assert_eq!(purge_stats, PurgeStats::default());
    state
        .db()
        .with_conn(|connection| {
            assert!(NotesRepo::get(connection, "future-note")?.is_some());
            let future = DeleteRecordsRepo::get(connection, "future-note", "note")?
                .expect("future delete record should remain");
            assert_eq!(future.deleted_at, None);
            assert!(SyncEventsRepo::list_since(connection, 0, 10)?.is_empty());
            Ok(())
        })
        .expect("empty purge results should be queryable");
}

#[test]
fn run_purge_once_processes_all_expired_records_across_batches() {
    let state = fresh_state("purge-scheduler-batches");
    let now = Utc::now();
    let expired_retention = now - ChronoDuration::hours(1);

    state
        .db()
        .with_tx(|transaction| {
            for index in 0..1_500 {
                let note_id = format!("note-{index:04}");
                let note = note_row(&note_id, 1, now);
                NotesRepo::create(transaction, &note)?;
                DeleteRecordsRepo::upsert(
                    transaction,
                    &delete_record(&note_id, "note", expired_retention, now),
                )?;
            }
            Ok(())
        })
        .expect("fixture rows should insert");

    let purge_stats = run_purge_once(&state).expect("purge should succeed");

    assert_eq!(
        purge_stats,
        PurgeStats {
            notes_purged: 1_500,
            assets_purged: 0,
            total: 1_500,
        }
    );
    state
        .db()
        .with_conn(|connection| {
            let expired = DeleteRecordsRepo::list_expired(connection, Utc::now(), 10)?;
            let events = SyncEventsRepo::list_since(connection, 0, 2_000)?;
            assert!(expired.is_empty());
            assert_eq!(events.len(), 1_500);
            assert!(NotesRepo::get(connection, "note-0000")?.is_none());
            assert!(NotesRepo::get(connection, "note-1499")?.is_none());
            Ok(())
        })
        .expect("batch purge results should be queryable");
}

#[tokio::test]
async fn shutdown_mid_sleep_cancels_cleanly() {
    let state = Arc::new(fresh_state("purge-scheduler-shutdown"));
    let shutdown = CancellationToken::new();

    let handle = spawn_purge_scheduler(Arc::clone(&state), shutdown.clone());
    sleep(Duration::from_millis(50)).await;
    shutdown.cancel();

    timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("purge scheduler should join promptly");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn spawned_scheduler_purges_on_immediate_and_hourly_ticks_exactly_once() {
    let fixture = fresh_temp_state("purge-scheduler-paused");
    let state = Arc::new(fixture.state.clone());
    let now = Utc::now();
    let shutdown = CancellationToken::new();

    insert_expired_note(&state, "immediate-note", 11, now);

    let handle = spawn_purge_scheduler(Arc::clone(&state), shutdown.clone());
    wait_for_purge_event_count(&state, 1).await;
    assert_note_purged_once(&state, "immediate-note", 1);

    insert_expired_note(&state, "hourly-note", 21, Utc::now());
    tokio::task::yield_now().await;
    assert_note_is_present(&state, "hourly-note");
    assert_eq!(purge_event_count(&state), 1);

    advance(
        PURGE_INTERVAL_FOR_TEST
            .checked_sub(Duration::from_secs(1))
            .expect("test interval is longer than one second"),
    )
    .await;
    tokio::task::yield_now().await;
    assert_note_is_present(&state, "hourly-note");
    assert_eq!(purge_event_count(&state), 1);

    advance(Duration::from_secs(1)).await;
    wait_for_purge_event_count(&state, 2).await;
    assert_note_purged_once(&state, "hourly-note", 2);

    advance(PURGE_INTERVAL_FOR_TEST).await;
    tokio::task::yield_now().await;
    assert_eq!(purge_event_count(&state), 2);

    shutdown.cancel();
    timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("purge scheduler should join after cancellation");
}

struct TempState {
    _root: TempDir,
    state: ServerState,
}

fn fresh_temp_state(prefix: &str) -> TempState {
    let temp_parent = std::path::Path::new("tmp").join("zk-server");
    std::fs::create_dir_all(&temp_parent).expect("test temp parent should be created");
    let root = TempDirBuilder::new()
        .prefix(prefix)
        .tempdir_in(temp_parent)
        .expect("temp server root should be created");
    let vault_path = root.path().join("vault");
    let database_path = root.path().join("db").join("server.sqlite");
    std::fs::create_dir_all(&vault_path).expect("vault directory should be created");
    std::fs::create_dir_all(
        database_path
            .parent()
            .expect("database path should have a parent"),
    )
    .expect("database directory should be created");

    let state = bootstrap_with_config(ServerConfig {
        vault_path,
        database_path,
        listen_addr: "127.0.0.1:0"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai: None,
    })
    .expect("server state should bootstrap");

    TempState { _root: root, state }
}

fn fresh_state(prefix: &str) -> ServerState {
    let root = unique_test_dir(prefix);
    let vault_path = root.join("vault");
    let database_path = root.join("db").join("server.sqlite");
    std::fs::create_dir_all(&vault_path).expect("vault directory should be created");
    std::fs::create_dir_all(
        database_path
            .parent()
            .expect("database path should have a parent"),
    )
    .expect("database directory should be created");

    bootstrap_with_config(ServerConfig {
        vault_path,
        database_path,
        listen_addr: "127.0.0.1:0"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai: None,
    })
    .expect("server state should bootstrap")
}

fn insert_expired_note(state: &ServerState, note_id: &str, revision: i64, now: DateTime<Utc>) {
    let expired_retention = now - ChronoDuration::hours(1);
    let note = note_row(note_id, revision, now);

    state
        .db()
        .with_tx(|transaction| {
            NotesRepo::create(transaction, &note)?;
            DeleteRecordsRepo::upsert(
                transaction,
                &delete_record(note_id, "note", expired_retention, now),
            )?;
            Ok(())
        })
        .expect("expired note fixture should insert");
}

async fn wait_for_purge_event_count(state: &ServerState, expected_count: usize) {
    for _ in 0..1_000 {
        if purge_event_count(state) == expected_count {
            return;
        }

        sleep(Duration::from_millis(1)).await;
    }

    assert_eq!(purge_event_count(state), expected_count);
}

fn purge_event_count(state: &ServerState) -> usize {
    state
        .db()
        .with_conn(|connection| {
            Ok(SyncEventsRepo::list_since(connection, 0, 100)?
                .into_iter()
                .filter(|event| event.event_type == "purge")
                .count())
        })
        .expect("purge event count should query")
}

fn assert_note_is_present(state: &ServerState, note_id: &str) {
    state
        .db()
        .with_conn(|connection| {
            assert!(NotesRepo::get(connection, note_id)?.is_some());
            Ok(())
        })
        .expect("note presence should query");
}

fn assert_note_purged_once(state: &ServerState, note_id: &str, expected_event_count: usize) {
    state
        .db()
        .with_conn(|connection| {
            assert!(NotesRepo::get(connection, note_id)?.is_none());
            let delete_record = DeleteRecordsRepo::get(connection, note_id, "note")?
                .expect("purged delete record should remain");
            assert!(delete_record.deleted_at.is_some());
            assert!(delete_record.delete_revision.is_some());

            let matching_events = SyncEventsRepo::list_since(connection, 0, 100)?
                .into_iter()
                .filter(|event| {
                    event.item_id == note_id
                        && event.item_type == "note"
                        && event.event_type == "purge"
                })
                .count();
            assert_eq!(matching_events, 1);
            Ok(())
        })
        .expect("purged note state should query");

    assert_eq!(purge_event_count(state), expected_event_count);
}

fn note_row(note_id: &str, revision: i64, now: DateTime<Utc>) -> NoteRow {
    NoteRow {
        note_id: note_id.to_string(),
        current_path: format!("notes/{note_id}.md"),
        title: format!("Title {note_id}"),
        content_hash: format!("hash-{note_id}-{revision}"),
        body_markdown: format!("# {note_id}\nrevision {revision}"),
        frontmatter_json: format!(r#"{{"id":"{note_id}"}}"#),
        created_at: now - ChronoDuration::days(1),
        updated_at: now,
        trashed_at: Some(now - ChronoDuration::hours(2)),
        deleted_at: None,
        is_trashed: true,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: revision,
    }
}

fn asset_row(asset_id: &str, revision: i64, now: DateTime<Utc>) -> AssetRow {
    AssetRow {
        asset_id: asset_id.to_string(),
        current_path: format!("assets/{asset_id}.png"),
        content_hash: format!("hash-{asset_id}-{revision}"),
        mime_type: Some("image/png".to_string()),
        size_bytes: 4096,
        created_at: now - ChronoDuration::days(1),
        updated_at: now,
        trashed_at: Some(now - ChronoDuration::hours(2)),
        deleted_at: None,
        is_trashed: true,
        is_deleted: false,
        server_revision: revision,
    }
}

fn delete_record(
    item_id: &str,
    item_type: &str,
    retention_until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> DeleteRecordRow {
    DeleteRecordRow {
        item_id: item_id.to_string(),
        item_type: item_type.to_string(),
        original_path: format!("vault/{item_id}"),
        trashed_at: now - ChronoDuration::hours(2),
        retention_until,
        deleted_at: None,
        delete_revision: None,
    }
}
