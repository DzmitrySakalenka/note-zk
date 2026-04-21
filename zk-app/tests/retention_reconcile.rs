use std::{
    fmt, fs,
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};
use zk_app::{
    Clock, RebuildOptions, RebuildOutcome, ReconciliationStats, rebuild_index as app_rebuild_index,
    reconcile_delete_records, trash_note,
};
use zk_domain::{
    ItemKind, TargetSelector, TrashRequest,
    config::EffectiveConfig,
    id::{ContentHash, NoteId, RelativePath},
    sync::ItemType,
    sync_state::DeleteRecord,
};
use zk_index::{
    AssetInput, AssetRepo, DeleteRecordRepo, NoteInput, ensure_schema, open_memory,
    rebuild_index as index_rebuild,
};

mod common;

use common::{unique_test_dir, write_text};

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetentionWarning {
    target: String,
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
            .expect("warning collection mutex should not be poisoned")
            .push(RetentionWarning {
                target: event.metadata().target().to_string(),
                message: visitor.message.unwrap_or_default(),
            });
    }
}

#[derive(Default)]
struct RetentionWarningVisitor {
    message: Option<String>,
}

impl Visit for RetentionWarningVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let value = format!("{value:?}");
        self.record_value(field.name(), &value);
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

#[test]
fn reconcile_delete_records_inserts_note_and_asset_rows_for_first_trash() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_inputs = vec![note_input(
        "01TRASHRECONCILENOTE0000001",
        "99_trash/lost.md",
        true,
        Some("2026-04-20T10:00:00Z"),
    )];
    let asset_inputs = vec![asset_input(
        "99_trash/diagram.png",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        true,
        Some("2026-04-20T11:00:00Z"),
    )];

    index_rebuild(&mut connection, &note_inputs, &asset_inputs).expect("rebuild should succeed");
    let stats = reconcile_delete_records(
        &connection,
        &note_inputs,
        &asset_inputs,
        14,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("reconciliation should succeed");

    assert_eq!(
        stats,
        ReconciliationStats {
            inserted: 2,
            refreshed: 0,
            cleared: 0,
            orphans_warned: 0,
        }
    );

    let note_record = DeleteRecordRepo::get_delete_record(
        &connection,
        note_inputs[0].note_id.as_str(),
        ItemType::Note,
    )
    .expect("note delete-record query should succeed")
    .expect("note delete record should exist");
    assert_eq!(note_record.original_path, note_inputs[0].path);
    assert_eq!(note_record.trashed_at, "2026-04-20T10:00:00Z");
    assert_eq!(note_record.retention_until, "2026-05-04T10:00:00Z");

    let asset_id = asset_id_for_path(&connection, "99_trash/diagram.png");
    let asset_record =
        DeleteRecordRepo::get_delete_record(&connection, asset_id.as_str(), ItemType::Asset)
            .expect("asset delete-record query should succeed")
            .expect("asset delete record should exist");
    assert_eq!(asset_record.original_path, asset_inputs[0].path);
    assert_eq!(asset_record.trashed_at, "2026-04-20T11:00:00Z");
    assert_eq!(asset_record.retention_until, "2026-05-04T11:00:00Z");
}

#[test]
fn reconcile_delete_records_skips_refresh_when_retention_is_unchanged() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_inputs = vec![note_input(
        "01TRASHRECONCILENOTE0000002",
        "99_trash/unchanged.md",
        true,
        Some("2026-04-20T10:00:00Z"),
    )];
    let asset_inputs = vec![asset_input(
        "99_trash/unchanged.png",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        true,
        Some("2026-04-20T11:00:00Z"),
    )];

    index_rebuild(&mut connection, &note_inputs, &asset_inputs).expect("rebuild should succeed");
    reconcile_delete_records(
        &connection,
        &note_inputs,
        &asset_inputs,
        14,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("initial reconciliation should succeed");

    let stats = reconcile_delete_records(
        &connection,
        &note_inputs,
        &asset_inputs,
        14,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("repeat reconciliation should succeed");

    assert_eq!(
        stats,
        ReconciliationStats {
            inserted: 0,
            refreshed: 0,
            cleared: 0,
            orphans_warned: 0,
        }
    );
}

#[test]
fn reconcile_delete_records_refreshes_rows_when_retention_days_change() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_inputs = vec![note_input(
        "01TRASHRECONCILENOTE0000003",
        "99_trash/retention.md",
        true,
        Some("2026-04-20T10:00:00Z"),
    )];

    index_rebuild(&mut connection, &note_inputs, &[]).expect("rebuild should succeed");
    reconcile_delete_records(
        &connection,
        &note_inputs,
        &[],
        14,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("initial reconciliation should succeed");

    let stats = reconcile_delete_records(
        &connection,
        &note_inputs,
        &[],
        30,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("retention refresh should succeed");

    assert_eq!(
        stats,
        ReconciliationStats {
            inserted: 0,
            refreshed: 1,
            cleared: 0,
            orphans_warned: 0,
        }
    );
    assert_eq!(
        DeleteRecordRepo::get_delete_record(
            &connection,
            note_inputs[0].note_id.as_str(),
            ItemType::Note
        )
        .expect("note delete-record query should succeed")
        .expect("note delete record should exist")
        .retention_until,
        "2026-05-20T10:00:00Z"
    );
}

#[test]
fn reconcile_delete_records_clears_rows_when_items_are_restored() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let trashed_note_inputs = vec![note_input(
        "01TRASHRECONCILENOTE0000004",
        "99_trash/restored.md",
        true,
        Some("2026-04-20T10:00:00Z"),
    )];
    let trashed_asset_inputs = vec![asset_input(
        "99_trash/restored.png",
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        true,
        Some("2026-04-20T11:00:00Z"),
    )];

    index_rebuild(&mut connection, &trashed_note_inputs, &trashed_asset_inputs)
        .expect("initial rebuild should succeed");
    reconcile_delete_records(
        &connection,
        &trashed_note_inputs,
        &trashed_asset_inputs,
        14,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("initial reconciliation should succeed");

    let restored_note_inputs = vec![note_input(
        "01TRASHRECONCILENOTE0000004",
        "notes/restored.md",
        false,
        None,
    )];
    let restored_asset_inputs = vec![asset_input(
        "assets/restored.png",
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        false,
        None,
    )];

    index_rebuild(
        &mut connection,
        &restored_note_inputs,
        &restored_asset_inputs,
    )
    .expect("restore rebuild should succeed");
    let stats = reconcile_delete_records(
        &connection,
        &restored_note_inputs,
        &restored_asset_inputs,
        14,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("restore reconciliation should succeed");

    assert_eq!(
        stats,
        ReconciliationStats {
            inserted: 0,
            refreshed: 0,
            cleared: 2,
            orphans_warned: 0,
        }
    );
    assert!(
        DeleteRecordRepo::get_delete_record(
            &connection,
            restored_note_inputs[0].note_id.as_str(),
            ItemType::Note
        )
        .expect("note delete-record query should succeed")
        .is_none()
    );

    let restored_asset_id = asset_id_for_path(&connection, "assets/restored.png");
    assert!(
        DeleteRecordRepo::get_delete_record(
            &connection,
            restored_asset_id.as_str(),
            ItemType::Asset
        )
        .expect("asset delete-record query should succeed")
        .is_none()
    );
}

#[test]
fn reconcile_delete_records_warns_about_orphans_without_deleting_them() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let orphan_record = DeleteRecord {
        item_id: "01TRASHRECONCILEORPHAN000001".to_string(),
        item_type: ItemType::Note,
        original_path: RelativePath::new("99_trash/orphan.md"),
        trashed_at: "2026-04-20T10:00:00Z".to_string(),
        retention_until: "2026-05-04T10:00:00Z".to_string(),
        deleted_at: None,
        delete_revision: None,
    };
    DeleteRecordRepo::insert_delete_record(&connection, &orphan_record)
        .expect("orphan delete-record insert should succeed");

    let layer = RetentionWarningLayer::default();
    let warnings = Arc::clone(&layer.warnings);
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let stats = reconcile_delete_records(
        &connection,
        &[],
        &[],
        14,
        &fixed_clock("2026-04-20T12:00:00Z"),
    )
    .expect("orphan reconciliation should succeed");

    assert_eq!(
        stats,
        ReconciliationStats {
            inserted: 0,
            refreshed: 0,
            cleared: 0,
            orphans_warned: 1,
        }
    );
    let warnings = warnings
        .lock()
        .expect("warning collection mutex should not be poisoned")
        .clone();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].target, "zk_app::retention");
    assert!(
        warnings[0].message.contains("orphaned delete record"),
        "warning should describe the orphaned logical delete record"
    );
    assert_eq!(
        DeleteRecordRepo::get_delete_record(
            &connection,
            &orphan_record.item_id,
            orphan_record.item_type
        )
        .expect("orphan delete-record query should succeed"),
        Some(orphan_record)
    );
}

#[test]
fn rebuild_preserves_explicit_trash_original_path() {
    let root = unique_test_dir("retention-preserve-original-path");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RETENTIONORIGINALPATH000001");
    let live_path = RelativePath::new("notes/preserve.md");
    write_note(
        &vault_root.join(live_path.as_str()),
        note_id.as_str(),
        "Preserve Original Path",
    );

    let config = base_config(&vault_root);
    let mut connection = seeded_connection(&vault_root, &config);
    trash_note(
        &vault_root,
        &config,
        &mut connection,
        &TrashRequest {
            kind: ItemKind::Note,
            target: TargetSelector::ById(note_id.as_str().to_string()),
            reason: None,
        },
    )
    .expect("trash note should succeed");

    let initial_record =
        DeleteRecordRepo::get_delete_record(&connection, note_id.as_str(), ItemType::Note)
            .expect("delete-record query should succeed")
            .expect("delete record should exist after trash");
    assert_eq!(initial_record.original_path, live_path);

    let outcome = app_rebuild_index(
        &vault_root,
        &config,
        &mut connection,
        RebuildOptions::default(),
    )
    .expect("rebuild should succeed");

    assert_eq!(reconciliation(&outcome).inserted, 0);
    assert_eq!(reconciliation(&outcome).refreshed, 0);
    assert_eq!(reconciliation(&outcome).cleared, 0);
    assert_eq!(reconciliation(&outcome).orphans_warned, 0);

    let refreshed_record =
        DeleteRecordRepo::get_delete_record(&connection, note_id.as_str(), ItemType::Note)
            .expect("delete-record query should succeed")
            .expect("delete record should still exist after rebuild");
    assert_eq!(refreshed_record.original_path, live_path);
}

#[test]
fn rebuild_uses_scanned_trash_path_for_manual_move_without_existing_delete_record() {
    let root = unique_test_dir("retention-manual-trash-path");
    let vault_root = root.join("vault");
    let note_id = NoteId::new("01RETENTIONMANUALTRASHPATH01");
    let live_path = RelativePath::new("notes/manual.md");
    let trash_path = RelativePath::new("99_trash/manual.md");
    write_note(
        &vault_root.join(live_path.as_str()),
        note_id.as_str(),
        "Manual Trash Path",
    );

    let config = base_config(&vault_root);
    let mut connection = seeded_connection(&vault_root, &config);
    move_relative(&vault_root, &live_path, &trash_path);

    let outcome = app_rebuild_index(
        &vault_root,
        &config,
        &mut connection,
        RebuildOptions::default(),
    )
    .expect("rebuild should succeed");

    assert_eq!(reconciliation(&outcome).inserted, 1);
    assert_eq!(reconciliation(&outcome).refreshed, 0);
    assert_eq!(reconciliation(&outcome).cleared, 0);
    assert_eq!(reconciliation(&outcome).orphans_warned, 0);

    let delete_record =
        DeleteRecordRepo::get_delete_record(&connection, note_id.as_str(), ItemType::Note)
            .expect("delete-record query should succeed")
            .expect("delete record should be created from scan");
    assert_eq!(delete_record.original_path, trash_path);
}

fn fixed_clock(value: &str) -> FixedClock {
    FixedClock(
        DateTime::parse_from_rfc3339(value)
            .expect("fixed clock timestamp should parse")
            .with_timezone(&Utc),
    )
}

fn note_input(note_id: &str, path: &str, is_trashed: bool, trashed_at: Option<&str>) -> NoteInput {
    NoteInput {
        note_id: NoteId::new(note_id),
        path: RelativePath::new(path),
        title: "Retention Test".to_string(),
        content_hash: ContentHash::new(
            "1111111111111111111111111111111111111111111111111111111111111111",
        ),
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"Retention Test"}}"#),
        created_at: "2026-04-20T08:00:00Z".to_string(),
        updated_at: "2026-04-20T09:00:00Z".to_string(),
        tags: Vec::new(),
        aliases: Vec::new(),
        note_links: Vec::new(),
        external_links: Vec::new(),
        asset_references: Vec::new(),
        is_trashed,
        trashed_at: trashed_at.map(ToOwned::to_owned),
    }
}

fn asset_input(
    path: &str,
    content_hash: &str,
    is_trashed: bool,
    trashed_at: Option<&str>,
) -> AssetInput {
    AssetInput {
        path: RelativePath::new(path),
        content_hash: ContentHash::new(content_hash),
        mime_type: Some("image/png".to_string()),
        size_bytes: 1024,
        is_trashed,
        trashed_at: trashed_at.map(ToOwned::to_owned),
    }
}

fn asset_id_for_path(conn: &rusqlite::Connection, path: &str) -> zk_domain::id::AssetId {
    AssetRepo::get_asset_by_path(conn, &RelativePath::new(path))
        .expect("asset lookup should succeed")
        .expect("asset row should exist after rebuild")
        .asset_id
}

fn reconciliation(outcome: &RebuildOutcome) -> &ReconciliationStats {
    match outcome {
        RebuildOutcome::ScanOnly { reconciliation, .. }
        | RebuildOutcome::Normalized { reconciliation, .. } => reconciliation,
    }
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

fn seeded_connection(vault_root: &Path, config: &EffectiveConfig) -> rusqlite::Connection {
    let mut connection = open_memory().expect("in-memory database should open");
    app_rebuild_index(
        vault_root,
        config,
        &mut connection,
        RebuildOptions::default(),
    )
    .expect("seed rebuild should succeed");
    connection
}

fn write_note(path: &Path, note_id: &str, title: &str) {
    write_text(
        path,
        &format!("---\nid: {note_id}\ntitle: {title}\n---\n# {title}\n"),
    );
}

fn move_relative(vault_root: &Path, from: &RelativePath, to: &RelativePath) {
    let destination = vault_root.join(to.as_str());
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("destination parent should exist");
    }
    fs::rename(vault_root.join(from.as_str()), destination)
        .expect("fixture move into trash should succeed");
}
