// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::{Clock, SystemClock};

#[test]
fn system_clock_returns_current_utc_timestamp() {
    let clock: &dyn Clock = &SystemClock;

    let now = clock.now();

    assert!(now.timestamp() > 0);
}
