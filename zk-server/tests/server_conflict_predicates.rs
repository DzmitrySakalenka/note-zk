// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_server::service::{
    AcceptOutcome, accept_or_conflict, is_fresh_create, is_stale_revision, next_server_revision,
};

fn expected_outcome(exists: bool, base: Option<i64>, current: i64) -> AcceptOutcome {
    match (exists, base) {
        (false, None) => AcceptOutcome::Accept { next_revision: 1 },
        (false, Some(_)) => AcceptOutcome::ConflictStale {
            current_revision: 0,
        },
        (true, None) => AcceptOutcome::ConflictDuplicateCreate,
        (true, Some(base_revision)) if base_revision != current => AcceptOutcome::ConflictStale {
            current_revision: current,
        },
        (true, Some(_)) => AcceptOutcome::Accept {
            next_revision: current + 1,
        },
    }
}

#[test]
fn is_stale_revision_matches_documented_rules() {
    assert!(!is_stale_revision(&None, 0));
    assert!(!is_stale_revision(&None, 5));
    assert!(!is_stale_revision(&Some(3), 3));
    assert!(is_stale_revision(&Some(3), 4));
}

#[test]
fn is_fresh_create_matches_documented_truth_table() {
    assert!(is_fresh_create(false, &None));
    assert!(!is_fresh_create(false, &Some(0)));
    assert!(!is_fresh_create(true, &None));
    assert!(!is_fresh_create(true, &Some(0)));
}

#[test]
fn next_server_revision_increments_current_revision() {
    assert_eq!(next_server_revision(0), 1);
    assert_eq!(next_server_revision(5), 6);
}

#[test]
fn accept_or_conflict_matches_exhaustive_truth_table() {
    let exists_cases = [false, true];
    let base_cases = [None, Some(0), Some(1), Some(5)];
    let current_cases = [0, 1, 5];

    for exists in exists_cases {
        for base in base_cases {
            for current in current_cases {
                let actual = accept_or_conflict(&base, current, exists);
                let expected = expected_outcome(exists, base, current);

                assert_eq!(
                    actual, expected,
                    "unexpected outcome for exists={exists}, base={base:?}, current={current}"
                );
            }
        }
    }
}
