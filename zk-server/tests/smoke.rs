// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{io, path::PathBuf};

use zk_server::{ServerError, config::ConfigError};

#[test]
fn server_error_not_found_displays_and_is_send_sync_static() {
    let error = ServerError::NotFound("x".to_string());
    assert_eq!(error.to_string(), "not found: x");

    assert_send_sync_static::<ServerError>();
}

#[test]
fn server_config_error_display_includes_config_source() {
    let error = ServerError::Config(ConfigError::InvalidBackupOutputDir {
        path: PathBuf::from("tmp/backups"),
        source: io::Error::other("blocked"),
    });

    assert_eq!(
        error.to_string(),
        "config error: invalid backup output directory 'tmp/backups': blocked"
    );
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}
