// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use quickcheck::{Arbitrary, Gen, QuickCheck};
use zk_auth::{
    AuthError,
    password::{HashParams, hash_password, verify_password},
};

#[derive(Clone, Debug)]
struct ValidPassword(String);

#[derive(Clone, Debug)]
struct DistinctPasswords {
    first: String,
    second: String,
}

impl Arbitrary for ValidPassword {
    fn arbitrary(g: &mut Gen) -> Self {
        Self(valid_password(g))
    }
}

impl Arbitrary for DistinctPasswords {
    fn arbitrary(g: &mut Gen) -> Self {
        let first = valid_password(g);
        let mut second = valid_password(g);

        if second == first {
            second = distinct_password(&first);
        }

        Self { first, second }
    }
}

#[test]
fn hash_then_verify_succeeds() {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "quickcheck property functions receive owned arbitrary values by design"
    )]
    fn property(candidate: ValidPassword) -> bool {
        let Ok(hash) = hash_password(&candidate.0, HashParams::default()) else {
            return false;
        };

        verify_password(&candidate.0, &hash).is_ok()
    }

    QuickCheck::new()
        .tests(8)
        .quickcheck(property as fn(ValidPassword) -> bool);
}

#[test]
fn verify_with_different_password_fails() {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "quickcheck property functions receive owned arbitrary values by design"
    )]
    fn property(candidate: DistinctPasswords) -> bool {
        let Ok(hash) = hash_password(&candidate.first, HashParams::default()) else {
            return false;
        };

        matches!(
            verify_password(&candidate.second, &hash),
            Err(AuthError::PasswordVerification)
        )
    }

    QuickCheck::new()
        .tests(8)
        .quickcheck(property as fn(DistinctPasswords) -> bool);
}

fn valid_password(g: &mut Gen) -> String {
    let mut password = String::arbitrary(g);

    password = password.chars().take(256).collect();
    while password.chars().count() < 8 {
        password.push('a');
    }

    password
}

fn distinct_password(password: &str) -> String {
    let mut chars = password.chars().collect::<Vec<_>>();

    if chars.len() < 256 {
        chars.push('!');
    } else {
        chars[0] = if chars[0] == '!' { '?' } else { '!' };
    }

    chars.into_iter().collect()
}
