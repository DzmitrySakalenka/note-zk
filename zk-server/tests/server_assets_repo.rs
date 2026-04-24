use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{AssetRow, AssetsRepo},
};

fn setup_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn asset_fixture(asset_id: &str) -> AssetRow {
    AssetRow {
        asset_id: asset_id.to_string(),
        current_path: format!("assets/{asset_id}.png"),
        content_hash: format!("hash-{asset_id}"),
        mime_type: Some("image/png".to_string()),
        size_bytes: 4096,
        created_at: parse_timestamp("2026-04-24T10:00:00.123Z"),
        updated_at: parse_timestamp("2026-04-24T10:05:00.456Z"),
        trashed_at: Some(parse_timestamp("2026-04-24T10:10:00.789Z")),
        deleted_at: None,
        is_trashed: true,
        is_deleted: false,
        server_revision: 1,
    }
}

fn assert_asset_row_eq(actual: &AssetRow, expected: &AssetRow) {
    assert_eq!(actual.asset_id, expected.asset_id);
    assert_eq!(actual.current_path, expected.current_path);
    assert_eq!(actual.content_hash, expected.content_hash);
    assert_eq!(actual.mime_type, expected.mime_type);
    assert_eq!(actual.size_bytes, expected.size_bytes);
    assert_eq!(actual.created_at, expected.created_at);
    assert_eq!(actual.updated_at, expected.updated_at);
    assert_eq!(actual.trashed_at, expected.trashed_at);
    assert_eq!(actual.deleted_at, expected.deleted_at);
    assert_eq!(actual.is_trashed, expected.is_trashed);
    assert_eq!(actual.is_deleted, expected.is_deleted);
    assert_eq!(actual.server_revision, expected.server_revision);
}

#[test]
fn create_then_get_round_trips_every_field() {
    let connection = setup_connection();
    let asset = asset_fixture("asset-a");

    AssetsRepo::create(&connection, &asset).expect("asset create should succeed");

    let loaded = AssetsRepo::get(&connection, "asset-a")
        .expect("asset lookup should succeed")
        .expect("asset row should exist");

    assert_asset_row_eq(&loaded, &asset);
}

#[test]
fn create_twice_with_same_id_returns_conflict() {
    let connection = setup_connection();
    let first = asset_fixture("asset-a");
    let mut duplicate = asset_fixture("asset-a");
    duplicate.current_path = "assets/other-path.jpg".to_string();

    AssetsRepo::create(&connection, &first).expect("first asset create should succeed");
    let error =
        AssetsRepo::create(&connection, &duplicate).expect_err("duplicate asset id should fail");

    assert!(matches!(error, ServerError::Conflict(message) if message == "asset already exists"));
}

#[test]
fn update_changes_all_fields_and_is_visible_on_next_get() {
    let connection = setup_connection();
    let original = asset_fixture("asset-a");
    let mut updated = asset_fixture("asset-a");
    updated.current_path = "archive/asset-a.webp".to_string();
    updated.content_hash = "hash-asset-a-updated".to_string();
    updated.mime_type = Some("image/webp".to_string());
    updated.size_bytes = 8192;
    updated.created_at = parse_timestamp("2026-04-24T10:00:00.123Z");
    updated.updated_at = parse_timestamp("2026-04-24T11:00:00.000Z");
    updated.trashed_at = None;
    updated.deleted_at = Some(parse_timestamp("2026-04-24T11:30:00.321Z"));
    updated.is_trashed = false;
    updated.is_deleted = true;
    updated.server_revision = 7;

    AssetsRepo::create(&connection, &original).expect("original asset create should succeed");
    AssetsRepo::update(&connection, &updated).expect("asset update should succeed");

    let loaded = AssetsRepo::get(&connection, "asset-a")
        .expect("asset lookup should succeed")
        .expect("asset row should exist");

    assert_asset_row_eq(&loaded, &updated);
}

#[test]
fn list_since_revision_zero_matches_list_all_when_nothing_is_deleted() {
    let connection = setup_connection();
    let mut first = asset_fixture("asset-a");
    first.trashed_at = None;
    first.is_trashed = false;
    first.server_revision = 1;
    let mut second = asset_fixture("asset-b");
    second.mime_type = None;
    second.trashed_at = None;
    second.is_trashed = false;
    second.server_revision = 2;

    AssetsRepo::create(&connection, &first).expect("first asset create should succeed");
    AssetsRepo::create(&connection, &second).expect("second asset create should succeed");

    let all_rows = AssetsRepo::list_all(&connection).expect("list_all should succeed");
    let since_rows = AssetsRepo::list_since_revision(&connection, 0)
        .expect("list_since_revision should succeed");

    assert_eq!(all_rows.len(), 2);
    assert_eq!(since_rows.len(), 2);
    assert_asset_row_eq(&all_rows[0], &since_rows[0]);
    assert_asset_row_eq(&all_rows[1], &since_rows[1]);
}

#[test]
fn get_by_path_hides_soft_deleted_rows() {
    let connection = setup_connection();
    let mut deleted = asset_fixture("asset-a");
    deleted.trashed_at = None;
    deleted.deleted_at = Some(parse_timestamp("2026-04-24T12:00:00Z"));
    deleted.is_trashed = false;
    deleted.is_deleted = true;

    AssetsRepo::create(&connection, &deleted).expect("deleted asset create should succeed");

    let loaded = AssetsRepo::get_by_path(&connection, "assets/asset-a.png")
        .expect("path lookup should succeed");

    assert!(loaded.is_none());
}

#[test]
fn delete_hard_removes_row_entirely() {
    let connection = setup_connection();
    let asset = asset_fixture("asset-a");

    AssetsRepo::create(&connection, &asset).expect("asset create should succeed");
    AssetsRepo::delete_hard(&connection, "asset-a").expect("hard delete should succeed");

    let loaded = AssetsRepo::get(&connection, "asset-a").expect("asset lookup should succeed");

    assert!(loaded.is_none());
}
