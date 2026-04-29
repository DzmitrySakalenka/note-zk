// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::HashSet;

use chrono::{Duration, TimeZone, Utc};
use zk_auth::{
    AuthError,
    session::{SessionLifetime, hash_session_id, issue_session, validate_session_id_format},
};

#[test]
fn issue_returns_unique_ids() {
    let now = fixed_now();
    let mut plaintext_ids = HashSet::with_capacity(1_000);

    for _ in 0..1_000 {
        let issued =
            issue_session(now, SessionLifetime::default()).expect("session should be issued");

        assert_eq!(issued.plaintext_id.len(), 52);
        assert_eq!(issued.sha256_hex.len(), 64);
        assert_eq!(issued.sha256_hex, hash_session_id(&issued.plaintext_id));
        assert!(plaintext_ids.insert(issued.plaintext_id));
    }
}

#[test]
fn expires_at_equals_now_plus_lifetime() {
    let now = fixed_now();
    let lifetime = SessionLifetime {
        max_age: Duration::minutes(90),
    };

    let issued = issue_session(now, lifetime).expect("session should be issued");

    assert_eq!(issued.expires_at, now + lifetime.max_age);
}

#[test]
fn default_lifetime_is_twenty_four_hours() {
    assert_eq!(SessionLifetime::default().max_age, Duration::hours(24));
}

#[test]
fn hash_matches_for_same_plaintext_id() {
    let first = hash_session_id("shared-secret");
    let second = hash_session_id("shared-secret");

    assert_eq!(first, second);
    assert_eq!(
        first,
        "d3046ecc8dd3242adf62801a33ef1004003b01b4c8f558df72e637da30321ccd"
    );
}

#[test]
fn validate_accepts_freshly_issued_id() {
    let issued =
        issue_session(fixed_now(), SessionLifetime::default()).expect("session should be issued");

    validate_session_id_format(&issued.plaintext_id).expect("issued session id should be valid");
}

#[test]
fn validate_rejects_wrong_length() {
    for invalid in ["A".repeat(51), "A".repeat(53), String::new()] {
        let error = validate_session_id_format(&invalid).expect_err("invalid length should fail");

        assert!(matches!(error, AuthError::InvalidSessionId));
    }
}

#[test]
fn validate_rejects_non_base32_chars() {
    for invalid_char in ['0', '1', '8', '9', 'a', 'z', '='] {
        let mut candidate = "A".repeat(52);
        candidate.replace_range(0..1, &invalid_char.to_string());

        let error =
            validate_session_id_format(&candidate).expect_err("invalid alphabet should fail");

        assert!(matches!(error, AuthError::InvalidSessionId));
    }
}

fn fixed_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("fixed timestamp is valid")
}
