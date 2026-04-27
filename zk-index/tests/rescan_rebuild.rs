// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use rusqlite::Connection;
use std::{thread, time::Duration};
use zk_domain::{
    DeleteRecordTarget,
    asset::AssetRecord,
    derived::{AssetReference, ExternalLink, ResolvedNoteLink},
    id::{AssetId, ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    sync::{PendingOperation, SyncStatus},
    sync_state::{AssetSyncState, DeleteRecord, DeviceRecord, NoteSyncState},
};
use zk_index::{
    AssetInput, AssetRepo, DeleteRecordRepo, DeviceRepo, LinkRepo, NoteInput, NoteRepo,
    RebuildStats, SyncStateRepo, ensure_schema, open_memory, rebuild_index,
};

fn alpha_note_id() -> NoteId {
    NoteId::new("01HZ3NOTE000000000000000A")
}

fn beta_note_id() -> NoteId {
    NoteId::new("01HZ3NOTE000000000000000B")
}

fn gamma_note_id() -> NoteId {
    NoteId::new("01HZ3NOTE000000000000000C")
}

fn existing_asset_id() -> AssetId {
    AssetId::new("01HZ3ASSET00000000000000A")
}

fn second_existing_asset_id() -> AssetId {
    AssetId::new("01HZ3ASSET00000000000000B")
}

fn sample_note_inputs() -> Vec<NoteInput> {
    vec![
        NoteInput {
            note_id: alpha_note_id(),
            path: RelativePath::new("notes/alpha.md"),
            title: "Alpha".to_string(),
            content_hash: ContentHash::new(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            frontmatter_json: r#"{"id":"01HZ3NOTE000000000000000A","title":"Alpha"}"#.to_string(),
            created_at: "2026-04-02T10:00:00Z".to_string(),
            updated_at: "2026-04-02T12:00:00Z".to_string(),
            tags: vec!["zettel".to_string(), "alpha".to_string()],
            aliases: vec!["A".to_string()],
            note_links: vec![
                ResolvedNoteLink {
                    source_note_id: alpha_note_id(),
                    raw_target: "Beta".to_string(),
                    target_note_id: Some(beta_note_id()),
                    link_text: Some("See Beta".to_string()),
                    is_ambiguous: false,
                },
                ResolvedNoteLink {
                    source_note_id: alpha_note_id(),
                    raw_target: "Missing".to_string(),
                    target_note_id: None,
                    link_text: None,
                    is_ambiguous: false,
                },
            ],
            external_links: vec![ExternalLink {
                source_note_id: alpha_note_id(),
                url: "https://example.com".to_string(),
                link_text: Some("Example".to_string()),
            }],
            asset_references: vec![AssetReference {
                source_note_id: alpha_note_id(),
                raw_path: "assets/diagram.png".to_string(),
                target_asset_id: None,
            }],
            is_trashed: false,
            trashed_at: None,
        },
        NoteInput {
            note_id: beta_note_id(),
            path: RelativePath::new("notes/beta.md"),
            title: "Beta".to_string(),
            content_hash: ContentHash::new(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            frontmatter_json: r#"{"id":"01HZ3NOTE000000000000000B","title":"Beta"}"#.to_string(),
            created_at: "2026-04-03T10:00:00Z".to_string(),
            updated_at: "2026-04-03T12:00:00Z".to_string(),
            tags: vec!["zettel".to_string()],
            aliases: vec!["Bee".to_string()],
            note_links: vec![],
            external_links: vec![],
            asset_references: vec![],
            is_trashed: false,
            trashed_at: None,
        },
        NoteInput {
            note_id: gamma_note_id(),
            path: RelativePath::new("notes/gamma.md"),
            title: "Gamma".to_string(),
            content_hash: ContentHash::new(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            ),
            frontmatter_json: r#"{"id":"01HZ3NOTE000000000000000C","title":"Gamma"}"#.to_string(),
            created_at: "2026-04-04T10:00:00Z".to_string(),
            updated_at: "2026-04-04T12:00:00Z".to_string(),
            tags: vec!["reference".to_string()],
            aliases: vec![],
            note_links: vec![],
            external_links: vec![],
            asset_references: vec![],
            is_trashed: false,
            trashed_at: None,
        },
    ]
}

fn sample_asset_inputs() -> Vec<AssetInput> {
    vec![
        AssetInput {
            path: RelativePath::new("assets/diagram.png"),
            content_hash: ContentHash::new(
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            ),
            mime_type: Some("image/png".to_string()),
            size_bytes: 4096,
            is_trashed: false,
            trashed_at: None,
        },
        AssetInput {
            path: RelativePath::new("assets/report.pdf"),
            content_hash: ContentHash::new(
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            ),
            mime_type: Some("application/pdf".to_string()),
            size_bytes: 8192,
            is_trashed: false,
            trashed_at: None,
        },
    ]
}

fn asset_record(
    asset_id: AssetId,
    path: &str,
    hash: &str,
    created_at: &str,
    updated_at: &str,
) -> AssetRecord {
    AssetRecord {
        asset_id,
        current_path: RelativePath::new(path),
        content_hash: ContentHash::new(hash),
        mime_type: Some("image/png".to_string()),
        size_bytes: 2048,
        created_at: created_at.to_string(),
        updated_at: updated_at.to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        server_revision: ServerRevision::new(0),
    }
}

fn note_sync_state(note_id: NoteId) -> NoteSyncState {
    NoteSyncState {
        item_id: note_id,
        device_id: DeviceId::new("device-alpha"),
        last_local_hash: Some(ContentHash::from_bytes(b"note local hash")),
        last_synced_hash: Some(ContentHash::from_bytes(b"note synced hash")),
        last_synced_path: Some(RelativePath::new("notes/existing.md")),
        last_known_server_revision: Some(ServerRevision::new(5)),
        sync_status: SyncStatus::Synced,
        last_sync_attempt_at: Some("2026-04-10T12:00:00Z".to_string()),
        last_sync_error: None,
        pending_operation: None,
    }
}

fn asset_sync_state(asset_id: AssetId) -> AssetSyncState {
    AssetSyncState {
        item_id: asset_id,
        device_id: DeviceId::new("device-beta"),
        last_local_hash: Some(ContentHash::from_bytes(b"asset local hash")),
        last_synced_hash: Some(ContentHash::from_bytes(b"asset synced hash")),
        last_synced_path: Some(RelativePath::new("assets/existing.png")),
        last_known_server_revision: Some(ServerRevision::new(7)),
        sync_status: SyncStatus::PendingUpload,
        last_sync_attempt_at: None,
        last_sync_error: None,
        pending_operation: Some(PendingOperation::Create),
    }
}

fn device_record() -> DeviceRecord {
    DeviceRecord {
        device_id: DeviceId::new("device-alpha"),
        device_name: "MacBook Pro".to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("0.1.0".to_string()),
        capabilities: vec!["sync".to_string()],
        created_at: "2026-04-01T08:00:00Z".to_string(),
        last_seen_at: "2026-04-10T12:00:00Z".to_string(),
        last_sync_cursor: Some(42),
    }
}

fn delete_record() -> DeleteRecord {
    DeleteRecord {
        target: DeleteRecordTarget::Note(NoteId::new("01HZ3NOTE000000000000000Z")),
        original_path: RelativePath::new("notes/deleted.md"),
        trashed_at: "2026-04-05T08:00:00Z".to_string(),
        retention_until: "2026-05-05T08:00:00Z".to_string(),
        deleted_at: None,
        delete_revision: None,
    }
}

fn table_count(conn: &Connection, table_name: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table_name}"), [], |row| {
        row.get(0)
    })
    .expect("count query should succeed")
}

fn note_lifecycle_columns(
    conn: &Connection,
    note_id: &NoteId,
) -> (Option<String>, i64, Option<String>, i64) {
    conn.query_row(
        "SELECT deleted_at, is_deleted, trashed_at, is_trashed FROM notes_index WHERE note_id = ?1",
        [note_id.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .expect("note lifecycle columns should query successfully")
}

fn asset_lifecycle_columns(
    conn: &Connection,
    asset_id: &AssetId,
) -> (Option<String>, i64, Option<String>, i64) {
    conn.query_row(
        "SELECT deleted_at, is_deleted, trashed_at, is_trashed FROM assets_index WHERE asset_id = ?1",
        [asset_id.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .expect("asset lifecycle columns should query successfully")
}

#[allow(
    clippy::too_many_arguments,
    reason = "named scalar expectations keep rebuild stat assertions readable in tests"
)]
fn assert_stats(
    stats: &RebuildStats,
    notes_upserted: usize,
    assets_upserted: usize,
    note_links_inserted: usize,
    external_links_inserted: usize,
    asset_references_inserted: usize,
) {
    assert_eq!(stats.notes_upserted, notes_upserted);
    assert_eq!(stats.assets_upserted, assets_upserted);
    assert_eq!(stats.note_links_inserted, note_links_inserted);
    assert_eq!(stats.external_links_inserted, external_links_inserted);
    assert_eq!(stats.asset_references_inserted, asset_references_inserted);
}

#[test]
fn rebuild_populates_notes_links_assets_and_references() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let notes = sample_note_inputs();
    let assets = sample_asset_inputs();

    let stats = rebuild_index(&mut connection, &notes, &assets).expect("rebuild should succeed");

    assert_stats(&stats, 3, 2, 2, 1, 1);
    assert_eq!(table_count(&connection, "note_tags"), 4);
    assert_eq!(table_count(&connection, "note_aliases"), 2);
    assert_eq!(table_count(&connection, "note_links"), 2);
    assert_eq!(table_count(&connection, "external_links"), 1);
    assert_eq!(table_count(&connection, "asset_references"), 1);

    let alpha = NoteRepo::get_note(&connection, &alpha_note_id())
        .expect("note query should succeed")
        .expect("alpha note should exist");
    assert_eq!(alpha.current_path.as_str(), "notes/alpha.md");
    assert_eq!(
        NoteRepo::get_tags(&connection, &alpha_note_id()).expect("tag query should succeed"),
        vec!["alpha".to_string(), "zettel".to_string()]
    );
    assert_eq!(
        NoteRepo::get_aliases(&connection, &alpha_note_id()).expect("alias query should succeed"),
        vec!["A".to_string()]
    );

    let outbound_links =
        LinkRepo::get_note_links(&connection, &alpha_note_id()).expect("link query should succeed");
    assert_eq!(outbound_links.len(), 2);
    assert_eq!(outbound_links[0].target_note_id, Some(beta_note_id()));
    assert_eq!(outbound_links[1].target_note_id, None);

    let backlinks = LinkRepo::get_backlinks(&connection, &beta_note_id())
        .expect("backlink query should succeed");
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].source_note_id, alpha_note_id());

    let external_links = LinkRepo::get_external_links(&connection, &alpha_note_id())
        .expect("external-link query should succeed");
    assert_eq!(external_links.len(), 1);
    assert_eq!(external_links[0].url, "https://example.com");

    let asset_refs = AssetRepo::get_asset_references(&connection, &alpha_note_id())
        .expect("asset-reference query should succeed");
    assert_eq!(asset_refs.len(), 1);
    assert_eq!(asset_refs[0].raw_path, "assets/diagram.png");
    assert_eq!(asset_refs[0].target_asset_id, None);

    let stored_assets = AssetRepo::list_assets(&connection).expect("asset list should succeed");
    assert_eq!(stored_assets.len(), 2);
    assert_eq!(stored_assets[0].current_path.as_str(), "assets/diagram.png");
    assert_eq!(stored_assets[1].current_path.as_str(), "assets/report.pdf");
}

#[test]
fn rebuild_persists_trash_fields_for_notes_and_assets() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let notes = vec![NoteInput {
        note_id: alpha_note_id(),
        path: RelativePath::new("99_trash/alpha.md"),
        title: "Alpha".to_string(),
        content_hash: ContentHash::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        frontmatter_json: r#"{"id":"01HZ3NOTE000000000000000A","title":"Alpha"}"#.to_string(),
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-02T12:00:00Z".to_string(),
        tags: vec!["zettel".to_string()],
        aliases: Vec::new(),
        note_links: Vec::new(),
        external_links: Vec::new(),
        asset_references: Vec::new(),
        is_trashed: true,
        trashed_at: Some("2026-04-20T08:00:00Z".to_string()),
    }];
    let assets = vec![AssetInput {
        path: RelativePath::new("99_trash/diagram.png"),
        content_hash: ContentHash::new(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
        mime_type: Some("image/png".to_string()),
        size_bytes: 4096,
        is_trashed: true,
        trashed_at: Some("2026-04-20T09:00:00Z".to_string()),
    }];

    rebuild_index(&mut connection, &notes, &assets).expect("rebuild should succeed");

    let note_columns: (i64, Option<String>) = connection
        .query_row(
            "SELECT is_trashed, trashed_at FROM notes_index WHERE note_id = ?1",
            [alpha_note_id().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("note lifecycle columns should query successfully");
    let asset_columns: (i64, Option<String>) = connection
        .query_row(
            "SELECT is_trashed, trashed_at FROM assets_index WHERE current_path = ?1",
            [RelativePath::new("99_trash/diagram.png").as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("asset lifecycle columns should query successfully");

    assert_eq!(note_columns, (1, Some("2026-04-20T08:00:00Z".to_string())));
    assert_eq!(asset_columns, (1, Some("2026-04-20T09:00:00Z".to_string())));
}

#[test]
fn rebuild_clears_previously_trashed_rows_when_inputs_return_live() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let trashed_notes = vec![NoteInput {
        note_id: alpha_note_id(),
        path: RelativePath::new("99_trash/alpha.md"),
        title: "Alpha".to_string(),
        content_hash: ContentHash::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        frontmatter_json: r#"{"id":"01HZ3NOTE000000000000000A","title":"Alpha"}"#.to_string(),
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-02T12:00:00Z".to_string(),
        tags: vec!["zettel".to_string()],
        aliases: Vec::new(),
        note_links: Vec::new(),
        external_links: Vec::new(),
        asset_references: Vec::new(),
        is_trashed: true,
        trashed_at: Some("2026-04-20T08:00:00Z".to_string()),
    }];
    let live_notes = vec![NoteInput {
        note_id: alpha_note_id(),
        path: RelativePath::new("notes/alpha.md"),
        title: "Alpha".to_string(),
        content_hash: ContentHash::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        frontmatter_json: r#"{"id":"01HZ3NOTE000000000000000A","title":"Alpha"}"#.to_string(),
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-02T12:00:00Z".to_string(),
        tags: vec!["zettel".to_string()],
        aliases: Vec::new(),
        note_links: Vec::new(),
        external_links: Vec::new(),
        asset_references: Vec::new(),
        is_trashed: false,
        trashed_at: None,
    }];
    let trashed_assets = vec![AssetInput {
        path: RelativePath::new("99_trash/diagram.png"),
        content_hash: ContentHash::new(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
        mime_type: Some("image/png".to_string()),
        size_bytes: 4096,
        is_trashed: true,
        trashed_at: Some("2026-04-20T09:00:00Z".to_string()),
    }];
    let live_assets = vec![AssetInput {
        path: RelativePath::new("assets/diagram.png"),
        content_hash: ContentHash::new(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
        mime_type: Some("image/png".to_string()),
        size_bytes: 4096,
        is_trashed: false,
        trashed_at: None,
    }];

    rebuild_index(&mut connection, &trashed_notes, &trashed_assets)
        .expect("initial rebuild should succeed");
    rebuild_index(&mut connection, &live_notes, &live_assets).expect("live rebuild should succeed");

    let note = NoteRepo::get_note(&connection, &alpha_note_id())
        .expect("note query should succeed")
        .expect("note should exist");
    let asset = AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/diagram.png"))
        .expect("asset query should succeed")
        .expect("asset should exist");

    assert!(!note.is_trashed);
    assert_eq!(note.trashed_at, None);
    assert!(!asset.is_trashed);
    assert_eq!(asset.trashed_at, None);
}

#[test]
fn rebuild_preserves_delete_columns_while_updating_trash_columns() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let deleted_note_at = "2026-04-18T11:00:00Z";
    let deleted_asset_at = "2026-04-19T12:00:00Z";

    NoteRepo::upsert_note(
        &connection,
        &alpha_note_id(),
        &RelativePath::new("notes/alpha.md"),
        "Alpha",
        &ContentHash::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        r#"{"id":"01HZ3NOTE000000000000000A","title":"Alpha"}"#,
        "2026-04-02T10:00:00Z",
        "2026-04-02T12:00:00Z",
        None,
        false,
    )
    .expect("seed note should upsert successfully");
    connection
        .execute(
            "UPDATE notes_index SET deleted_at = ?1, is_deleted = ?2 WHERE note_id = ?3",
            rusqlite::params![deleted_note_at, 1_i64, alpha_note_id().as_str()],
        )
        .expect("seed note delete columns should update successfully");

    AssetRepo::upsert_asset(
        &connection,
        &AssetRecord {
            asset_id: existing_asset_id(),
            current_path: RelativePath::new("assets/diagram.png"),
            content_hash: ContentHash::new(
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            ),
            mime_type: Some("image/png".to_string()),
            size_bytes: 4096,
            created_at: "2026-04-02T10:00:00Z".to_string(),
            updated_at: "2026-04-02T12:00:00Z".to_string(),
            trashed_at: None,
            deleted_at: Some(deleted_asset_at.to_string()),
            is_trashed: false,
            is_deleted: true,
            server_revision: ServerRevision::new(0),
        },
    )
    .expect("seed asset should upsert successfully");

    let notes = vec![NoteInput {
        note_id: alpha_note_id(),
        path: RelativePath::new("99_trash/alpha.md"),
        title: "Alpha".to_string(),
        content_hash: ContentHash::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        frontmatter_json: r#"{"id":"01HZ3NOTE000000000000000A","title":"Alpha"}"#.to_string(),
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-20T08:00:00Z".to_string(),
        tags: Vec::new(),
        aliases: Vec::new(),
        note_links: Vec::new(),
        external_links: Vec::new(),
        asset_references: Vec::new(),
        is_trashed: true,
        trashed_at: Some("2026-04-20T08:00:00Z".to_string()),
    }];
    let assets = vec![AssetInput {
        path: RelativePath::new("99_trash/diagram.png"),
        content_hash: ContentHash::new(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
        mime_type: Some("image/png".to_string()),
        size_bytes: 4096,
        is_trashed: true,
        trashed_at: Some("2026-04-20T09:00:00Z".to_string()),
    }];

    rebuild_index(&mut connection, &notes, &assets).expect("rebuild should succeed");

    let note_columns = note_lifecycle_columns(&connection, &alpha_note_id());
    let asset_columns = asset_lifecycle_columns(&connection, &existing_asset_id());

    assert_eq!(
        note_columns,
        (
            Some(deleted_note_at.to_string()),
            1,
            Some("2026-04-20T08:00:00Z".to_string()),
            1,
        )
    );
    assert_eq!(
        asset_columns,
        (
            Some(deleted_asset_at.to_string()),
            1,
            Some("2026-04-20T09:00:00Z".to_string()),
            1,
        )
    );
}

#[test]
fn rebuild_is_idempotent_for_identical_input() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let notes = sample_note_inputs();
    let assets = sample_asset_inputs();

    let first_stats =
        rebuild_index(&mut connection, &notes, &assets).expect("first rebuild should succeed");
    let first_asset_id =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/diagram.png"))
            .expect("asset query should succeed")
            .expect("diagram asset should exist")
            .asset_id;
    let first_counts = (
        table_count(&connection, "notes_index"),
        table_count(&connection, "assets_index"),
        table_count(&connection, "note_tags"),
        table_count(&connection, "note_aliases"),
        table_count(&connection, "note_links"),
        table_count(&connection, "external_links"),
        table_count(&connection, "asset_references"),
    );

    let second_stats =
        rebuild_index(&mut connection, &notes, &assets).expect("second rebuild should succeed");
    let second_asset_id =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/diagram.png"))
            .expect("asset query should succeed")
            .expect("diagram asset should exist")
            .asset_id;
    let second_counts = (
        table_count(&connection, "notes_index"),
        table_count(&connection, "assets_index"),
        table_count(&connection, "note_tags"),
        table_count(&connection, "note_aliases"),
        table_count(&connection, "note_links"),
        table_count(&connection, "external_links"),
        table_count(&connection, "asset_references"),
    );

    assert_eq!(first_stats, second_stats);
    assert_eq!(first_counts, second_counts);
    assert_eq!(first_asset_id, second_asset_id);
}

#[test]
fn asset_path_match_retains_existing_asset_id() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let existing = asset_record(
        existing_asset_id(),
        "assets/diagram.png",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "2026-04-01T08:00:00Z",
        "2026-04-02T09:00:00Z",
    );
    AssetRepo::upsert_asset(&connection, &existing).expect("existing asset upsert should succeed");

    rebuild_index(
        &mut connection,
        &[],
        &[AssetInput {
            path: RelativePath::new("assets/diagram.png"),
            content_hash: ContentHash::new(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            ),
            mime_type: Some("image/png".to_string()),
            size_bytes: 7777,
            is_trashed: false,
            trashed_at: None,
        }],
    )
    .expect("rebuild should succeed");

    let actual =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/diagram.png"))
            .expect("asset query should succeed")
            .expect("diagram asset should exist");
    assert_eq!(actual.asset_id, existing_asset_id());
    assert_eq!(
        actual.content_hash,
        ContentHash::new("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    );
    assert_eq!(actual.created_at, "2026-04-01T08:00:00Z");
    assert!(actual.updated_at.as_str() > "2026-04-02T09:00:00Z");
}

#[test]
fn asset_hash_match_retains_existing_asset_id_after_path_change() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let existing = asset_record(
        existing_asset_id(),
        "assets/old-diagram.png",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "2026-04-01T08:00:00Z",
        "2026-04-02T09:00:00Z",
    );
    AssetRepo::upsert_asset(&connection, &existing).expect("existing asset upsert should succeed");

    rebuild_index(
        &mut connection,
        &[],
        &[AssetInput {
            path: RelativePath::new("assets/new-diagram.png"),
            content_hash: ContentHash::new(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            mime_type: Some("image/png".to_string()),
            size_bytes: 3333,
            is_trashed: false,
            trashed_at: None,
        }],
    )
    .expect("rebuild should succeed");

    let actual =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/new-diagram.png"))
            .expect("asset query should succeed")
            .expect("renamed asset should exist");
    assert_eq!(actual.asset_id, existing_asset_id());
    assert_eq!(actual.created_at, "2026-04-01T08:00:00Z");
    assert!(actual.updated_at.as_str() > "2026-04-02T09:00:00Z");
}

#[test]
fn new_asset_gets_fresh_asset_id() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    AssetRepo::upsert_asset(
        &connection,
        &asset_record(
            existing_asset_id(),
            "assets/existing.png",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "2026-04-01T08:00:00Z",
            "2026-04-02T09:00:00Z",
        ),
    )
    .expect("existing asset upsert should succeed");

    rebuild_index(
        &mut connection,
        &[],
        &[AssetInput {
            path: RelativePath::new("assets/new.png"),
            content_hash: ContentHash::new(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            mime_type: Some("image/png".to_string()),
            size_bytes: 1111,
            is_trashed: false,
            trashed_at: None,
        }],
    )
    .expect("rebuild should succeed");

    let actual = AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/new.png"))
        .expect("asset query should succeed")
        .expect("new asset should exist");
    assert_ne!(actual.asset_id, existing_asset_id());
    assert!(!actual.asset_id.as_str().is_empty());
}

#[test]
fn rebuild_updates_note_content_hash_on_second_run() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let mut notes = sample_note_inputs();

    rebuild_index(&mut connection, &notes, &[]).expect("first rebuild should succeed");

    notes[0].content_hash =
        ContentHash::new("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    rebuild_index(&mut connection, &notes, &[]).expect("second rebuild should succeed");

    let actual = NoteRepo::get_note(&connection, &alpha_note_id())
        .expect("note query should succeed")
        .expect("alpha note should exist");
    assert_eq!(
        actual.content_hash,
        ContentHash::new("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    );
}

#[test]
fn rebuild_does_not_touch_sync_device_or_delete_tables() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let note_state = note_sync_state(alpha_note_id());
    let asset_state = asset_sync_state(existing_asset_id());
    let device = device_record();
    let delete_record = delete_record();
    SyncStateRepo::upsert_note_sync_state(&connection, &note_state)
        .expect("note sync-state upsert should succeed");
    SyncStateRepo::upsert_asset_sync_state(&connection, &asset_state)
        .expect("asset sync-state upsert should succeed");
    DeviceRepo::upsert_device(&connection, &device).expect("device upsert should succeed");
    DeleteRecordRepo::insert_delete_record(&connection, &delete_record)
        .expect("delete-record insert should succeed");

    rebuild_index(
        &mut connection,
        &sample_note_inputs(),
        &sample_asset_inputs(),
    )
    .expect("rebuild should succeed");

    assert_eq!(
        SyncStateRepo::get_note_sync_state(&connection, &alpha_note_id())
            .expect("note sync-state query should succeed"),
        Some(note_state)
    );
    assert_eq!(
        SyncStateRepo::get_asset_sync_state(&connection, &existing_asset_id())
            .expect("asset sync-state query should succeed"),
        Some(asset_state)
    );
    assert_eq!(
        DeviceRepo::get_device(&connection, &device.device_id)
            .expect("device query should succeed"),
        Some(device)
    );
    assert_eq!(
        DeleteRecordRepo::get_delete_record(&connection, &delete_record.target)
            .expect("delete-record query should succeed"),
        Some(delete_record)
    );
}

#[test]
fn rebuild_preserves_stale_base_rows_but_rebuilds_only_current_derived_rows() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    NoteRepo::upsert_note(
        &connection,
        &NoteId::new("01HZ3NOTE00000000000000ZZ"),
        &RelativePath::new("notes/stale.md"),
        "Stale",
        &ContentHash::new("9999999999999999999999999999999999999999999999999999999999999999"),
        r#"{"id":"01HZ3NOTE00000000000000ZZ","title":"Stale"}"#,
        "2026-04-01T00:00:00Z",
        "2026-04-01T00:00:00Z",
        None,
        false,
    )
    .expect("stale note upsert should succeed");
    NoteRepo::replace_tags(
        &connection,
        &NoteId::new("01HZ3NOTE00000000000000ZZ"),
        &["stale".to_string()],
    )
    .expect("stale tag replacement should succeed");
    LinkRepo::replace_note_links(
        &connection,
        &NoteId::new("01HZ3NOTE00000000000000ZZ"),
        &[ResolvedNoteLink {
            source_note_id: NoteId::new("01HZ3NOTE00000000000000ZZ"),
            raw_target: "Old".to_string(),
            target_note_id: None,
            link_text: None,
            is_ambiguous: false,
        }],
    )
    .expect("stale link replacement should succeed");
    AssetRepo::replace_asset_references(
        &connection,
        &NoteId::new("01HZ3NOTE00000000000000ZZ"),
        &[AssetReference {
            source_note_id: NoteId::new("01HZ3NOTE00000000000000ZZ"),
            raw_path: "assets/stale.png".to_string(),
            target_asset_id: None,
        }],
    )
    .expect("stale asset-reference replacement should succeed");
    AssetRepo::upsert_asset(
        &connection,
        &asset_record(
            second_existing_asset_id(),
            "assets/stale.png",
            "1212121212121212121212121212121212121212121212121212121212121212",
            "2026-04-01T08:00:00Z",
            "2026-04-01T09:00:00Z",
        ),
    )
    .expect("stale asset upsert should succeed");

    rebuild_index(
        &mut connection,
        &sample_note_inputs(),
        &sample_asset_inputs(),
    )
    .expect("rebuild should succeed");

    assert!(
        NoteRepo::get_note(&connection, &NoteId::new("01HZ3NOTE00000000000000ZZ"))
            .expect("stale note query should succeed")
            .is_some()
    );
    assert_eq!(table_count(&connection, "assets_index"), 3);
    assert_eq!(
        NoteRepo::get_tags(&connection, &NoteId::new("01HZ3NOTE00000000000000ZZ"))
            .expect("stale tag query should succeed"),
        Vec::<String>::new()
    );
    assert_eq!(
        LinkRepo::get_note_links(&connection, &NoteId::new("01HZ3NOTE00000000000000ZZ"))
            .expect("stale link query should succeed")
            .len(),
        0
    );
    assert_eq!(
        AssetRepo::get_asset_references(&connection, &NoteId::new("01HZ3NOTE00000000000000ZZ"))
            .expect("stale asset-reference query should succeed")
            .len(),
        0
    );
    assert_eq!(table_count(&connection, "note_tags"), 4);
    assert_eq!(table_count(&connection, "note_links"), 2);
    assert_eq!(table_count(&connection, "asset_references"), 1);
}

#[test]
fn new_asset_timestamps_are_initialized_and_known_asset_timestamps_are_preserved() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let existing = asset_record(
        existing_asset_id(),
        "assets/existing.png",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "2026-04-01T08:00:00Z",
        "2026-04-02T09:00:00Z",
    );
    AssetRepo::upsert_asset(&connection, &existing).expect("existing asset upsert should succeed");

    rebuild_index(
        &mut connection,
        &[],
        &[
            AssetInput {
                path: RelativePath::new("assets/existing.png"),
                content_hash: ContentHash::new(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                mime_type: Some("image/png".to_string()),
                size_bytes: 2048,
                is_trashed: false,
                trashed_at: None,
            },
            AssetInput {
                path: RelativePath::new("assets/new.png"),
                content_hash: ContentHash::new(
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
                mime_type: Some("image/png".to_string()),
                size_bytes: 1024,
                is_trashed: false,
                trashed_at: None,
            },
        ],
    )
    .expect("rebuild should succeed");

    let preserved =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/existing.png"))
            .expect("existing asset query should succeed")
            .expect("existing asset should exist");
    assert_eq!(preserved.created_at, "2026-04-01T08:00:00Z");
    assert_eq!(preserved.updated_at, "2026-04-02T09:00:00Z");

    let initialized =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/new.png"))
            .expect("new asset query should succeed")
            .expect("new asset should exist");
    assert!(!initialized.created_at.is_empty());
    assert_eq!(initialized.created_at, initialized.updated_at);
    assert!(initialized.created_at.ends_with('Z'));
}

#[test]
fn existing_asset_updated_at_bumps_only_on_hash_path_or_size_change() {
    let mut connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let seeded = asset_record(
        existing_asset_id(),
        "assets/existing.png",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "2026-04-01T08:00:00Z",
        "2026-04-02T09:00:00Z",
    );
    AssetRepo::upsert_asset(&connection, &seeded).expect("existing asset upsert should succeed");

    rebuild_index(
        &mut connection,
        &[],
        &[AssetInput {
            path: RelativePath::new("assets/existing.png"),
            content_hash: ContentHash::new(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            mime_type: Some("application/octet-stream".to_string()),
            size_bytes: seeded.size_bytes,
            is_trashed: false,
            trashed_at: None,
        }],
    )
    .expect("first rebuild should succeed");
    let unchanged =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/existing.png"))
            .expect("asset query should succeed")
            .expect("existing asset should exist");
    assert_eq!(unchanged.updated_at, seeded.updated_at);

    thread::sleep(Duration::from_millis(1));
    rebuild_index(
        &mut connection,
        &[],
        &[AssetInput {
            path: RelativePath::new("assets/existing.png"),
            content_hash: ContentHash::new(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            mime_type: Some("application/octet-stream".to_string()),
            size_bytes: seeded.size_bytes,
            is_trashed: false,
            trashed_at: None,
        }],
    )
    .expect("second rebuild should succeed");
    let hash_changed =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/existing.png"))
            .expect("asset query should succeed")
            .expect("existing asset should exist");
    assert!(hash_changed.updated_at > unchanged.updated_at);
    assert_eq!(
        hash_changed.content_hash,
        ContentHash::new("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );

    thread::sleep(Duration::from_millis(1));
    rebuild_index(
        &mut connection,
        &[],
        &[AssetInput {
            path: RelativePath::new("assets/renamed.png"),
            content_hash: ContentHash::new(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            mime_type: Some("application/octet-stream".to_string()),
            size_bytes: seeded.size_bytes,
            is_trashed: false,
            trashed_at: None,
        }],
    )
    .expect("third rebuild should succeed");
    let path_changed =
        AssetRepo::get_asset_by_path(&connection, &RelativePath::new("assets/renamed.png"))
            .expect("asset query should succeed")
            .expect("renamed asset should exist");
    assert!(path_changed.updated_at > hash_changed.updated_at);
    assert_eq!(path_changed.asset_id, existing_asset_id());
}
