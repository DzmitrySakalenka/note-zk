use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

mod common;

use chrono::{DateTime, Utc};
use common::{unique_test_dir, write_text};
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};
use zk_app::{
    AppError, Clock, RebuildOptions, RebuildOutcome, doctor_report, purge_expired, rebuild_index,
    restore_note, scan_vault, trash_note,
};
use zk_domain::{
    ItemKind, PurgeRequest, PurgeScope, RestoreRequest, TargetSelector, TrashRequest,
    config::{EffectiveConfig, FolderPolicy},
    id::{NoteId, RelativePath},
    sync::ItemType,
};
use zk_index::{DeleteRecordRepo, NoteRepo, open_memory};

#[test]
fn manual_trash_rebuild_marks_note_trashed_and_creates_delete_record() {
    let root = unique_test_dir("lifecycle-manual-trash");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01LIFECYCLEMANUALTRASH00001");
    let live_path = RelativePath::new("notes/foo.md");
    let trash_path = RelativePath::new("99_trash/foo.md");
    write_note(
        &vault_root.join(live_path.as_str()),
        note_id.as_str(),
        "Foo",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config);

    move_relative(&vault_root, &live_path, &trash_path);
    let scan_outcome = scan_vault(&vault_root, &config).expect("scan should succeed");
    assert_eq!(scan_outcome.scan_result.trashed_note_count, 1);

    let reconciliation = reconciliation_from(
        rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
            .expect("rebuild after manual trash should succeed"),
    );
    assert_eq!(reconciliation.inserted, 1);

    let note_row = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("note row should exist");
    assert_eq!(note_row.current_path, trash_path);
    assert!(note_row.is_trashed);
    assert!(note_row.trashed_at.is_some());

    let delete_record = fetch_note_delete_record(&conn, &note_id);
    assert_eq!(delete_record.item_id, note_id.as_str());
    assert_eq!(delete_record.item_type, ItemType::Note);
    assert!(delete_record.deleted_at.is_none());
    assert!(
        !delete_record.retention_until.is_empty(),
        "rebuild should compute a retention horizon"
    );
}

#[test]
fn manual_restore_rebuild_clears_trash_fields_and_delete_record() {
    let root = unique_test_dir("lifecycle-manual-restore");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01LIFECYCLEMANUALRESTORE000");
    let live_path = RelativePath::new("notes/foo.md");
    let trash_path = RelativePath::new("99_trash/foo.md");
    write_note(
        &vault_root.join(live_path.as_str()),
        note_id.as_str(),
        "Foo",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config);
    move_relative(&vault_root, &live_path, &trash_path);
    rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("rebuild after manual trash should succeed");

    move_relative(&vault_root, &trash_path, &live_path);
    let reconciliation = reconciliation_from(
        rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
            .expect("rebuild after manual restore should succeed"),
    );
    assert_eq!(reconciliation.cleared, 1);

    let note_row = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("restored note row should exist");
    assert_eq!(note_row.current_path, live_path);
    assert!(!note_row.is_trashed);
    assert!(note_row.trashed_at.is_none());
    assert!(
        DeleteRecordRepo::get_delete_record(&conn, note_id.as_str(), ItemType::Note)
            .expect("delete-record query should succeed")
            .is_none()
    );
}

#[test]
fn api_trash_and_restore_round_trip_moves_file_and_clears_row() {
    let root = unique_test_dir("lifecycle-api-roundtrip");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01LIFECYCLEAPIROUNDTRIP0001");
    let live_path = RelativePath::new("notes/alpha.md");
    write_note(
        &vault_root.join(live_path.as_str()),
        note_id.as_str(),
        "Alpha",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config);
    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    assert!(!vault_root.join(live_path.as_str()).exists());
    assert!(vault_root.join(trashed.new_path.as_str()).exists());
    assert!(
        DeleteRecordRepo::get_delete_record(&conn, note_id.as_str(), ItemType::Note)
            .expect("delete-record query should succeed")
            .is_some()
    );

    let restored = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect("restore note should succeed");

    assert_eq!(restored.restored_to, live_path);
    assert!(vault_root.join(live_path.as_str()).exists());
    assert!(!vault_root.join(trashed.new_path.as_str()).exists());

    let note_row = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("restored note row should exist");
    assert_eq!(note_row.current_path, live_path);
    assert!(!note_row.is_trashed);
    assert!(note_row.trashed_at.is_none());
    assert!(
        DeleteRecordRepo::get_delete_record(&conn, note_id.as_str(), ItemType::Note)
            .expect("delete-record query should succeed")
            .is_none()
    );
}

#[test]
fn purge_expired_dry_run_then_confirmed_creates_tombstone() {
    let root = unique_test_dir("lifecycle-purge-expired");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01LIFECYCLEPURGEEXPIRED0001");
    let live_path = RelativePath::new("notes/expired.md");
    write_note(
        &vault_root.join(live_path.as_str()),
        note_id.as_str(),
        "Expired",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config);
    let trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    age_note_delete_record(
        &conn,
        &note_id,
        "2000-01-01T00:00:00Z",
        "2000-01-02T00:00:00Z",
    );
    let clock = fixed_clock("2026-04-21T12:00:00Z");

    let planned = purge_expired(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: false,
        },
        &clock,
    )
    .expect("dry-run purge should succeed");
    assert_eq!(planned.planned.len(), 1);
    assert!(planned.executed.is_empty());
    assert!(vault_root.join(trashed.new_path.as_str()).exists());

    let confirmed = purge_expired(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: true,
        },
        &clock,
    )
    .expect("confirmed purge should succeed");
    assert_eq!(confirmed.executed.len(), 1);
    assert!(confirmed.skipped.is_empty());
    assert!(!vault_root.join(trashed.new_path.as_str()).exists());

    let note_row = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("note tombstone should exist");
    assert!(note_row.is_deleted);
    assert!(!note_row.is_trashed);
    assert_eq!(note_row.current_path, trashed.new_path);

    let delete_record = fetch_note_delete_record(&conn, &note_id);
    assert!(delete_record.deleted_at.is_some());
}

#[test]
fn restore_after_purge_returns_already_purged() {
    let root = unique_test_dir("lifecycle-restore-purged");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01LIFECYCLERESTOREPURGED001");
    write_note(
        &vault_root.join("notes").join("purged.md"),
        note_id.as_str(),
        "Purged",
    );

    let config = base_config(&vault_root);
    let mut conn = seeded_connection(&vault_root, &config);
    let _trashed = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");
    age_note_delete_record(
        &conn,
        &note_id,
        "2000-01-01T00:00:00Z",
        "2000-01-02T00:00:00Z",
    );
    purge_expired(
        &vault_root,
        &config,
        &mut conn,
        &PurgeRequest {
            scope: PurgeScope::Expired,
            confirm: true,
        },
        &fixed_clock("2026-04-21T12:00:00Z"),
    )
    .expect("confirmed purge should succeed");

    let err = restore_note(
        &vault_root,
        &config,
        &mut conn,
        &RestoreRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            alternate_path: None,
        },
    )
    .expect_err("restoring a purged item should fail");

    assert!(matches!(
        err,
        AppError::AlreadyPurged { target } if target == note_id.as_str()
    ));
}

#[test]
fn rebuild_warns_about_orphan_and_doctor_reports_it() {
    let root = unique_test_dir("lifecycle-orphan-detection");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("vault root should exist");

    let config = base_config(&vault_root);
    let mut conn = open_memory().expect("open in-memory SQLite");
    let _ = rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("initial rebuild should succeed");
    DeleteRecordRepo::insert_delete_record(
        &conn,
        &zk_domain::sync_state::DeleteRecord {
            item_id: "01LIFECYCLEORPHANDETECT00001".to_string(),
            item_type: ItemType::Note,
            original_path: RelativePath::new("notes/orphan.md"),
            trashed_at: "2026-04-20T10:00:00Z".to_string(),
            retention_until: "2099-01-01T00:00:00Z".to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("orphan delete record should insert");

    let layer = RetentionWarningLayer::default();
    let warnings = Arc::clone(&layer.warnings);
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let reconciliation = reconciliation_from(
        rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
            .expect("rebuild with orphan should succeed"),
    );
    assert_eq!(reconciliation.orphans_warned, 1);

    let captured = warnings
        .lock()
        .expect("warning mutex should not be poisoned")
        .clone();
    assert_eq!(captured.len(), 1);
    assert!(
        captured[0]
            .message
            .contains("orphaned delete record remains after rebuild"),
        "rebuild should emit the orphan warning event"
    );

    let report = doctor_report(&vault_root, &config, &conn).expect("doctor report should succeed");
    let issue = report
        .warnings
        .iter()
        .find(|issue| issue.rule_id == "orphan-delete-record")
        .expect("doctor should report the orphan delete record");
    assert_eq!(
        issue.item_id.as_deref(),
        Some("01LIFECYCLEORPHANDETECT00001")
    );
    assert_eq!(issue.item_type, Some(ItemType::Note));
}

#[test]
fn trashing_note_under_immutable_folder_is_rejected_without_side_effects() {
    let root = unique_test_dir("lifecycle-immutable-trash");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01LIFECYCLEIMMUTABLETRASH001");
    let note_path = RelativePath::new("wiki/meta/blocked.md");
    write_note(
        &vault_root.join(note_path.as_str()),
        note_id.as_str(),
        "Blocked",
    );

    let config = immutable_config(&vault_root, "wiki/meta");
    let mut conn = seeded_connection(&vault_root, &config);

    let err = trash_note(
        &vault_root,
        &config,
        &mut conn,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ByPath(note_path.clone()),
            reason: None,
        },
    )
    .expect_err("immutable note trash should fail");

    assert!(matches!(
        err,
        AppError::ImmutableFolder { path } if path == note_path
    ));
    assert!(vault_root.join(note_path.as_str()).exists());

    let note_row = NoteRepo::get_note(&conn, &note_id)
        .expect("note query should succeed")
        .expect("note row should still exist");
    assert_eq!(note_row.current_path, note_path);
    assert!(!note_row.is_trashed);
    assert!(note_row.trashed_at.is_none());
    assert!(
        DeleteRecordRepo::get_delete_record(&conn, note_id.as_str(), ItemType::Note)
            .expect("delete-record query should succeed")
            .is_none()
    );
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RetentionWarning {
    message: String,
}

#[derive(Clone, Default)]
struct RetentionWarningLayer {
    warnings: Arc<Mutex<Vec<RetentionWarning>>>,
}

impl<S> Layer<S> for RetentionWarningLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN
            || event.metadata().target() != "zk_app::retention"
        {
            return;
        }

        let mut visitor = RetentionWarningVisitor::default();
        event.record(&mut visitor);
        self.warnings
            .lock()
            .expect("warning mutex should not be poisoned")
            .push(RetentionWarning {
                message: visitor.message.unwrap_or_default(),
            });
    }
}

#[derive(Default)]
struct RetentionWarningVisitor {
    message: Option<String>,
}

impl Visit for RetentionWarningVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record_value(field.name(), &format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field.name(), value);
    }
}

impl RetentionWarningVisitor {
    fn record_value(&mut self, field_name: &str, value: &str) {
        if field_name == "message" {
            self.message = Some(value.trim_matches('"').to_string());
        }
    }
}

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn fixed_clock(now: &str) -> FixedClock {
    FixedClock(
        DateTime::parse_from_rfc3339(now)
            .expect("fixed clock timestamp should parse")
            .with_timezone(&Utc),
    )
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

fn immutable_config(vault_root: &Path, immutable_prefix: &str) -> EffectiveConfig {
    let mut config = base_config(vault_root);
    config.folders = BTreeMap::from([(
        immutable_prefix.to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);
    config
}

fn seeded_connection(vault_root: &Path, config: &EffectiveConfig) -> rusqlite::Connection {
    let mut conn = open_memory().expect("open in-memory SQLite");
    rebuild_index(vault_root, config, &mut conn, RebuildOptions::default())
        .expect("seed rebuild should succeed");
    conn
}

fn write_note(path: &Path, note_id: &str, title: &str) {
    write_text(
        path,
        &format!("---\nid: {note_id}\ntitle: {title}\ntags:\n  - tg10\n---\n# {title}\n\nBody\n"),
    );
}

fn move_relative(vault_root: &Path, from: &RelativePath, to: &RelativePath) {
    let destination = vault_root.join(to.as_str());
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("destination parent should exist");
    }
    fs::rename(vault_root.join(from.as_str()), destination).expect("fixture move should succeed");
}

fn fetch_note_delete_record(
    conn: &rusqlite::Connection,
    note_id: &NoteId,
) -> zk_domain::sync_state::DeleteRecord {
    DeleteRecordRepo::get_delete_record(conn, note_id.as_str(), ItemType::Note)
        .expect("delete-record query should succeed")
        .expect("delete record should exist")
}

fn age_note_delete_record(
    conn: &rusqlite::Connection,
    note_id: &NoteId,
    trashed_at: &str,
    retention_until: &str,
) {
    let mut delete_record = fetch_note_delete_record(conn, note_id);
    delete_record.trashed_at = trashed_at.to_string();
    delete_record.retention_until = retention_until.to_string();
    DeleteRecordRepo::insert_delete_record(conn, &delete_record)
        .expect("aged delete-record should persist");
}

fn reconciliation_from(outcome: RebuildOutcome) -> zk_app::ReconciliationStats {
    match outcome {
        RebuildOutcome::ScanOnly { reconciliation, .. }
        | RebuildOutcome::Normalized { reconciliation, .. } => reconciliation,
    }
}
