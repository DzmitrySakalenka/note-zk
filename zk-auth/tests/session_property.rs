// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use chrono::{TimeZone, Utc};
use common::{
    Base32WrongLength, InvalidBase32Text, SHA256_DIGEST_LEN, SHA256_HEX_LEN, decoded_sha256_len,
};
use quickcheck_macros::quickcheck;
use zk_auth::{
    AuthError,
    session::{
        SessionLifetime, hash_csrf_token, hash_session_id, issue_csrf_token, issue_session,
        validate_session_id_format,
    },
};

#[quickcheck]
fn issue_then_validate_format_always_ok(_case: u8) -> bool {
    let Ok(issued) = issue_session(fixed_now(), SessionLifetime::default()) else {
        return false;
    };

    validate_session_id_format(&issued.plaintext_id).is_ok()
}

#[quickcheck]
#[expect(
    clippy::needless_pass_by_value,
    reason = "quickcheck property functions receive owned arbitrary values by design"
)]
fn random_string_outside_alphabet_fails_validation(candidate: InvalidBase32Text) -> bool {
    matches!(
        validate_session_id_format(&candidate.0),
        Err(AuthError::InvalidSessionId)
    )
}

#[quickcheck]
#[expect(
    clippy::needless_pass_by_value,
    reason = "quickcheck property functions receive owned arbitrary values by design"
)]
fn length_other_than_52_fails_validation(candidate: Base32WrongLength) -> bool {
    matches!(
        validate_session_id_format(&candidate.0),
        Err(AuthError::InvalidSessionId)
    )
}

#[quickcheck]
#[expect(
    clippy::needless_pass_by_value,
    reason = "quickcheck property functions receive owned arbitrary values by design"
)]
fn hash_session_id_is_deterministic(plaintext: String) -> bool {
    let first = hash_session_id(&plaintext);
    let second = hash_session_id(&plaintext);

    first == second
        && first.len() == SHA256_HEX_LEN
        && decoded_sha256_len(&first) == Some(SHA256_DIGEST_LEN)
}

#[quickcheck]
#[expect(
    clippy::needless_pass_by_value,
    reason = "quickcheck property functions receive owned arbitrary values by design"
)]
fn hash_csrf_token_is_deterministic(plaintext: String) -> bool {
    let first = hash_csrf_token(&plaintext);
    let second = hash_csrf_token(&plaintext);

    first == second
        && first.len() == SHA256_HEX_LEN
        && decoded_sha256_len(&first) == Some(SHA256_DIGEST_LEN)
}

#[quickcheck]
fn issued_csrf_hash_matches_plaintext(_case: u8) -> bool {
    let Ok(issued) = issue_csrf_token() else {
        return false;
    };

    issued.sha256_hex == hash_csrf_token(&issued.plaintext)
        && issued.sha256_hex.len() == SHA256_HEX_LEN
        && decoded_sha256_len(&issued.sha256_hex) == Some(SHA256_DIGEST_LEN)
}

fn fixed_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("fixed timestamp is valid")
}
