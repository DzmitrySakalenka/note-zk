// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, sync::Arc, time::Duration};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tokio::time::{sleep, timeout};
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
    })
    .expect("server state should bootstrap")
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
