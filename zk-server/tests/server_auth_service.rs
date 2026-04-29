// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_auth::{
    password::{HashParams, hash_password},
    token::hash_device_token,
};
use zk_domain::AuthScope;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{DeviceTokenRepo, UserRepo},
    service::{
        IssueTokenSpec, RotateTokenSpec, issue_device_token, register_user, revoke_device_token,
        rotate_device_token, verify_password,
    },
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

fn issue_token_spec(label: &str) -> IssueTokenSpec {
    IssueTokenSpec {
        label: label.to_string(),
        device_id: None,
        scopes: vec![AuthScope::Sync],
    }
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

#[test]
fn issue_returns_unique_plaintexts() {
    let connection = setup_connection();
    let now = parse_timestamp("2026-04-29T12:00:00Z");

    let first = issue_device_token(&connection, &issue_token_spec("Laptop"), now)
        .expect("first token issue should succeed");
    let second = issue_device_token(&connection, &issue_token_spec("Phone"), now)
        .expect("second token issue should succeed");
    let first_row =
        DeviceTokenRepo::lookup_by_hash(&connection, &hash_device_token(&first.plaintext))
            .expect("first token lookup should succeed")
            .expect("first token should be persisted");
    let second_row =
        DeviceTokenRepo::lookup_by_hash(&connection, &hash_device_token(&second.plaintext))
            .expect("second token lookup should succeed")
            .expect("second token should be persisted");

    assert_ne!(first.token_id, second.token_id);
    assert_ne!(first.plaintext, second.plaintext);
    assert_eq!(first.plaintext.len(), 52);
    assert_eq!(second.plaintext.len(), 52);
    assert_eq!(first.scopes, vec![AuthScope::Sync]);
    assert_eq!(first.created_at, now);
    assert_eq!(first_row.token_id, first.token_id);
    assert_eq!(second_row.token_id, second.token_id);
    assert_eq!(
        first_row.token_sha256_hex,
        hash_device_token(&first.plaintext)
    );
    assert_ne!(first_row.token_sha256_hex, first.plaintext);
}

#[test]
fn issue_with_empty_scopes_returns_invalid_request() {
    let connection = setup_connection();
    let spec = IssueTokenSpec {
        label: "Laptop".to_string(),
        device_id: None,
        scopes: Vec::new(),
    };

    let error = issue_device_token(&connection, &spec, parse_timestamp("2026-04-29T12:00:00Z"))
        .expect_err("empty scopes should fail");

    assert!(
        matches!(error, ServerError::InvalidRequest(message) if message == "scopes must not be empty")
    );
}

#[test]
fn issue_with_invalid_label_returns_invalid_request() {
    let connection = setup_connection();
    let too_long = "a".repeat(129);
    let cases = ["", too_long.as_str()];

    for label in cases {
        let error = issue_device_token(
            &connection,
            &issue_token_spec(label),
            parse_timestamp("2026-04-29T12:00:00Z"),
        )
        .expect_err("invalid label should fail");

        assert!(
            matches!(error, ServerError::InvalidRequest(message) if message == "token label format")
        );
    }
}

#[test]
fn issued_token_debug_redacts_plaintext() {
    let connection = setup_connection();

    let issued = issue_device_token(
        &connection,
        &issue_token_spec("Laptop"),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("token issue should succeed");
    let debug = format!("{issued:?}");

    assert!(debug.contains("plaintext: <REDACTED>"));
    assert!(!debug.contains(&issued.plaintext));
}

#[test]
fn revoke_succeeds_then_lookup_by_hash_returns_none() {
    let connection = setup_connection();
    let issued = issue_device_token(
        &connection,
        &issue_token_spec("Laptop"),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("token issue should succeed");
    let token_hash = hash_device_token(&issued.plaintext);

    let revoked = revoke_device_token(
        &connection,
        &issued.token_id,
        parse_timestamp("2026-04-29T13:00:00Z"),
    )
    .expect("revoke should succeed");
    let loaded =
        DeviceTokenRepo::lookup_by_hash(&connection, &token_hash).expect("lookup should succeed");

    assert!(revoked);
    assert!(loaded.is_none());
}

#[test]
fn revoke_idempotent_second_call_returns_false() {
    let connection = setup_connection();
    let issued = issue_device_token(
        &connection,
        &issue_token_spec("Laptop"),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("token issue should succeed");

    let first = revoke_device_token(
        &connection,
        &issued.token_id,
        parse_timestamp("2026-04-29T13:00:00Z"),
    )
    .expect("first revoke should succeed");
    let second = revoke_device_token(
        &connection,
        &issued.token_id,
        parse_timestamp("2026-04-29T14:00:00Z"),
    )
    .expect("second revoke should succeed");

    assert!(first);
    assert!(!second);
}

#[test]
fn rotate_revokes_old_and_issues_new_atomically() {
    let mut connection = setup_connection();
    let issued = issue_device_token(
        &connection,
        &issue_token_spec("Laptop"),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("token issue should succeed");
    let old_hash = hash_device_token(&issued.plaintext);
    let spec = RotateTokenSpec {
        current_token_id: issued.token_id.clone(),
        label: "Laptop replacement".to_string(),
        device_id: None,
        scopes: vec![AuthScope::Sync, AuthScope::Admin],
    };

    let rotated = rotate_device_token(
        &mut connection,
        &spec,
        parse_timestamp("2026-04-29T13:00:00Z"),
    )
    .expect("rotation should succeed");
    let old_lookup =
        DeviceTokenRepo::lookup_by_hash(&connection, &old_hash).expect("old lookup should succeed");
    let new_lookup =
        DeviceTokenRepo::lookup_by_hash(&connection, &hash_device_token(&rotated.plaintext))
            .expect("new lookup should succeed")
            .expect("new token should be active");

    assert!(old_lookup.is_none());
    assert_ne!(rotated.token_id, issued.token_id);
    assert_eq!(rotated.scopes, vec![AuthScope::Sync, AuthScope::Admin]);
    assert_eq!(new_lookup.token_id, rotated.token_id);
}

#[test]
fn rotate_with_unknown_token_id_returns_not_found_and_does_not_issue() {
    let mut connection = setup_connection();
    let spec = RotateTokenSpec {
        current_token_id: "missing-token".to_string(),
        label: "Replacement".to_string(),
        device_id: None,
        scopes: vec![AuthScope::Sync],
    };

    let error = rotate_device_token(
        &mut connection,
        &spec,
        parse_timestamp("2026-04-29T13:00:00Z"),
    )
    .expect_err("unknown token rotation should fail");
    let active_tokens =
        DeviceTokenRepo::list_all_active(&connection).expect("active token list should succeed");

    assert!(matches!(error, ServerError::NotFound(message) if message == "token id"));
    assert!(active_tokens.is_empty());
}

#[test]
fn rotate_failure_in_issue_rolls_back_revoke() {
    let mut connection = setup_connection();
    let issued = issue_device_token(
        &connection,
        &issue_token_spec("Laptop"),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("token issue should succeed");
    let old_hash = hash_device_token(&issued.plaintext);
    let spec = RotateTokenSpec {
        current_token_id: issued.token_id.clone(),
        label: "Replacement".to_string(),
        device_id: Some("missing-device".to_string()),
        scopes: vec![AuthScope::Sync],
    };

    let error = rotate_device_token(
        &mut connection,
        &spec,
        parse_timestamp("2026-04-29T13:00:00Z"),
    )
    .expect_err("foreign-key failure should roll back rotation");
    let original = DeviceTokenRepo::lookup_by_hash(&connection, &old_hash)
        .expect("original lookup should succeed")
        .expect("original token should remain active");
    let active_tokens =
        DeviceTokenRepo::list_all_active(&connection).expect("active token list should succeed");

    assert!(matches!(error, ServerError::Database(_)));
    assert_eq!(original.token_id, issued.token_id);
    assert_eq!(active_tokens.len(), 1);
    assert_eq!(active_tokens[0].token_id, issued.token_id);
}
