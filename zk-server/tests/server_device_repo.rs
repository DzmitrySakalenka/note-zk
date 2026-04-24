use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_server::{
    migration::ensure_schema,
    repo::{DeviceRepo, DeviceRow},
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

fn sample_device_row(
    device_id: &str,
    device_name: &str,
    created_at: &str,
    last_seen_at: &str,
) -> DeviceRow {
    DeviceRow {
        device_id: device_id.to_string(),
        device_name: device_name.to_string(),
        platform: Some("macos".to_string()),
        app_version: Some("1.0.0".to_string()),
        capabilities_json: r#"["pull","push"]"#.to_string(),
        created_at: parse_timestamp(created_at),
        last_seen_at: parse_timestamp(last_seen_at),
    }
}

#[test]
fn upsert_then_get_returns_inserted_row_unchanged() {
    let connection = setup_connection();
    let inserted = sample_device_row(
        "device-a",
        "Alpha",
        "2026-04-23T12:00:00.123Z",
        "2026-04-23T12:05:00.456Z",
    );

    DeviceRepo::upsert(&connection, &inserted).expect("device upsert should succeed");

    let loaded = DeviceRepo::get(&connection, "device-a")
        .expect("device lookup should succeed")
        .expect("device row should exist");

    assert_eq!(loaded.device_id, inserted.device_id);
    assert_eq!(loaded.device_name, inserted.device_name);
    assert_eq!(loaded.platform, inserted.platform);
    assert_eq!(loaded.app_version, inserted.app_version);
    assert_eq!(loaded.capabilities_json, inserted.capabilities_json);
    assert_eq!(loaded.created_at, inserted.created_at);
    assert_eq!(loaded.last_seen_at, inserted.last_seen_at);
    assert_eq!(
        loaded.created_at.timestamp_millis(),
        inserted.created_at.timestamp_millis()
    );
    assert_eq!(
        loaded.last_seen_at.timestamp_millis(),
        inserted.last_seen_at.timestamp_millis()
    );
}

#[test]
fn upsert_twice_updates_mutable_fields_and_preserves_created_at() {
    let connection = setup_connection();
    let first = sample_device_row(
        "device-a",
        "Alpha",
        "2026-04-23T12:00:00.123Z",
        "2026-04-23T12:05:00.456Z",
    );
    let mut second = sample_device_row(
        "device-a",
        "Beta",
        "2026-04-24T12:00:00.000Z",
        "2026-04-24T12:10:00.789Z",
    );
    second.platform = Some("linux".to_string());
    second.app_version = Some("2.0.0".to_string());
    second.capabilities_json = r#"["push"]"#.to_string();

    DeviceRepo::upsert(&connection, &first).expect("first device upsert should succeed");
    DeviceRepo::upsert(&connection, &second).expect("second device upsert should succeed");

    let loaded = DeviceRepo::get(&connection, "device-a")
        .expect("device lookup should succeed")
        .expect("device row should exist");

    assert_eq!(loaded.device_name, "Beta");
    assert_eq!(loaded.platform.as_deref(), Some("linux"));
    assert_eq!(loaded.app_version.as_deref(), Some("2.0.0"));
    assert_eq!(loaded.capabilities_json, r#"["push"]"#);
    assert_eq!(
        loaded.last_seen_at,
        parse_timestamp("2026-04-24T12:10:00.789Z")
    );
    assert_eq!(loaded.created_at, first.created_at);
}

#[test]
fn list_returns_rows_sorted_by_device_id_ascending() {
    let connection = setup_connection();

    DeviceRepo::upsert(
        &connection,
        &sample_device_row(
            "device-c",
            "Gamma",
            "2026-04-23T12:00:00Z",
            "2026-04-23T12:00:00Z",
        ),
    )
    .expect("device-c upsert should succeed");
    DeviceRepo::upsert(
        &connection,
        &sample_device_row(
            "device-a",
            "Alpha",
            "2026-04-23T12:01:00Z",
            "2026-04-23T12:01:00Z",
        ),
    )
    .expect("device-a upsert should succeed");
    DeviceRepo::upsert(
        &connection,
        &sample_device_row(
            "device-b",
            "Beta",
            "2026-04-23T12:02:00Z",
            "2026-04-23T12:02:00Z",
        ),
    )
    .expect("device-b upsert should succeed");

    let device_ids: Vec<String> = DeviceRepo::list(&connection)
        .expect("device list should succeed")
        .into_iter()
        .map(|row| row.device_id)
        .collect();

    assert_eq!(
        device_ids,
        vec![
            "device-a".to_string(),
            "device-b".to_string(),
            "device-c".to_string(),
        ]
    );
}

#[test]
fn update_last_seen_changes_only_last_seen_at() {
    let connection = setup_connection();
    let inserted = sample_device_row(
        "device-a",
        "Alpha",
        "2026-04-23T12:00:00Z",
        "2026-04-23T12:05:00Z",
    );
    let new_last_seen = parse_timestamp("2026-04-23T13:00:00.321Z");

    DeviceRepo::upsert(&connection, &inserted).expect("device upsert should succeed");
    DeviceRepo::update_last_seen(&connection, "device-a", new_last_seen)
        .expect("update_last_seen should succeed");

    let loaded = DeviceRepo::get(&connection, "device-a")
        .expect("device lookup should succeed")
        .expect("device row should exist");

    assert_eq!(loaded.device_id, inserted.device_id);
    assert_eq!(loaded.device_name, inserted.device_name);
    assert_eq!(loaded.platform, inserted.platform);
    assert_eq!(loaded.app_version, inserted.app_version);
    assert_eq!(loaded.capabilities_json, inserted.capabilities_json);
    assert_eq!(loaded.created_at, inserted.created_at);
    assert_eq!(loaded.last_seen_at, new_last_seen);
}

#[test]
fn missing_device_returns_none_and_false() {
    let connection = setup_connection();

    let loaded = DeviceRepo::get(&connection, "missing-device")
        .expect("missing device lookup should succeed");
    let exists = DeviceRepo::exists(&connection, "missing-device")
        .expect("missing device exists query should succeed");

    assert!(loaded.is_none());
    assert!(!exists);
}
