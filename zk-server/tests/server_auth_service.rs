// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_auth::password::{HashParams, hash_password};
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::UserRepo,
    service::{register_user, verify_password},
};

const TEST_HASH_PARAMS: HashParams = HashParams {
    memory_kib: 8,
    iterations: 1,
    parallelism: 1,
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

fn create_user_fixture(connection: &Connection, user_id: &str, username: &str, password: &str) {
    let password_hash =
        hash_password(password, TEST_HASH_PARAMS).expect("fixture password hash should succeed");
    UserRepo::create(
        connection,
        user_id,
        username,
        &password_hash,
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("user fixture insert should succeed");
}

#[test]
fn register_then_verify_succeeds() {
    let connection = setup_connection();
    let now = parse_timestamp("2026-04-29T12:00:00Z");

    let registered = register_user(&connection, "alice_1", "correct horse battery", now)
        .expect("user registration should succeed");
    let verified = verify_password(&connection, "alice_1", "correct horse battery")
        .expect("password verification should succeed");
    let login_row = UserRepo::get_password_hash_for_login(&connection, "alice_1")
        .expect("login lookup should succeed")
        .expect("registered user should exist");

    assert_eq!(registered.username, "alice_1");
    assert_eq!(registered.user_id.len(), 26);
    assert_eq!(verified.user_id, registered.user_id);
    assert_eq!(login_row.user_id, registered.user_id);
    assert_ne!(login_row.password_hash, "correct horse battery");
    assert!(login_row.password_hash.starts_with("$argon2id$"));
}

#[test]
fn register_with_short_password_returns_invalid_request() {
    let connection = setup_connection();

    let error = register_user(
        &connection,
        "alice",
        "short",
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect_err("short password should fail");

    assert!(matches!(error, ServerError::InvalidRequest(message) if message == "password length"));
}

#[test]
fn register_with_invalid_username_returns_invalid_request() {
    let connection = setup_connection();
    let too_long = "a".repeat(65);
    let cases = ["", "bad.name", "bad name", too_long.as_str()];

    for username in cases {
        let error = register_user(
            &connection,
            username,
            "correct horse battery",
            parse_timestamp("2026-04-29T12:00:00Z"),
        )
        .expect_err("invalid username should fail");

        assert!(
            matches!(error, ServerError::InvalidRequest(message) if message == "username format")
        );
    }
}

#[test]
fn register_with_duplicate_username_returns_conflict() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "existing password");

    let error = register_user(
        &connection,
        "alice",
        "correct horse battery",
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect_err("duplicate username should fail");

    assert!(
        matches!(error, ServerError::Conflict(message) if message == "username already exists")
    );
}

#[test]
fn verify_with_wrong_password_returns_unauthorized() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");

    let error = verify_password(&connection, "alice", "wrong password")
        .expect_err("wrong password should fail");

    assert!(matches!(error, ServerError::Unauthorized));
}

#[test]
fn verify_with_missing_user_returns_unauthorized() {
    let connection = setup_connection();

    let error = verify_password(&connection, "missing", "correct horse battery")
        .expect_err("missing user should fail");

    assert!(matches!(error, ServerError::Unauthorized));
}

#[test]
fn verify_with_disabled_user_returns_unauthorized() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    UserRepo::disable(
        &connection,
        "user-1",
        parse_timestamp("2026-04-30T12:00:00Z"),
    )
    .expect("disable should succeed");

    let error = verify_password(&connection, "alice", "correct horse battery")
        .expect_err("disabled user should fail");

    assert!(matches!(error, ServerError::Unauthorized));
}

#[test]
#[ignore = "timing-sensitive characterization; run manually on an idle machine"]
fn verify_timing_for_missing_and_wrong_are_within_tolerance() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");

    let wrong_start = Instant::now();
    let wrong = verify_password(&connection, "alice", "wrong password");
    let wrong_elapsed = wrong_start.elapsed();

    thread::sleep(Duration::from_millis(20));

    let missing_start = Instant::now();
    let missing = verify_password(&connection, "missing", "wrong password");
    let missing_elapsed = missing_start.elapsed();

    assert!(matches!(wrong, Err(ServerError::Unauthorized)));
    assert!(matches!(missing, Err(ServerError::Unauthorized)));
    assert!(missing_elapsed >= Duration::from_millis(45));
    assert!(wrong_elapsed.abs_diff(missing_elapsed) <= Duration::from_millis(80));
}
