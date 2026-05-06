// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]
#![cfg(feature = "test-helpers")]

use rusqlite::Connection;
use zk_server::{
    ServerError,
    migration::ensure_schema,
    repo::UserRepo,
    service::{register_user_with_test_params, verify_password},
};

fn setup_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

#[test]
fn helper_registers_user_that_password_verification_accepts() {
    let connection = setup_connection();

    let registered = register_user_with_test_params(&connection, "alice", "correct horse battery")
        .expect("test helper registration should succeed");
    let verified = verify_password(&connection, "alice", "correct horse battery")
        .expect("registered password should verify");

    assert_eq!(registered.username, "alice");
    assert_eq!(verified.user_id, registered.user_id);
}

#[test]
fn helper_preserves_registration_validation() {
    let connection = setup_connection();

    let error = register_user_with_test_params(&connection, "alice", "short")
        .expect_err("short password should still fail");

    assert!(matches!(error, ServerError::InvalidRequest(message) if message == "password length"));
}

#[test]
fn helper_persists_argon2_hash_not_plaintext() {
    let connection = setup_connection();

    let registered = register_user_with_test_params(&connection, "alice", "correct horse battery")
        .expect("test helper registration should succeed");
    let row = UserRepo::get_password_hash_for_login(&connection, "alice")
        .expect("login lookup should succeed")
        .expect("user should exist");

    assert_eq!(row.user_id, registered.user_id);
    assert_ne!(row.password_hash, "correct horse battery");
    assert!(row.password_hash.contains("m=8192,t=2,p=1"));
}
