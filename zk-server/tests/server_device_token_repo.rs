// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_server::{
    migration::ensure_schema,
    repo::{DeviceTokenRepo, DeviceTokenRow},
};

const INSERT_DEVICE_TOKEN_SQL: &str = r"
INSERT INTO device_tokens (
    token_id,
    device_id,
    label,
    token_sha256_hex,
    scopes_json,
    created_at,
    last_used_at,
    revoked_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
)
";
const SELECT_DEVICE_TOKEN_BY_ID_SQL: &str = r"
SELECT
    token_id,
    device_id,
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
    token_sha256_hex: &'a str,
    created_at: &'a str,
    last_used_at: Option<&'a str>,
    revoked_at: Option<&'a str>,
}

fn insert_token_row(connection: &Connection, fixture: &TokenFixture<'_>) {
    connection
        .execute(
            INSERT_DEVICE_TOKEN_SQL,
            rusqlite::params![
                fixture.token_id,
                fixture.device_id,
                "CLI token",
                fixture.token_sha256_hex,
                r#"["sync"]"#,
                fixture.created_at,
                fixture.last_used_at,
                fixture.revoked_at,
            ],
        )
        .expect("device token fixture insert should succeed");
}

fn load_token_by_id(connection: &Connection, token_id: &str) -> DeviceTokenRow {
    connection
        .query_row(SELECT_DEVICE_TOKEN_BY_ID_SQL, [token_id], |row| {
            let created_at = DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                .expect("created_at fixture should parse")
                .with_timezone(&Utc);
            let last_used_at = row.get::<_, Option<String>>(6)?.map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .expect("last_used_at fixture should parse")
                    .with_timezone(&Utc)
            });
            let revoked_at = row.get::<_, Option<String>>(7)?.map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .expect("revoked_at fixture should parse")
                    .with_timezone(&Utc)
            });

            Ok(DeviceTokenRow {
                token_id: row.get(0)?,
                device_id: row.get(1)?,
                label: row.get(2)?,
                token_sha256_hex: row.get(3)?,
                scopes_json: row.get(4)?,
                created_at,
                last_used_at,
                revoked_at,
            })
        })
        .expect("raw device token lookup should succeed")
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
            token_sha256_hex: token_hash,
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
    assert_eq!(loaded.label, "CLI token");
    assert_eq!(loaded.token_sha256_hex, token_hash);
    assert_eq!(loaded.scopes_json, r#"["sync"]"#);
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
            token_sha256_hex: token_hash,
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
            token_sha256_hex: token_hash,
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
    assert_eq!(after.scopes_json, before.scopes_json);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.revoked_at, before.revoked_at);
    assert_eq!(after.last_used_at, Some(new_last_used));
}
