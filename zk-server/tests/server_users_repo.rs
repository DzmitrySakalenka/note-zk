// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{UserRepo, UserRow},
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

fn create_user(
    connection: &Connection,
    user_id: &str,
    username: &str,
    password_hash: &str,
    created_at: &str,
) {
    UserRepo::create(
        connection,
        user_id,
        username,
        password_hash,
        parse_timestamp(created_at),
    )
    .expect("user create should succeed");
}

struct RawUserRow {
    user_id: String,
    username: String,
    password_hash: String,
    created_at: DateTime<Utc>,
    disabled_at: Option<DateTime<Utc>>,
}

fn load_raw_user(connection: &Connection, user_id: &str) -> RawUserRow {
    connection
        .query_row(
            "SELECT user_id, username, password_hash, created_at, disabled_at
             FROM users
             WHERE user_id = ?1",
            [user_id],
            |row| {
                let created_at = DateTime::parse_from_rfc3339(&row.get::<_, String>(3)?)
                    .expect("created_at fixture should parse")
                    .with_timezone(&Utc);
                let disabled_at = row.get::<_, Option<String>>(4)?.map(|value| {
                    DateTime::parse_from_rfc3339(&value)
                        .expect("disabled_at fixture should parse")
                        .with_timezone(&Utc)
                });

                Ok(RawUserRow {
                    user_id: row.get(0)?,
                    username: row.get(1)?,
                    password_hash: row.get(2)?,
                    created_at,
                    disabled_at,
                })
            },
        )
        .expect("raw user lookup should succeed")
}

fn assert_public_user_row(
    row: &UserRow,
    user_id: &str,
    username: &str,
    created_at: &str,
    disabled_at: Option<&str>,
) {
    assert_eq!(row.user_id, user_id);
    assert_eq!(row.username, username);
    assert_eq!(row.created_at, parse_timestamp(created_at));
    assert_eq!(row.disabled_at, disabled_at.map(parse_timestamp));
}

#[test]
fn create_then_get_by_id_round_trips() {
    let connection = setup_connection();

    create_user(
        &connection,
        "user-1",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash",
        "2026-04-29T12:00:00.123Z",
    );

    let loaded = UserRepo::get_by_id(&connection, "user-1")
        .expect("user lookup should succeed")
        .expect("user row should exist");

    assert_public_user_row(&loaded, "user-1", "alice", "2026-04-29T12:00:00.123Z", None);
}

#[test]
fn create_with_duplicate_username_returns_conflict() {
    let connection = setup_connection();
    create_user(
        &connection,
        "user-1",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash-1",
        "2026-04-29T12:00:00Z",
    );

    let error = UserRepo::create(
        &connection,
        "user-2",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash-2",
        parse_timestamp("2026-04-29T12:01:00Z"),
    )
    .expect_err("duplicate username should fail");

    assert!(
        matches!(error, ServerError::Conflict(message) if message == "username already exists")
    );
}

#[test]
fn get_password_hash_for_missing_username_returns_none() {
    let connection = setup_connection();

    let loaded = UserRepo::get_password_hash_for_login(&connection, "missing")
        .expect("missing login lookup should succeed");

    assert!(loaded.is_none());
}

#[test]
fn get_password_hash_for_disabled_user_still_returns_row() {
    let connection = setup_connection();
    create_user(
        &connection,
        "user-1",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash",
        "2026-04-29T12:00:00Z",
    );
    let disabled_at = parse_timestamp("2026-04-30T08:00:00Z");
    UserRepo::disable(&connection, "user-1", disabled_at).expect("disable should succeed");

    let loaded = UserRepo::get_password_hash_for_login(&connection, "alice")
        .expect("disabled login lookup should succeed")
        .expect("disabled user should still be returned");

    assert_eq!(loaded.user_id, "user-1");
    assert_eq!(
        loaded.password_hash,
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash"
    );
    assert_eq!(loaded.disabled_at, Some(disabled_at));
}

#[test]
fn update_password_hash_changes_only_that_column() {
    let connection = setup_connection();
    create_user(
        &connection,
        "user-1",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$old",
        "2026-04-29T12:00:00Z",
    );
    let before = load_raw_user(&connection, "user-1");

    UserRepo::update_password_hash(
        &connection,
        "user-1",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$new",
    )
    .expect("password hash update should succeed");

    let after = load_raw_user(&connection, "user-1");

    assert_eq!(after.user_id, before.user_id);
    assert_eq!(after.username, before.username);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.disabled_at, before.disabled_at);
    assert_eq!(
        after.password_hash,
        "$argon2id$v=19$m=19456,t=2,p=1$salt$new"
    );
}

#[test]
fn disable_sets_disabled_at_only() {
    let connection = setup_connection();
    create_user(
        &connection,
        "user-1",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash",
        "2026-04-29T12:00:00Z",
    );
    let before = load_raw_user(&connection, "user-1");
    let disabled_at = parse_timestamp("2026-04-30T08:00:00Z");

    UserRepo::disable(&connection, "user-1", disabled_at).expect("disable should succeed");

    let after = load_raw_user(&connection, "user-1");

    assert_eq!(after.user_id, before.user_id);
    assert_eq!(after.username, before.username);
    assert_eq!(after.password_hash, before.password_hash);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.disabled_at, Some(disabled_at));
}

#[test]
fn count_reflects_inserts_and_disabled_users_still_count() {
    let connection = setup_connection();

    assert_eq!(
        UserRepo::count(&connection).expect("count should succeed"),
        0
    );

    create_user(
        &connection,
        "user-1",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash-1",
        "2026-04-29T12:00:00Z",
    );
    create_user(
        &connection,
        "user-2",
        "bob",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash-2",
        "2026-04-29T12:01:00Z",
    );

    assert_eq!(
        UserRepo::count(&connection).expect("count should succeed"),
        2
    );

    UserRepo::disable(
        &connection,
        "user-1",
        parse_timestamp("2026-04-30T08:00:00Z"),
    )
    .expect("disable should succeed");

    assert_eq!(
        UserRepo::count(&connection).expect("count should succeed"),
        2
    );
}

#[test]
fn list_returns_all_rows_including_disabled() {
    let connection = setup_connection();
    create_user(
        &connection,
        "user-b",
        "bob",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash-b",
        "2026-04-29T12:01:00Z",
    );
    create_user(
        &connection,
        "user-a",
        "alice",
        "$argon2id$v=19$m=19456,t=2,p=1$salt$hash-a",
        "2026-04-29T12:00:00Z",
    );
    UserRepo::disable(
        &connection,
        "user-a",
        parse_timestamp("2026-04-30T08:00:00Z"),
    )
    .expect("disable should succeed");

    let rows = UserRepo::list(&connection).expect("list should succeed");

    assert_eq!(rows.len(), 2);
    assert_public_user_row(
        &rows[0],
        "user-a",
        "alice",
        "2026-04-29T12:00:00Z",
        Some("2026-04-30T08:00:00Z"),
    );
    assert_public_user_row(&rows[1], "user-b", "bob", "2026-04-29T12:01:00Z", None);
}
