// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::time::{Duration, Instant};

use zk_auth::{
    AuthError,
    password::{HashParams, hash_password, verify_password},
};

const TEST_PARAMS: HashParams = HashParams {
    memory_kib: 8,
    iterations: 1,
    parallelism: 1,
};

#[test]
fn hash_then_verify_succeeds_with_default_params() {
    let phc = hash_password("correct horse battery staple", HashParams::default())
        .expect("password hash should be generated");

    assert!(phc.starts_with("$argon2id$v=19$"));
    assert!(phc.contains("$m=19456,t=2,p=1$"));
    verify_password("correct horse battery staple", &phc)
        .expect("password should verify against generated hash");
}

#[test]
fn verify_rejects_wrong_password() {
    let phc = hash_password("correct horse battery staple", TEST_PARAMS)
        .expect("password hash should be generated");

    let error = verify_password("wrong password", &phc).expect_err("wrong password should fail");

    assert!(matches!(error, AuthError::PasswordVerification));
}

#[test]
fn verify_rejects_malformed_hash() {
    let error =
        verify_password("password", "not-a-phc-string").expect_err("malformed hash should fail");

    assert!(matches!(error, AuthError::InvalidPasswordHash));
}

#[test]
fn same_password_hashed_twice_yields_different_phc_strings() {
    let first = hash_password("same password", TEST_PARAMS).expect("first hash should succeed");
    let second = hash_password("same password", TEST_PARAMS).expect("second hash should succeed");

    assert_ne!(first, second);
    verify_password("same password", &first).expect("first hash should verify");
    verify_password("same password", &second).expect("second hash should verify");
}

#[test]
fn hash_with_test_params_is_fast_enough() {
    let started_at = Instant::now();

    let phc = hash_password("fast password", TEST_PARAMS).expect("test hash should succeed");

    assert!(started_at.elapsed() < Duration::from_millis(50));
    verify_password("fast password", &phc).expect("fast hash should verify");
}

#[test]
#[ignore = "timing-sensitive characterization; run manually on an idle machine"]
fn verify_constant_time_property() {
    let phc = hash_password("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", TEST_PARAMS)
        .expect("test hash should be generated");
    let mut timings = Vec::with_capacity(100);

    for offset in 0..100 {
        let mut password = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec();
        let index = offset % password.len();
        password[index] = b'b';
        let candidate = String::from_utf8(password).expect("test password is valid UTF-8");

        let started_at = Instant::now();
        let result = verify_password(&candidate, &phc);
        timings.push(started_at.elapsed().as_nanos());

        assert!(matches!(result, Err(AuthError::PasswordVerification)));
    }

    let sample_count = u128::try_from(timings.len()).expect("timing sample count fits u128");
    let mean = timings.iter().sum::<u128>() / sample_count;
    let variance = timings
        .iter()
        .map(|duration| {
            let delta = duration.abs_diff(mean);
            delta * delta
        })
        .sum::<u128>()
        / sample_count;

    assert!(
        variance * 10_000 < mean * mean * 1_225,
        "timing variance too high: mean={mean}, variance={variance}"
    );
}
