// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_auth::{AuthError, AuthResult};

#[test]
fn auth_error_variants_display_and_are_send_sync_static() {
    let invalid_password_hash = AuthError::InvalidPasswordHash;
    assert_eq!(
        invalid_password_hash.to_string(),
        "invalid password hash format"
    );

    let password_verification = AuthError::PasswordVerification;
    assert_eq!(
        password_verification.to_string(),
        "password verification failed"
    );

    let invalid_token = AuthError::InvalidToken;
    assert_eq!(invalid_token.to_string(), "token format invalid");

    let invalid_session_id = AuthError::InvalidSessionId;
    assert_eq!(invalid_session_id.to_string(), "session id format invalid");

    let internal = AuthError::Internal("rng unavailable".to_string());
    assert_eq!(internal.to_string(), "internal error: rng unavailable");

    let alias_round_trip: AuthResult<()> = Err(AuthError::InvalidToken);
    assert!(matches!(alias_round_trip, Err(AuthError::InvalidToken)));

    assert_send_sync_static::<AuthError>();
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}
