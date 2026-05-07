// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf, ptr};

use chrono::{Duration as ChronoDuration, Utc};
use rusqlite::Connection;
use tempfile::NamedTempFile;
use zk_domain::AuthScope;
use zk_server::{
    ServerError,
    config::ServerConfig,
    migration::ensure_schema,
    repo::{WebSessionInsert, WebSessionRepo},
    state::bootstrap_with_config,
};

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        vault_path: PathBuf::from("."),
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
        ai: None,
    }
}

fn insert_user(connection: &Connection, user_id: &str) {
    connection
        .execute(
            r"
INSERT INTO users (
    user_id,
    username,
    password_hash,
    created_at
) VALUES (
    ?1,
    ?2,
    '$argon2id$v=19$m=19456,t=2,p=1$fixture$fixturehash',
    '2026-04-29T12:00:00Z'
)
",
            rusqlite::params![user_id, user_id],
        )
        .expect("user fixture insert should succeed");
}

fn insert_session(connection: &Connection, session_hash: &str, expires_hours_from_now: i64) {
    let now = Utc::now();
    let user_id = format!("user-{session_hash}");
    insert_user(connection, &user_id);
    WebSessionRepo::create(
        connection,
        &WebSessionInsert {
            session_id_sha256_hex: session_hash.to_string(),
            user_id,
            csrf_token_sha256_hex:
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            scopes: vec![AuthScope::Admin],
            created_at: now - ChronoDuration::hours(72),
            expires_at: now + ChronoDuration::hours(expires_hours_from_now),
        },
    )
    .expect("session fixture insert should succeed");
}

fn session_count(connection: &Connection) -> Result<u32, ServerError> {
    connection
        .query_row("SELECT COUNT(*) FROM web_sessions", [], |row| row.get(0))
        .map_err(ServerError::Database)
}

#[test]
fn bootstrap_with_config_succeeds_and_exposes_injected_config_and_db() {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let config = test_config(database_file.path().to_path_buf());
    let before_bootstrap = Utc::now();

    let state =
        bootstrap_with_config(config.clone()).expect("bootstrap_with_config should succeed");

    assert_eq!(state.config(), &config);
    assert_eq!(
        state
            .db()
            .schema_version()
            .expect("schema lookup should succeed"),
        3
    );
    assert!(state.started_at() >= before_bootstrap);
    assert!(state.started_at() <= Utc::now());
}

#[test]
fn clones_share_same_underlying_state() {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let state = bootstrap_with_config(test_config(database_file.path().to_path_buf()))
        .expect("bootstrap_with_config should succeed");
    let cloned_state = state.clone();

    assert!(ptr::eq(state.config(), cloned_state.config()));
    assert!(ptr::eq(state.db(), cloned_state.db()));
}

#[test]
fn bootstrap_with_config_sweeps_expired_sessions_once() {
    let database_file = NamedTempFile::new().expect("temp database file should be created");
    let database_path = database_file.path().to_path_buf();
    {
        let connection = Connection::open(&database_path).expect("database should open");
        connection
            .pragma_update(None, "foreign_keys", true)
            .expect("foreign keys pragma should be enabled");
        ensure_schema(&connection).expect("schema migration should succeed");
        insert_session(
            &connection,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            -48,
        );
        insert_session(
            &connection,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
        );
        WebSessionRepo::revoke(
            &connection,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Utc::now() - ChronoDuration::hours(47),
        )
        .expect("session revoke should succeed");
        assert_eq!(session_count(&connection).expect("count should succeed"), 2);
    }

    let state = bootstrap_with_config(test_config(database_path))
        .expect("bootstrap_with_config should succeed");
    let remaining = state
        .db()
        .with_conn(session_count)
        .expect("session count should succeed");

    assert_eq!(remaining, 1);
}
