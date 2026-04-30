// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rusqlite::Connection;
use zk_auth::{
    password::{HashParams, hash_password},
    session::{hash_csrf_token, hash_session_id},
    token::hash_device_token,
};
use zk_domain::AuthScope;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::{DeviceTokenRepo, UserRepo, WebSessionInsert, WebSessionRepo},
    service::{
        CreateSessionSpec, IssueTokenSpec, RotateTokenSpec, SessionLifetime, create_web_session,
        issue_device_token, refresh_web_session, register_user, revoke_device_token,
        revoke_web_session, rotate_device_token, sweep_expired_sessions, verify_password,
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
    create_user_fixture_with_params(connection, user_id, username, password, TEST_HASH_PARAMS);
}

fn create_user_fixture_with_params(
    connection: &Connection,
    user_id: &str,
    username: &str,
    password: &str,
    hash_params: HashParams,
) {
    let password_hash =
        hash_password(password, hash_params).expect("fixture password hash should succeed");
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

fn create_session_spec(user_id: &str, lifetime: SessionLifetime) -> CreateSessionSpec {
    CreateSessionSpec {
        user_id: user_id.to_string(),
        scopes: vec![AuthScope::Admin],
        lifetime,
    }
}

fn short_session_lifetime() -> SessionLifetime {
    SessionLifetime {
        max_age: ChronoDuration::minutes(30),
    }
}

fn web_session_count(connection: &Connection) -> u32 {
    connection
        .query_row("SELECT COUNT(*) FROM web_sessions", [], |row| row.get(0))
        .expect("web session count should succeed")
}

fn insert_web_session_fixture(
    connection: &Connection,
    session_hash: &str,
    user_id: &str,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
) {
    create_user_fixture(connection, user_id, user_id, "correct horse battery");
    WebSessionRepo::create(
        connection,
        &WebSessionInsert {
            session_id_sha256_hex: session_hash.to_string(),
            user_id: user_id.to_string(),
            csrf_token_sha256_hex:
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            scopes: vec![AuthScope::Admin],
            created_at: parse_timestamp("2026-04-29T12:00:00Z"),
            expires_at,
        },
    )
    .expect("web session fixture insert should succeed");

    if let Some(revoked_at) = revoked_at {
        WebSessionRepo::revoke(connection, session_hash, revoked_at)
            .expect("web session fixture revoke should succeed");
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
fn verify_password_timing_does_not_leak_user_existence() {
    let connection = setup_connection();
    create_user_fixture_with_params(
        &connection,
        "user-1",
        "alice",
        "correct horse battery",
        HashParams::default(),
    );

    assert!(matches!(
        verify_password(&connection, "alice", "wrong password"),
        Err(ServerError::Unauthorized)
    ));
    assert!(matches!(
        verify_password(&connection, "missing", "wrong password"),
        Err(ServerError::Unauthorized)
    ));

    let wrong_timings = verify_timings(&connection, "alice");
    let missing_timings = verify_timings(&connection, "missing");
    let wrong_median = median_duration(wrong_timings);
    let missing_median = median_duration(missing_timings);
    let tolerance = wrong_median.max(missing_median).mul_f64(0.30);

    // This is a cargo-test-grade timing guard, not a benchmark. A 30% band
    // catches the missing-user fast path while allowing shared-runner noise.
    assert!(
        wrong_median.abs_diff(missing_median) <= tolerance,
        "verify timing differs too much: wrong={wrong_median:?}, missing={missing_median:?}, tolerance={tolerance:?}"
    );
}

fn verify_timings(connection: &Connection, username: &str) -> [Duration; 5] {
    std::array::from_fn(|_| {
        let started_at = Instant::now();
        let result = verify_password(connection, username, "wrong password");
        let elapsed = started_at.elapsed();

        assert!(matches!(result, Err(ServerError::Unauthorized)));

        elapsed
    })
}

fn median_duration(mut durations: [Duration; 5]) -> Duration {
    durations.sort();
    durations[durations.len() / 2]
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
fn rotate_with_unknown_device_id_rolls_back_revoke() {
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
    .expect_err("unknown device id should roll back rotation");
    let original = DeviceTokenRepo::lookup_by_hash(&connection, &old_hash)
        .expect("original lookup should succeed")
        .expect("original token should remain active");
    let active_tokens =
        DeviceTokenRepo::list_all_active(&connection).expect("active token list should succeed");

    assert!(matches!(error, ServerError::NotFound(message) if message == "device id"));
    assert_eq!(original.token_id, issued.token_id);
    assert_eq!(active_tokens.len(), 1);
    assert_eq!(active_tokens[0].token_id, issued.token_id);
}

#[test]
fn create_session_returns_distinct_plaintexts_each_call() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    let now = parse_timestamp("2026-04-29T12:00:00Z");
    let spec = create_session_spec("user-1", short_session_lifetime());

    let first =
        create_web_session(&connection, &spec, now).expect("first web session should be created");
    let second =
        create_web_session(&connection, &spec, now).expect("second web session should be created");

    assert_ne!(first.plaintext_session_id, second.plaintext_session_id);
    assert_ne!(first.plaintext_csrf_token, second.plaintext_csrf_token);
    assert_eq!(first.plaintext_session_id.len(), 52);
    assert_eq!(first.plaintext_csrf_token.len(), 52);
    assert_eq!(first.expires_at, now + short_session_lifetime().max_age);
    assert_eq!(web_session_count(&connection), 2);
}

#[test]
fn create_session_writes_only_hashes_to_db() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    let now = parse_timestamp("2026-04-29T12:00:00Z");
    let spec = create_session_spec("user-1", short_session_lifetime());

    let issued =
        create_web_session(&connection, &spec, now).expect("web session should be created");
    let session_hash = hash_session_id(&issued.plaintext_session_id);
    let csrf_hash = hash_csrf_token(&issued.plaintext_csrf_token);
    let row = WebSessionRepo::get_by_session_hash(&connection, &session_hash, now)
        .expect("session lookup should succeed")
        .expect("session row should exist");

    assert_eq!(row.session_id_sha256_hex, session_hash);
    assert_eq!(row.csrf_token_sha256_hex, csrf_hash);
    assert_ne!(row.session_id_sha256_hex, issued.plaintext_session_id);
    assert_ne!(row.csrf_token_sha256_hex, issued.plaintext_csrf_token);
    assert_eq!(row.user_id, "user-1");
    assert_eq!(row.scopes, vec![AuthScope::Admin]);
    assert_eq!(row.expires_at, issued.expires_at);
}

#[test]
fn issued_session_debug_redacts_plaintexts() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    let issued = create_web_session(
        &connection,
        &create_session_spec("user-1", short_session_lifetime()),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("web session should be created");

    let debug = format!("{issued:?}");

    assert!(debug.contains("plaintext_session_id: <REDACTED>"));
    assert!(debug.contains("plaintext_csrf_token: <REDACTED>"));
    assert!(!debug.contains(&issued.plaintext_session_id));
    assert!(!debug.contains(&issued.plaintext_csrf_token));
}

#[test]
fn refresh_rotates_session_id_and_csrf() {
    let mut connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    let created_at = parse_timestamp("2026-04-29T12:00:00Z");
    let refreshed_at = parse_timestamp("2026-04-29T12:10:00Z");
    let spec = create_session_spec("user-1", short_session_lifetime());
    let issued =
        create_web_session(&connection, &spec, created_at).expect("web session should be created");
    let old_hash = hash_session_id(&issued.plaintext_session_id);

    let refreshed = refresh_web_session(
        &mut connection,
        &issued.plaintext_session_id,
        refreshed_at,
        short_session_lifetime(),
    )
    .expect("web session should refresh");
    let old_lookup = WebSessionRepo::get_by_session_hash(&connection, &old_hash, refreshed_at)
        .expect("old session lookup should succeed");
    let new_hash = hash_session_id(&refreshed.plaintext_session_id);
    let new_row = WebSessionRepo::get_by_session_hash(&connection, &new_hash, refreshed_at)
        .expect("new session lookup should succeed")
        .expect("new session should be active");

    assert!(old_lookup.is_none());
    assert_ne!(refreshed.plaintext_session_id, issued.plaintext_session_id);
    assert_ne!(refreshed.plaintext_csrf_token, issued.plaintext_csrf_token);
    assert_eq!(
        refreshed.expires_at,
        refreshed_at + short_session_lifetime().max_age
    );
    assert_eq!(new_row.user_id, "user-1");
    assert_eq!(new_row.scopes, vec![AuthScope::Admin]);
}

#[test]
fn refresh_with_revoked_session_returns_unauthorized() {
    let mut connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    let issued = create_web_session(
        &connection,
        &create_session_spec("user-1", short_session_lifetime()),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("web session should be created");
    revoke_web_session(
        &connection,
        &issued.plaintext_session_id,
        parse_timestamp("2026-04-29T12:05:00Z"),
    )
    .expect("revoke should succeed");

    let error = refresh_web_session(
        &mut connection,
        &issued.plaintext_session_id,
        parse_timestamp("2026-04-29T12:10:00Z"),
        short_session_lifetime(),
    )
    .expect_err("revoked session should not refresh");

    assert!(matches!(error, ServerError::Unauthorized));
}

#[test]
fn refresh_with_expired_session_returns_unauthorized() {
    let mut connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    let lifetime = SessionLifetime {
        max_age: ChronoDuration::minutes(1),
    };
    let issued = create_web_session(
        &connection,
        &create_session_spec("user-1", lifetime),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("web session should be created");

    let error = refresh_web_session(
        &mut connection,
        &issued.plaintext_session_id,
        parse_timestamp("2026-04-29T12:02:00Z"),
        short_session_lifetime(),
    )
    .expect_err("expired session should not refresh");

    assert!(matches!(error, ServerError::Unauthorized));
}

#[test]
fn revoke_web_session_is_idempotent() {
    let connection = setup_connection();
    create_user_fixture(&connection, "user-1", "alice", "correct horse battery");
    let issued = create_web_session(
        &connection,
        &create_session_spec("user-1", short_session_lifetime()),
        parse_timestamp("2026-04-29T12:00:00Z"),
    )
    .expect("web session should be created");

    let first = revoke_web_session(
        &connection,
        &issued.plaintext_session_id,
        parse_timestamp("2026-04-29T12:05:00Z"),
    )
    .expect("first revoke should succeed");
    let second = revoke_web_session(
        &connection,
        &issued.plaintext_session_id,
        parse_timestamp("2026-04-29T12:10:00Z"),
    )
    .expect("second revoke should succeed");

    assert!(first);
    assert!(!second);
}

#[test]
fn sweep_returns_count_of_pruned_rows() {
    let connection = setup_connection();
    let now = parse_timestamp("2026-04-29T12:00:00Z");
    insert_web_session_fixture(
        &connection,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "user-a",
        now - ChronoDuration::hours(48),
        None,
    );
    insert_web_session_fixture(
        &connection,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "user-b",
        now - ChronoDuration::hours(2),
        Some(now - ChronoDuration::hours(1)),
    );
    insert_web_session_fixture(
        &connection,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "user-c",
        now + ChronoDuration::hours(1),
        None,
    );

    let deleted = sweep_expired_sessions(&connection, now).expect("session sweep should succeed");

    assert_eq!(deleted, 2);
    assert_eq!(web_session_count(&connection), 1);
}
