use chrono::{DateTime, Utc};
use zk_server::{
    ServerError,
    db::ServerDb,
    repo::{DeviceCursorRepo, DeviceRepo},
    service::{RegisterDeviceInput, register_device},
};

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

#[test]
fn first_registration_creates_device_and_cursor_row() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let now = parse_timestamp("2026-04-24T10:00:00.123Z");

    let outcome = register_device(
        &db,
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Alpha",
            platform: Some("macos"),
            app_version: Some("1.0.0"),
            capabilities_json: r#"["pull","push"]"#,
        },
        now,
    )
    .expect("first registration should succeed");

    assert!(outcome.first_registration);
    assert_eq!(outcome.created_at, now);
    assert_eq!(outcome.last_seen_at, now);

    db.with_conn(|connection| {
        let device = DeviceRepo::get(connection, "device-a")?
            .expect("device row should exist after registration");
        let cursor = DeviceCursorRepo::get_cursor(connection, "device-a")?;

        assert_eq!(device.device_id, "device-a");
        assert_eq!(device.device_name, "Alpha");
        assert_eq!(device.platform.as_deref(), Some("macos"));
        assert_eq!(device.app_version.as_deref(), Some("1.0.0"));
        assert_eq!(device.capabilities_json, r#"["pull","push"]"#);
        assert_eq!(device.created_at, now);
        assert_eq!(device.last_seen_at, now);
        assert_eq!(cursor, 0);

        Ok(())
    })
    .expect("device state lookup should succeed");
}

#[test]
fn reregistration_updates_mutable_fields_and_preserves_created_at() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let first_now = parse_timestamp("2026-04-24T10:00:00.123Z");
    let second_now = parse_timestamp("2026-04-24T11:00:00.456Z");

    register_device(
        &db,
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Alpha",
            platform: Some("macos"),
            app_version: Some("1.0.0"),
            capabilities_json: r#"["pull"]"#,
        },
        first_now,
    )
    .expect("first registration should succeed");

    let outcome = register_device(
        &db,
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Beta",
            platform: Some("linux"),
            app_version: Some("2.0.0"),
            capabilities_json: r#"["push"]"#,
        },
        second_now,
    )
    .expect("second registration should succeed");

    assert!(!outcome.first_registration);
    assert_eq!(outcome.created_at, first_now);
    assert_eq!(outcome.last_seen_at, second_now);

    db.with_conn(|connection| {
        let device = DeviceRepo::get(connection, "device-a")?
            .expect("device row should exist after re-registration");

        assert_eq!(device.device_name, "Beta");
        assert_eq!(device.platform.as_deref(), Some("linux"));
        assert_eq!(device.app_version.as_deref(), Some("2.0.0"));
        assert_eq!(device.capabilities_json, r#"["push"]"#);
        assert_eq!(device.created_at, first_now);
        assert_eq!(device.last_seen_at, second_now);

        Ok(())
    })
    .expect("device state lookup should succeed");
}

#[test]
fn reregistration_does_not_reinitialize_advanced_cursor() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let first_now = parse_timestamp("2026-04-24T10:00:00Z");
    let cursor_now = parse_timestamp("2026-04-24T10:30:00Z");
    let second_now = parse_timestamp("2026-04-24T11:00:00Z");

    register_device(
        &db,
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Alpha",
            platform: Some("macos"),
            app_version: Some("1.0.0"),
            capabilities_json: r#"["pull"]"#,
        },
        first_now,
    )
    .expect("first registration should succeed");

    db.with_conn(|connection| {
        DeviceCursorRepo::set_cursor(connection, "device-a", 7, cursor_now)?;
        Ok(())
    })
    .expect("cursor advance should succeed");

    register_device(
        &db,
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Beta",
            platform: Some("linux"),
            app_version: Some("2.0.0"),
            capabilities_json: r#"["push"]"#,
        },
        second_now,
    )
    .expect("re-registration should succeed");

    db.with_conn(|connection| {
        let cursor = DeviceCursorRepo::get_cursor(connection, "device-a")?;
        assert_eq!(cursor, 7);
        Ok(())
    })
    .expect("cursor lookup should succeed");
}

#[test]
fn invalid_device_id_returns_invalid_request() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let now = parse_timestamp("2026-04-24T10:00:00Z");
    let too_long_device_id = "a".repeat(129);
    let cases = vec![
        (String::new(), "device_id must not be empty"),
        (
            too_long_device_id,
            "device_id must be at most 128 characters",
        ),
        (
            "bad/id".to_string(),
            "device_id must contain only ASCII letters, digits, '-' or '_'",
        ),
    ];

    for (device_id, expected_message) in cases {
        let error = register_device(
            &db,
            RegisterDeviceInput {
                device_id: &device_id,
                device_name: "Alpha",
                platform: Some("macos"),
                app_version: Some("1.0.0"),
                capabilities_json: r#"["pull"]"#,
            },
            now,
        )
        .expect_err("invalid device id should be rejected");

        assert!(
            matches!(error, ServerError::InvalidRequest(message) if message == expected_message)
        );
    }
}

#[test]
fn invalid_capabilities_json_returns_invalid_request() {
    let db = ServerDb::open_in_memory().expect("in-memory database should open");
    let now = parse_timestamp("2026-04-24T10:00:00Z");

    let error = register_device(
        &db,
        RegisterDeviceInput {
            device_id: "device-a",
            device_name: "Alpha",
            platform: Some("macos"),
            app_version: Some("1.0.0"),
            capabilities_json: "{not-json",
        },
        now,
    )
    .expect_err("invalid capabilities json should be rejected");

    assert!(
        matches!(error, ServerError::InvalidRequest(message) if message == "capabilities_json must be valid JSON")
    );

    db.with_conn(|connection| {
        let device = DeviceRepo::get(connection, "device-a")?;
        assert!(device.is_none());
        Ok(())
    })
    .expect("device lookup should succeed");
}
