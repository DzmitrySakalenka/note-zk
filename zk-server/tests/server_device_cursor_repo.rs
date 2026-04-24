use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{DeviceCursorRepo, DeviceRepo, DeviceRow},
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

fn sample_device_row(device_id: &str) -> DeviceRow {
    DeviceRow {
        device_id: device_id.to_string(),
        device_name: format!("Device {device_id}"),
        platform: Some("macos".to_string()),
        app_version: Some("1.0.0".to_string()),
        capabilities_json: r#"["pull","push"]"#.to_string(),
        created_at: parse_timestamp("2026-04-24T10:00:00.123Z"),
        last_seen_at: parse_timestamp("2026-04-24T10:05:00.456Z"),
    }
}

fn cursor_row_count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM device_sync_cursors", [], |row| {
            row.get(0)
        })
        .expect("cursor row count query should succeed")
}

fn cursor_updated_at(connection: &Connection, device_id: &str) -> Option<String> {
    connection
        .query_row(
            "SELECT updated_at FROM device_sync_cursors WHERE device_id = ?1",
            [device_id],
            |row| row.get(0),
        )
        .optional()
        .expect("cursor updated_at query should succeed")
}

#[test]
fn get_cursor_on_unknown_device_returns_zero_without_inserting() {
    let connection = setup_connection();

    let cursor = DeviceCursorRepo::get_cursor(&connection, "missing-device")
        .expect("get_cursor should work");

    assert_eq!(cursor, 0);
    assert_eq!(cursor_row_count(&connection), 0);
}

#[test]
fn initialize_creates_row_and_second_call_is_idempotent() {
    let connection = setup_connection();
    let device = sample_device_row("device-a");
    let first_now = parse_timestamp("2026-04-24T12:00:00.000Z");
    let second_now = parse_timestamp("2026-04-24T13:00:00.000Z");

    DeviceRepo::upsert(&connection, &device).expect("device upsert should succeed");
    DeviceCursorRepo::initialize(&connection, "device-a", first_now)
        .expect("first initialize should succeed");
    let first_updated_at = cursor_updated_at(&connection, "device-a")
        .expect("cursor row should exist after initialize");

    DeviceCursorRepo::initialize(&connection, "device-a", second_now)
        .expect("second initialize should succeed");

    assert_eq!(
        DeviceCursorRepo::get_cursor(&connection, "device-a").expect("get_cursor should succeed"),
        0
    );
    assert_eq!(cursor_row_count(&connection), 1);
    assert_eq!(first_updated_at, first_now.to_rfc3339());
    assert_eq!(
        cursor_updated_at(&connection, "device-a"),
        Some(first_updated_at)
    );
}

#[test]
fn set_cursor_advances_and_rejects_lower_cursor() {
    let connection = setup_connection();
    let device = sample_device_row("device-a");
    let updated_at = parse_timestamp("2026-04-24T12:00:00.000Z");
    let later_updated_at = parse_timestamp("2026-04-24T13:00:00.000Z");

    DeviceRepo::upsert(&connection, &device).expect("device upsert should succeed");
    DeviceCursorRepo::set_cursor(&connection, "device-a", 2, updated_at)
        .expect("cursor advance should succeed");

    assert_eq!(
        DeviceCursorRepo::get_cursor(&connection, "device-a").expect("get_cursor should succeed"),
        2
    );

    let error = DeviceCursorRepo::set_cursor(&connection, "device-a", 1, later_updated_at)
        .expect_err("cursor rewind should fail");

    assert!(
        matches!(error, ServerError::InvalidRequest(message) if message == "cursor must not decrease")
    );
    assert_eq!(
        DeviceCursorRepo::get_cursor(&connection, "device-a").expect("get_cursor should succeed"),
        2
    );
}
