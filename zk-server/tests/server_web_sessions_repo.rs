// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::AuthScope;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{WebSessionInsert, WebSessionRepo, WebSessionRow},
};

const INSERT_USER_SQL: &str = r"
INSERT OR IGNORE INTO users (
    user_id,
    username,
    password_hash,
    created_at
) VALUES (
    ?1, ?2, '$argon2id$v=19$m=19456,t=2,p=1$fixture$fixturehash', '2026-04-24T09:00:00+00:00'
)
";
const INSERT_SESSION_SQL: &str = r"
INSERT INTO web_sessions (
    session_id_sha256_hex,
    user_id,
    csrf_token_sha256_hex,
    scopes_json,
    created_at,
    expires_at,
    last_seen_at,
    revoked_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
)
";
const SELECT_SESSION_BY_HASH_SQL: &str = r"
SELECT
    session_id_sha256_hex,
    user_id,
    csrf_token_sha256_hex,
    scopes_json,
    created_at,
    expires_at,
    last_seen_at,
    revoked_at
FROM web_sessions
WHERE session_id_sha256_hex = ?1
LIMIT 1
";
const SELECT_SESSION_IDS_SQL: &str =
    "SELECT session_id_sha256_hex FROM web_sessions ORDER BY session_id_sha256_hex ASC";

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

struct SessionFixture<'a> {
    session_id_sha256_hex: &'a str,
    user_id: &'a str,
    csrf_token_sha256_hex: &'a str,
    scopes_json: &'a str,
    created_at: &'a str,
    expires_at: &'a str,
    last_seen_at: &'a str,
    revoked_at: Option<&'a str>,
}

fn insert_user(connection: &Connection, user_id: &str) {
    connection
        .execute(INSERT_USER_SQL, rusqlite::params![user_id, user_id])
        .expect("user fixture insert should succeed");
}

fn insert_session_row(connection: &Connection, fixture: &SessionFixture<'_>) {
    insert_user(connection, fixture.user_id);
    connection
        .execute(
            INSERT_SESSION_SQL,
            rusqlite::params![
                fixture.session_id_sha256_hex,
                fixture.user_id,
                fixture.csrf_token_sha256_hex,
                fixture.scopes_json,
                fixture.created_at,
                fixture.expires_at,
                fixture.last_seen_at,
                fixture.revoked_at,
            ],
        )
        .expect("web session fixture insert should succeed");
}

fn insert_fixture(
    connection: &Connection,
    session_id_sha256_hex: &'static str,
    user_id: &'static str,
    expires_at: &'static str,
    revoked_at: Option<&'static str>,
) {
    insert_session_row(
        connection,
        &SessionFixture {
            session_id_sha256_hex,
            user_id,
            csrf_token_sha256_hex: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            scopes_json: r#"["admin"]"#,
            created_at: "2026-04-24T10:00:00+00:00",
            expires_at,
            last_seen_at: "2026-04-24T10:00:00+00:00",
            revoked_at,
        },
    );
}

fn insert_row(connection: &Connection, row: &WebSessionInsert) {
    insert_user(connection, &row.user_id);
    WebSessionRepo::create(connection, row).expect("web session insert should succeed");
}

fn session_insert(
    session_id_sha256_hex: &str,
    user_id: &str,
    csrf_token_sha256_hex: &str,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> WebSessionInsert {
    WebSessionInsert {
        session_id_sha256_hex: session_id_sha256_hex.to_string(),
        user_id: user_id.to_string(),
        csrf_token_sha256_hex: csrf_token_sha256_hex.to_string(),
        scopes: vec![AuthScope::Admin, AuthScope::Read],
        created_at,
        expires_at,
    }
}

fn load_session_by_hash(connection: &Connection, session_id_sha256_hex: &str) -> WebSessionRow {
    connection
        .query_row(SELECT_SESSION_BY_HASH_SQL, [session_id_sha256_hex], |row| {
            let scopes = serde_json::from_str(&row.get::<_, String>(3)?)
                .expect("scopes fixture should parse");

            Ok(WebSessionRow {
                session_id_sha256_hex: row.get(0)?,
                user_id: row.get(1)?,
                csrf_token_sha256_hex: row.get(2)?,
                scopes,
                created_at: parse_timestamp(&row.get::<_, String>(4)?),
                expires_at: parse_timestamp(&row.get::<_, String>(5)?),
                last_seen_at: parse_timestamp(&row.get::<_, String>(6)?),
                revoked_at: row
                    .get::<_, Option<String>>(7)?
                    .map(|value| parse_timestamp(&value)),
            })
        })
        .expect("raw web session lookup should succeed")
}

fn session_ids(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare(SELECT_SESSION_IDS_SQL)
        .expect("session id query should prepare");
    let rows = statement
        .query_map([], |row| row.get(0))
        .expect("session id query should run");

    rows.collect::<Result<Vec<_>, _>>()
        .expect("session ids should decode")
}

#[test]
fn create_then_get_round_trips() {
    let connection = setup_connection();
    let created_at = parse_timestamp("2026-04-24T10:00:00+00:00");
    let expires_at = parse_timestamp("2026-04-24T11:00:00+00:00");
    let row = session_insert(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "user-a",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        created_at,
        expires_at,
    );

    insert_row(&connection, &row);

    let loaded = WebSessionRepo::get_by_session_hash(
        &connection,
        &row.session_id_sha256_hex,
        parse_timestamp("2026-04-24T10:30:00+00:00"),
    )
    .expect("session lookup should succeed")
    .expect("active session should be returned");

    assert_eq!(loaded.session_id_sha256_hex, row.session_id_sha256_hex);
    assert_eq!(loaded.user_id, row.user_id);
    assert_eq!(loaded.csrf_token_sha256_hex, row.csrf_token_sha256_hex);
    assert_eq!(loaded.scopes, row.scopes);
    assert_eq!(loaded.created_at, created_at);
    assert_eq!(loaded.expires_at, expires_at);
    assert_eq!(loaded.last_seen_at, created_at);
    assert_eq!(loaded.revoked_at, None);
}

#[test]
fn get_returns_none_for_revoked_session() {
    let connection = setup_connection();
    let session_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_fixture(
        &connection,
        session_hash,
        "user-a",
        "2026-04-24T11:00:00+00:00",
        Some("2026-04-24T10:30:00+00:00"),
    );

    let loaded = WebSessionRepo::get_by_session_hash(
        &connection,
        session_hash,
        parse_timestamp("2026-04-24T10:45:00+00:00"),
    )
    .expect("revoked session lookup should succeed");

    assert!(loaded.is_none());
}

#[test]
fn get_returns_none_for_expired_session() {
    let connection = setup_connection();
    let session_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_fixture(
        &connection,
        session_hash,
        "user-a",
        "2026-04-24T11:00:00+00:00",
        None,
    );

    let loaded = WebSessionRepo::get_by_session_hash(
        &connection,
        session_hash,
        parse_timestamp("2026-04-24T11:00:00+00:00"),
    )
    .expect("expired session lookup should succeed");

    assert!(loaded.is_none());
}

#[test]
fn refresh_last_seen_advances_only_that_column() {
    let connection = setup_connection();
    let session_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_fixture(
        &connection,
        session_hash,
        "user-a",
        "2026-04-24T11:00:00+00:00",
        None,
    );
    let before = load_session_by_hash(&connection, session_hash);
    let now = parse_timestamp("2026-04-24T10:30:00+00:00");

    WebSessionRepo::refresh_last_seen(&connection, session_hash, now)
        .expect("last_seen refresh should succeed");

    let after = load_session_by_hash(&connection, session_hash);
    assert_eq!(after.session_id_sha256_hex, before.session_id_sha256_hex);
    assert_eq!(after.user_id, before.user_id);
    assert_eq!(after.csrf_token_sha256_hex, before.csrf_token_sha256_hex);
    assert_eq!(after.scopes, before.scopes);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.expires_at, before.expires_at);
    assert_eq!(after.revoked_at, before.revoked_at);
    assert_eq!(after.last_seen_at, now);
}

#[test]
fn revoke_returns_true_for_first_call_false_for_second() {
    let connection = setup_connection();
    let session_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_fixture(
        &connection,
        session_hash,
        "user-a",
        "2026-04-24T11:00:00+00:00",
        None,
    );
    let revoked_at = parse_timestamp("2026-04-24T10:30:00+00:00");

    let first = WebSessionRepo::revoke(&connection, session_hash, revoked_at)
        .expect("first revoke should succeed");
    let second = WebSessionRepo::revoke(&connection, session_hash, revoked_at)
        .expect("second revoke should succeed");

    let loaded = load_session_by_hash(&connection, session_hash);
    assert!(first);
    assert!(!second);
    assert_eq!(loaded.revoked_at, Some(revoked_at));
}

#[test]
fn revoke_all_for_user_revokes_only_unrevoked_rows() {
    let connection = setup_connection();
    insert_fixture(
        &connection,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "user-a",
        "2026-04-24T11:00:00+00:00",
        None,
    );
    insert_fixture(
        &connection,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "user-a",
        "2026-04-24T11:00:00+00:00",
        None,
    );
    insert_fixture(
        &connection,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "user-a",
        "2026-04-24T11:00:00+00:00",
        Some("2026-04-24T10:15:00+00:00"),
    );
    insert_fixture(
        &connection,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "user-b",
        "2026-04-24T11:00:00+00:00",
        None,
    );
    let revoked_at = parse_timestamp("2026-04-24T10:30:00+00:00");

    let revoked = WebSessionRepo::revoke_all_for_user(&connection, "user-a", revoked_at)
        .expect("bulk revoke should succeed");

    assert_eq!(revoked, 2);
    assert_eq!(
        load_session_by_hash(
            &connection,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .revoked_at,
        Some(revoked_at)
    );
    assert_eq!(
        load_session_by_hash(
            &connection,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        )
        .revoked_at,
        Some(revoked_at)
    );
    assert_eq!(
        load_session_by_hash(
            &connection,
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        )
        .revoked_at,
        Some(parse_timestamp("2026-04-24T10:15:00+00:00"))
    );
    assert_eq!(
        load_session_by_hash(
            &connection,
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        )
        .revoked_at,
        None
    );
}

#[test]
fn sweep_expired_deletes_old_revoked_rows_keeps_recent() {
    let connection = setup_connection();
    let now = parse_timestamp("2026-04-25T12:00:00+00:00");
    insert_fixture(
        &connection,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "user-a",
        "2026-04-24T10:00:00+00:00",
        Some("2026-04-24T10:30:00+00:00"),
    );
    insert_fixture(
        &connection,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "user-a",
        "2026-04-25T11:00:00+00:00",
        Some("2026-04-25T11:15:00+00:00"),
    );
    insert_fixture(
        &connection,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "user-a",
        "2026-04-24T10:00:00+00:00",
        None,
    );
    insert_fixture(
        &connection,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "user-a",
        "2026-04-25T11:00:00+00:00",
        None,
    );
    insert_fixture(
        &connection,
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "user-a",
        "2026-04-25T13:00:00+00:00",
        Some("2026-04-25T11:30:00+00:00"),
    );

    let deleted = WebSessionRepo::sweep_expired(&connection, now)
        .expect("expired session sweep should succeed");

    assert_eq!(deleted, 3);
    assert_eq!(
        session_ids(&connection),
        vec![
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_string(),
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
        ]
    );
}

#[test]
fn sweep_expired_keeps_unexpired_unrevoked_rows() {
    let connection = setup_connection();
    let session_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_fixture(
        &connection,
        session_hash,
        "user-a",
        "2026-04-25T13:00:00+00:00",
        None,
    );

    let deleted =
        WebSessionRepo::sweep_expired(&connection, parse_timestamp("2026-04-25T12:00:00+00:00"))
            .expect("expired session sweep should succeed");

    assert_eq!(deleted, 0);
    assert_eq!(session_ids(&connection), vec![session_hash.to_string()]);
}

#[test]
fn create_duplicate_session_hash_returns_conflict() {
    let connection = setup_connection();
    let created_at = parse_timestamp("2026-04-24T10:00:00+00:00");
    let expires_at = parse_timestamp("2026-04-24T11:00:00+00:00");
    let first = session_insert(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "user-a",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        created_at,
        expires_at,
    );
    let duplicate = session_insert(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "user-a",
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        created_at,
        expires_at,
    );
    insert_row(&connection, &first);

    let error = WebSessionRepo::create(&connection, &duplicate)
        .expect_err("duplicate session hash should fail");

    assert!(matches!(error, ServerError::Conflict(message) if message == "session id collision"));
}

#[test]
fn decoder_rejects_malformed_scopes_json() {
    let connection = setup_connection();
    let session_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    insert_session_row(
        &connection,
        &SessionFixture {
            session_id_sha256_hex: session_hash,
            user_id: "user-a",
            csrf_token_sha256_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            scopes_json: "not-json",
            created_at: "2026-04-24T10:00:00+00:00",
            expires_at: "2026-04-24T11:00:00+00:00",
            last_seen_at: "2026-04-24T10:00:00+00:00",
            revoked_at: None,
        },
    );

    let error = WebSessionRepo::get_by_session_hash(
        &connection,
        session_hash,
        parse_timestamp("2026-04-24T10:30:00+00:00"),
    )
    .expect_err("malformed scopes JSON should fail row decoding");

    assert!(matches!(error, ServerError::Database(_)));
}
