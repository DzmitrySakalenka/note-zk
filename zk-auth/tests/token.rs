// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::HashSet;

use zk_auth::{
    AuthError,
    token::{hash_device_token, issue_device_token, validate_device_token_format},
};

#[test]
fn issue_returns_distinct_plaintext_each_call() {
    let mut plaintexts = HashSet::with_capacity(1_000);

    for _ in 0..1_000 {
        let issued = issue_device_token().expect("device token should be issued");

        assert_eq!(issued.plaintext.len(), 52);
        assert_eq!(issued.sha256_hex.len(), 64);
        assert_eq!(issued.sha256_hex, hash_device_token(&issued.plaintext));
        assert!(plaintexts.insert(issued.plaintext));
    }
}

#[test]
fn hash_matches_for_same_plaintext() {
    let first = hash_device_token("shared-secret");
    let second = hash_device_token("shared-secret");

    assert_eq!(first, second);
    assert_eq!(
        first,
        "d3046ecc8dd3242adf62801a33ef1004003b01b4c8f558df72e637da30321ccd"
    );
}

#[test]
fn hash_differs_across_distinct_plaintexts() {
    let first = hash_device_token("first-token");
    let second = hash_device_token("second-token");

    assert_ne!(first, second);
}

#[test]
fn validate_accepts_freshly_issued_token() {
    let issued = issue_device_token().expect("device token should be issued");

    validate_device_token_format(&issued.plaintext).expect("issued token should be valid");
}

#[test]
fn validate_rejects_wrong_length() {
    for invalid in ["A".repeat(51), "A".repeat(53), String::new()] {
        let error = validate_device_token_format(&invalid).expect_err("invalid length should fail");

        assert!(matches!(error, AuthError::InvalidToken));
    }
}

#[test]
fn validate_rejects_non_base32_chars() {
    for invalid_char in ['0', '1', '8', '9', 'a', 'z', '='] {
        let mut candidate = "A".repeat(52);
        candidate.replace_range(0..1, &invalid_char.to_string());

        let error =
            validate_device_token_format(&candidate).expect_err("invalid alphabet should fail");

        assert!(matches!(error, AuthError::InvalidToken));
    }
}
