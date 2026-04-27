// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_server::ServerError;

#[test]
fn server_error_not_found_displays_and_is_send_sync_static() {
    let error = ServerError::NotFound("x".to_string());
    assert_eq!(error.to_string(), "not found: x");

    assert_send_sync_static::<ServerError>();
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}
