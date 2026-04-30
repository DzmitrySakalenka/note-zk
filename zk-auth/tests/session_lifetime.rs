// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{Duration, TimeZone, Utc};
use zk_auth::session::{SessionLifetime, issue_session};

#[test]
fn default_lifetime_expires_exactly_twenty_four_hours_after_issue() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 29, 12, 0, 0)
        .single()
        .expect("fixed timestamp is valid");
    let lifetime = SessionLifetime::default();

    let issued = issue_session(now, lifetime).expect("session should be issued");

    assert_eq!(lifetime.max_age, Duration::hours(24));
    assert_eq!(issued.expires_at, now + Duration::hours(24));
}
