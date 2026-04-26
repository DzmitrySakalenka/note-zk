use zk_domain::{Clock, SystemClock};

#[test]
fn system_clock_returns_current_utc_timestamp() {
    let clock: &dyn Clock = &SystemClock;

    let now = clock.now();

    assert!(now.timestamp() > 0);
}
