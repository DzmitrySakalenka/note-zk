// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use rusqlite::Connection;
use zk_server::{db::ServerDb, repo::UserRepo};

struct Harness {
    root: PathBuf,
    database_path: PathBuf,
}

fn harness(name: &str) -> Harness {
    let root = common::unique_test_dir(name);
    let database_path = root.join("zk-server.sqlite3");

    Harness {
        root,
        database_path,
    }
}

fn command(harness: &Harness) -> Command {
    let mut command = Command::cargo_bin("zk-server").expect("zk-server binary should build");

    command
        .env("HOME", &harness.root)
        .env("APPDATA", &harness.root)
        .env("XDG_CONFIG_HOME", harness.root.join("xdg"))
        .env("ZK_SERVER_DATABASE_PATH", &harness.database_path)
        .env_remove("ZK_SERVER_CONFIG")
        .env_remove("ZK_SERVER_LISTEN_ADDR")
        .env_remove("ZK_SERVER_LOG_FILTER")
        .env_remove("ZK_SERVER_TOKEN_SHA256")
        .env_remove("ZK_SERVER_TRASH_RETENTION_DAYS")
        .env_remove("ZK_SERVER_MAX_BATCH_SIZE");

    command
}

fn create_user(
    harness: &Harness,
    username: &str,
    password: &str,
    force: bool,
) -> assert_cmd::assert::Assert {
    let mut command = command(harness);
    command
        .arg("admin")
        .arg("create-user")
        .arg("--username")
        .arg(username)
        .write_stdin(password);

    if force {
        command.arg("--force");
    }

    command.assert()
}

fn user_count(database_path: &Path) -> u32 {
    let database = ServerDb::open(database_path).expect("server database should open");

    database
        .with_conn(UserRepo::count)
        .expect("user count should succeed")
}

fn username_exists(database_path: &Path, username: &str) -> bool {
    let database = ServerDb::open(database_path).expect("server database should open");

    database
        .with_conn(|connection: &Connection| {
            UserRepo::get_password_hash_for_login(connection, username).map(|row| row.is_some())
        })
        .expect("user lookup should succeed")
}

#[test]
fn create_first_user_succeeds_with_piped_password() {
    let harness = harness("admin-create-first-user");

    let assert = create_user(&harness, "alice", "correct horse battery\n", false).success();
    let output = assert.get_output();

    assert!(String::from_utf8_lossy(&output.stdout).contains("Created user alice ("));
    assert!(output.stderr.is_empty());
    assert_eq!(user_count(&harness.database_path), 1);
    assert!(username_exists(&harness.database_path, "alice"));
}

#[test]
fn create_second_user_without_force_returns_conflict() {
    let harness = harness("admin-create-second-without-force");
    create_user(&harness, "alice", "correct horse battery\n", false).success();

    let assert = create_user(&harness, "bob", "correct horse battery\n", false).failure();
    let output = assert.get_output();

    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("at least one user exists; pass --force to create another")
    );
    assert_eq!(user_count(&harness.database_path), 1);
}

#[test]
fn create_second_user_with_force_succeeds() {
    let harness = harness("admin-create-second-with-force");
    create_user(&harness, "alice", "correct horse battery\n", false).success();

    create_user(&harness, "bob", "correct horse battery\n", true).success();

    assert_eq!(user_count(&harness.database_path), 2);
    assert!(username_exists(&harness.database_path, "bob"));
}

#[test]
fn empty_password_is_rejected() {
    let harness = harness("admin-empty-password");

    let assert = create_user(&harness, "alice", "\n", false).failure();
    let output = assert.get_output();

    assert!(String::from_utf8_lossy(&output.stderr).contains("password must not be empty"));
    assert_eq!(user_count(&harness.database_path), 0);
}

#[test]
fn duplicate_username_is_rejected() {
    let harness = harness("admin-duplicate-username");
    create_user(&harness, "alice", "correct horse battery\n", false).success();

    let assert = create_user(&harness, "alice", "another correct horse battery\n", true).failure();
    let output = assert.get_output();

    assert!(String::from_utf8_lossy(&output.stderr).contains("username already exists"));
    assert_eq!(user_count(&harness.database_path), 1);
}
