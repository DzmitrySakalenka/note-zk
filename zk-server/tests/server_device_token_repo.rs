// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fmt::Write as _;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use zk_domain::AuthScope;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{DeviceTokenInsert, DeviceTokenRepo, DeviceTokenRow},
};

const INSERT_DEVICE_TOKEN_SQL: &str = r"
INSERT INTO device_tokens (
    token_id,
    device_id,
    user_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
)
";
const INSERT_USER_SQL: &str = r"
INSERT OR IGNORE INTO users (
    user_id,
    username,
    password_hash,
    created_at
) VALUES (
    ?1, ?2, 'test-password-hash', '2026-05-13T12:00:00Z'
)
";
const INSERT_DEVICE_SQL: &str = r"
INSERT OR IGNORE INTO devices (
    device_id,
    device_name,
    capabilities_json,
    created_at,
    last_seen_at
) VALUES (
    ?1, ?2, '{}', '2026-04-24T09:00:00Z', '2026-04-24T09:00:00Z'
)
";
const SELECT_DEVICE_TOKEN_BY_ID_SQL: &str = r"
SELECT
    token_id,
    device_id,
    user_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
FROM device_tokens
WHERE token_id = ?1
LIMIT 1
";

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

struct TokenFixture<'a> {
    token_id: &'a str,
    device_id: Option<&'a str>,
    user_id: Option<&'a str>,
    label: &'a str,
    token_sha256_hex: &'a str,
    scopes_json: &'a str,
    created_at: &'a str,
    last_used_at: Option<&'a str>,
    revoked_at: Option<&'a str>,
}

fn insert_token_row(connection: &Connection, fixture: &TokenFixture<'_>) {
    if let Some(device_id) = fixture.device_id {
        insert_device(connection, device_id);
    }
    if let Some(user_id) = fixture.user_id {
        insert_user(connection, user_id);
    }

    connection
        .execute(
            INSERT_DEVICE_TOKEN_SQL,
            rusqlite::params![
                fixture.token_id,
                fixture.device_id,
                fixture.user_id,
                fixture.label,
                fixture.token_sha256_hex,
                fixture.scopes_json,
                fixture.created_at,
                fixture.last_used_at,
                fixture.revoked_at,
            ],
        )
        .expect("device token fixture insert should succeed");
}

fn insert_device(connection: &Connection, device_id: &str) {
    connection
        .execute(INSERT_DEVICE_SQL, rusqlite::params![device_id, device_id])
        .expect("device fixture insert should succeed");
}

fn insert_user(connection: &Connection, user_id: &str) {
    connection
        .execute(INSERT_USER_SQL, rusqlite::params![user_id, user_id])
        .expect("user fixture insert should succeed");
}

fn load_token_by_id(connection: &Connection, token_id: &str) -> DeviceTokenRow {
    connection
        .query_row(SELECT_DEVICE_TOKEN_BY_ID_SQL, [token_id], |row| {
            let created_at = DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                .expect("created_at fixture should parse")
                .with_timezone(&Utc);
            let last_used_at = row.get::<_, Option<String>>(7)?.map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .expect("last_used_at fixture should parse")
                    .with_timezone(&Utc)
            });
            let revoked_at = row.get::<_, Option<String>>(8)?.map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .expect("revoked_at fixture should parse")
                    .with_timezone(&Utc)
            });
            let scopes = serde_json::from_str(&row.get::<_, String>(5)?)
                .expect("scopes fixture should parse");

            Ok(DeviceTokenRow {
                token_id: row.get(0)?,
                device_id: row.get(1)?,
                user_id: row.get(2)?,
                label: row.get(3)?,
                token_sha256_hex: row.get(4)?,
                scopes,
                created_at,
                last_used_at,
                revoked_at,
            })
        })
        .expect("raw device token lookup should succeed")
}

fn insert_fixture(
    connection: &Connection,
    token_id: &'static str,
    device_id: Option<&'static str>,
    token_sha256_hex: &'static str,
) {
    insert_token_row(
        connection,
        &TokenFixture {
            token_id,
            device_id,
            user_id: None,
            label: "CLI token",
            token_sha256_hex,
            scopes_json: r#"["sync"]"#,
            created_at: "2026-04-24T10:00:00Z",
            last_used_at: None,
            revoked_at: None,
        },
    );
}

fn issue_row(
    token_id: &str,
    device_id: Option<&str>,
    created_at: DateTime<Utc>,
) -> DeviceTokenInsert {
    DeviceTokenInsert {
        token_id: token_id.to_string(),
        device_id: device_id.map(str::to_string),
        user_id: None,
        label: format!("Token {token_id}"),
        token_sha256_hex: hash_for_token_id(token_id),
        scopes: vec![AuthScope::Sync],
        created_at,
    }
}

fn hash_for_token_id(token_id: &str) -> String {
    let mut hash = String::with_capacity(64);
    for byte in token_id.bytes() {
        write!(&mut hash, "{byte:02x}").expect("writing to String should not fail");
    }
    hash.push_str("0000000000000000000000000000000000000000000000000000000000000000");
    hash.truncate(64);
    hash
}

#[test]
fn lookup_by_hash_finds_inserted_row() {
    let connection = setup_connection();
    let token_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_token_row(
        &connection,
        &TokenFixture {
            token_id: "token-a",
            device_id: None,
            user_id: None,
            label: "CLI token",
            token_sha256_hex: token_hash,
            scopes_json: r#"["sync"]"#,
            created_at: "2026-04-24T10:00:00.123Z",
            last_used_at: Some("2026-04-24T10:05:00.456Z"),
            revoked_at: None,
        },
    );

    let loaded = DeviceTokenRepo::lookup_by_hash(&connection, token_hash)
        .expect("lookup_by_hash should succeed")
        .expect("active token should be returned");

    assert_eq!(loaded.token_id, "token-a");
    assert_eq!(loaded.device_id, None);
    assert_eq!(loaded.user_id, None);
    assert_eq!(loaded.label, "CLI token");
    assert_eq!(loaded.token_sha256_hex, token_hash);
    assert_eq!(loaded.scopes, vec![AuthScope::Sync]);
    assert_eq!(
        loaded.created_at,
        parse_timestamp("2026-04-24T10:00:00.123Z")
    );
    assert_eq!(
        loaded.last_used_at,
        Some(parse_timestamp("2026-04-24T10:05:00.456Z"))
    );
    assert_eq!(loaded.revoked_at, None);
}

#[test]
fn lookup_by_hash_returns_none_for_unknown_hash() {
    let connection = setup_connection();

    let loaded = DeviceTokenRepo::lookup_by_hash(
        &connection,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("unknown hash lookup should succeed");

    assert!(loaded.is_none());
}

#[test]
fn revoked_row_is_hidden_from_lookup_by_hash() {
    let connection = setup_connection();
    let token_hash = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    insert_token_row(
        &connection,
        &TokenFixture {
            token_id: "token-a",
            device_id: None,
            user_id: None,
            label: "CLI token",
            token_sha256_hex: token_hash,
            scopes_json: r#"["sync"]"#,
            created_at: "2026-04-24T10:00:00Z",
            last_used_at: None,
            revoked_at: Some("2026-04-24T11:00:00Z"),
        },
    );

    let loaded = DeviceTokenRepo::lookup_by_hash(&connection, token_hash)
        .expect("revoked token lookup should succeed");

    assert!(loaded.is_none());
}

#[test]
fn touch_last_used_updates_only_last_used_at() {
    let connection = setup_connection();
    let token_hash = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    insert_token_row(
        &connection,
        &TokenFixture {
            token_id: "token-a",
            device_id: None,
            user_id: None,
            label: "CLI token",
            token_sha256_hex: token_hash,
            scopes_json: r#"["sync"]"#,
            created_at: "2026-04-24T10:00:00Z",
            last_used_at: Some("2026-04-24T10:05:00Z"),
            revoked_at: None,
        },
    );
    let before = load_token_by_id(&connection, "token-a");
    let new_last_used = parse_timestamp("2026-04-24T12:34:56.789Z");

    DeviceTokenRepo::touch_last_used(&connection, "token-a", new_last_used)
        .expect("touch_last_used should succeed");

    let after = load_token_by_id(&connection, "token-a");

    assert_eq!(after.token_id, before.token_id);
    assert_eq!(after.device_id, before.device_id);
    assert_eq!(after.label, before.label);
    assert_eq!(after.token_sha256_hex, before.token_sha256_hex);
    assert_eq!(after.scopes, before.scopes);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.revoked_at, before.revoked_at);
    assert_eq!(after.last_used_at, Some(new_last_used));
}

#[test]
fn issue_then_get_round_trips_with_scopes() {
    let connection = setup_connection();
    let created_at = parse_timestamp("2026-04-24T10:00:00Z");
    insert_device(&connection, "device-a");
    insert_user(&connection, "user-a");
    let row = DeviceTokenInsert {
        token_id: "token-a".to_string(),
        device_id: Some("device-a".to_string()),
        user_id: Some("user-a".to_string()),
        label: "Laptop token".to_string(),
        token_sha256_hex: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            .to_string(),
        scopes: vec![AuthScope::Sync, AuthScope::Admin],
        created_at,
    };

    DeviceTokenRepo::issue(&connection, &row).expect("token issue should succeed");

    let loaded = DeviceTokenRepo::get(&connection, "token-a")
        .expect("token get should succeed")
        .expect("token row should exist");

    assert_eq!(loaded.token_id, row.token_id);
    assert_eq!(loaded.device_id, row.device_id);
    assert_eq!(loaded.user_id, row.user_id);
    assert_eq!(loaded.label, row.label);
    assert_eq!(loaded.token_sha256_hex, row.token_sha256_hex);
    assert_eq!(loaded.scopes, row.scopes);
    assert_eq!(loaded.created_at, created_at);
    assert_eq!(loaded.last_used_at, None);
    assert_eq!(loaded.revoked_at, None);
}

#[test]
fn list_for_device_filters_to_device_id() {
    let connection = setup_connection();
    insert_fixture(
        &connection,
        "token-a",
        Some("device-a"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    insert_fixture(
        &connection,
        "token-b",
        Some("device-b"),
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    insert_fixture(
        &connection,
        "token-admin",
        None,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    );

    let device_a_ids: Vec<String> = DeviceTokenRepo::list_for_device(&connection, Some("device-a"))
        .expect("device token list should succeed")
        .into_iter()
        .map(|row| row.token_id)
        .collect();
    let unbound_ids: Vec<String> = DeviceTokenRepo::list_for_device(&connection, None)
        .expect("unbound token list should succeed")
        .into_iter()
        .map(|row| row.token_id)
        .collect();

    assert_eq!(device_a_ids, vec!["token-a".to_string()]);
    assert_eq!(unbound_ids, vec!["token-admin".to_string()]);
}

#[test]
fn list_for_device_excludes_revoked_rows() {
    let connection = setup_connection();
    insert_token_row(
        &connection,
        &TokenFixture {
            token_id: "token-a",
            device_id: Some("device-a"),
            user_id: None,
            label: "CLI token",
            token_sha256_hex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            scopes_json: r#"["sync"]"#,
            created_at: "2026-04-24T10:00:00Z",
            last_used_at: None,
            revoked_at: None,
        },
    );
    insert_token_row(
        &connection,
        &TokenFixture {
            token_id: "token-b",
            device_id: Some("device-a"),
            user_id: None,
            label: "CLI token",
            token_sha256_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            scopes_json: r#"["sync"]"#,
            created_at: "2026-04-24T10:01:00Z",
            last_used_at: None,
            revoked_at: Some("2026-04-24T11:00:00Z"),
        },
    );

    let token_ids: Vec<String> = DeviceTokenRepo::list_for_device(&connection, Some("device-a"))
        .expect("device token list should succeed")
        .into_iter()
        .map(|row| row.token_id)
        .collect();

    assert_eq!(token_ids, vec!["token-a".to_string()]);
}

#[test]
fn list_all_active_orders_by_created_at_desc_and_caps_at_1000() {
    let connection = setup_connection();
    let base = parse_timestamp("2026-04-24T10:00:00Z");

    for index in 0..1001 {
        let token_id = format!("token-{index:04}");
        let row = issue_row(&token_id, None, base + Duration::seconds(index));
        DeviceTokenRepo::issue(&connection, &row).expect("bulk token issue should succeed");
    }

    let rows = DeviceTokenRepo::list_all_active(&connection).expect("active list should succeed");

    assert_eq!(rows.len(), 1000);
    assert_eq!(rows[0].token_id, "token-1000");
    assert_eq!(rows[999].token_id, "token-0001");
}

#[test]
fn revoke_returns_true_for_first_call_false_for_second() {
    let connection = setup_connection();
    insert_fixture(
        &connection,
        "token-a",
        None,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let revoked_at = parse_timestamp("2026-04-24T11:00:00Z");

    let first = DeviceTokenRepo::revoke(&connection, "token-a", revoked_at)
        .expect("first revoke should succeed");
    let second = DeviceTokenRepo::revoke(&connection, "token-a", revoked_at)
        .expect("second revoke should succeed");

    let loaded = DeviceTokenRepo::get(&connection, "token-a")
        .expect("token get should succeed")
        .expect("token should still exist");

    assert!(first);
    assert!(!second);
    assert_eq!(loaded.revoked_at, Some(revoked_at));
}

#[test]
fn revoke_hides_token_from_lookup_by_hash() {
    let connection = setup_connection();
    let token_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_fixture(&connection, "token-a", None, token_hash);

    DeviceTokenRepo::revoke(
        &connection,
        "token-a",
        parse_timestamp("2026-04-24T11:00:00Z"),
    )
    .expect("revoke should succeed");

    let loaded = DeviceTokenRepo::lookup_by_hash(&connection, token_hash)
        .expect("lookup by hash should succeed");

    assert!(loaded.is_none());
}

#[test]
fn malformed_scopes_json_returns_typed_error() {
    let connection = setup_connection();
    let token_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_token_row(
        &connection,
        &TokenFixture {
            token_id: "token-a",
            device_id: None,
            user_id: None,
            label: "CLI token",
            token_sha256_hex: token_hash,
            scopes_json: "not-json",
            created_at: "2026-04-24T10:00:00Z",
            last_used_at: None,
            revoked_at: None,
        },
    );

    let error = DeviceTokenRepo::lookup_by_hash(&connection, token_hash)
        .expect_err("malformed scopes JSON should fail row decoding");

    assert!(matches!(error, ServerError::ScopeDecode(token_id) if token_id == "token-a"));
}
