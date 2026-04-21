use std::fs;

mod common;

use common::unique_test_dir;
use zk_app::status_summary;
use zk_domain::{
    asset::AssetRecord,
    config::EffectiveConfig,
    id::{AssetId, ContentHash, NoteId, RelativePath, ServerRevision},
    note::NoteFrontmatter,
    sync::ItemType,
    sync_state::DeleteRecord,
};
use zk_index::{AssetRepo, DeleteRecordRepo, NoteRepo, ensure_schema, open_memory};

fn sample_frontmatter(note_id: &str, title: &str) -> String {
    serde_json::to_string(&NoteFrontmatter {
        id: NoteId::new(note_id),
        title: title.to_string(),
        tags: vec!["tg10".to_string()],
        note_type: None,
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: std::collections::BTreeMap::default(),
    })
    .expect("frontmatter JSON should serialize")
}

fn upsert_note(
    conn: &rusqlite::Connection,
    note_id: &str,
    path: &str,
    trashed_at: Option<&str>,
    is_trashed: bool,
) {
    NoteRepo::upsert_note(
        conn,
        &NoteId::new(note_id),
        &RelativePath::new(path),
        "Seeded Note",
        &ContentHash::new(&format!("hash-{note_id}")),
        &sample_frontmatter(note_id, "Seeded Note"),
        "2026-04-20T10:00:00Z",
        "2026-04-20T10:00:00Z",
        trashed_at,
        is_trashed,
    )
    .expect("seeded note should upsert");
}

fn upsert_asset(
    conn: &rusqlite::Connection,
    asset_id: &str,
    path: &str,
    trashed_at: Option<&str>,
    is_trashed: bool,
) {
    AssetRepo::upsert_asset(
        conn,
        &AssetRecord {
            asset_id: AssetId::new(asset_id),
            current_path: RelativePath::new(path),
            content_hash: ContentHash::new(&format!("hash-{asset_id}")),
            mime_type: Some("image/png".to_string()),
            size_bytes: 128,
            created_at: "2026-04-20T10:00:00Z".to_string(),
            updated_at: "2026-04-20T10:00:00Z".to_string(),
            trashed_at: trashed_at.map(str::to_string),
            deleted_at: None,
            is_trashed,
            is_deleted: false,
            server_revision: ServerRevision::new(0),
        },
    )
    .expect("seeded asset should upsert");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the mixed lifecycle counter scenario is clearer as one seeded integration test"
)]
fn status_summary_reports_trash_and_purge_counters_from_index() {
    let root = unique_test_dir("status-trash");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("vault root should exist");

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };
    let conn = open_memory().expect("open in-memory SQLite");
    ensure_schema(&conn).expect("schema should exist");

    upsert_note(
        &conn,
        "01TG10STATUSNOTE000000000001",
        "notes/live.md",
        None,
        false,
    );
    upsert_note(
        &conn,
        "01TG10STATUSNOTE000000000002",
        "99_trash/2026-04-20/trashed.md",
        Some("2026-04-20T10:00:00Z"),
        true,
    );
    upsert_note(
        &conn,
        "01TG10STATUSNOTE000000000003",
        "99_trash/2026-04-01/purged.md",
        Some("2026-04-01T10:00:00Z"),
        true,
    );
    NoteRepo::mark_deleted(
        &conn,
        &NoteId::new("01TG10STATUSNOTE000000000003"),
        "2026-04-21T10:00:00Z",
    )
    .expect("purged note should be marked deleted");

    upsert_asset(
        &conn,
        "01tg10statusasset000000000001",
        "assets/live.png",
        None,
        false,
    );
    upsert_asset(
        &conn,
        "01tg10statusasset000000000002",
        "99_trash/2026-04-20/trashed.png",
        Some("2026-04-20T10:00:00Z"),
        true,
    );
    upsert_asset(
        &conn,
        "01tg10statusasset000000000003",
        "99_trash/2026-04-01/purged.png",
        Some("2026-04-01T10:00:00Z"),
        true,
    );
    AssetRepo::mark_deleted(
        &conn,
        &AssetId::new("01tg10statusasset000000000003"),
        "2026-04-21T10:00:00Z",
    )
    .expect("purged asset should be marked deleted");

    DeleteRecordRepo::insert_delete_record(
        &conn,
        &DeleteRecord {
            item_id: "01TG10STATUSNOTE000000000002".to_string(),
            item_type: ItemType::Note,
            original_path: RelativePath::new("notes/trashed.md"),
            trashed_at: "2026-04-20T10:00:00Z".to_string(),
            retention_until: "2099-01-01T00:00:00Z".to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("future delete record should insert");
    DeleteRecordRepo::insert_delete_record(
        &conn,
        &DeleteRecord {
            item_id: "01tg10statusasset000000000002".to_string(),
            item_type: ItemType::Asset,
            original_path: RelativePath::new("assets/trashed.png"),
            trashed_at: "2026-04-20T10:00:00Z".to_string(),
            retention_until: "2026-04-01T00:00:00Z".to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("expired delete record should insert");

    let summary =
        status_summary(&vault_root, &config, &conn).expect("status summary should succeed");

    assert_eq!(summary.total_notes, 3);
    assert_eq!(summary.total_assets, 3);
    assert_eq!(summary.notes_with_errors, 0);
    assert_eq!(summary.notes_with_warnings, 0);
    assert_eq!(summary.unresolved_link_count, 0);
    assert_eq!(summary.duplicate_id_count, 0);
    assert_eq!(summary.trashed_notes, 1);
    assert_eq!(summary.trashed_assets, 1);
    assert_eq!(summary.pending_purge, 1);
    assert_eq!(summary.purged_notes, 1);
    assert_eq!(summary.purged_assets, 1);
}
