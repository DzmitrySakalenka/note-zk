// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use common::{
    Base32WrongLength, InvalidBase32Text, SHA256_DIGEST_LEN, SHA256_HEX_LEN, decoded_sha256_len,
};
use quickcheck_macros::quickcheck;
use zk_auth::{
    AuthError,
    token::{hash_device_token, issue_device_token, validate_device_token_format},
};

#[quickcheck]
fn issue_then_validate_format_always_ok(_case: u8) -> bool {
    let Ok(issued) = issue_device_token() else {
        return false;
    };

    validate_device_token_format(&issued.plaintext).is_ok()
}

#[quickcheck]
#[expect(
    clippy::needless_pass_by_value,
    reason = "quickcheck property functions receive owned arbitrary values by design"
)]
fn random_string_outside_alphabet_fails_validation(candidate: InvalidBase32Text) -> bool {
    matches!(
        validate_device_token_format(&candidate.0),
        Err(AuthError::InvalidToken)
    )
}

#[quickcheck]
#[expect(
    clippy::needless_pass_by_value,
    reason = "quickcheck property functions receive owned arbitrary values by design"
)]
fn length_other_than_52_fails_validation(candidate: Base32WrongLength) -> bool {
    matches!(
        validate_device_token_format(&candidate.0),
        Err(AuthError::InvalidToken)
    )
}

#[quickcheck]
#[expect(
    clippy::needless_pass_by_value,
    reason = "quickcheck property functions receive owned arbitrary values by design"
)]
fn hash_device_token_is_deterministic(plaintext: String) -> bool {
    let first = hash_device_token(&plaintext);
    let second = hash_device_token(&plaintext);

    first == second
        && first.len() == SHA256_HEX_LEN
        && decoded_sha256_len(&first) == Some(SHA256_DIGEST_LEN)
}
