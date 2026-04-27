// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{net::SocketAddr, path::PathBuf, ptr};

use chrono::Utc;
use tempfile::NamedTempFile;
use zk_server::{config::ServerConfig, state::bootstrap_with_config};

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
    }
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
        1
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
