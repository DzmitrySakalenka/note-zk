use chrono::{DateTime, Utc};

use zk_app::config::ConfigValidationError;
use zk_app::{AppError, Clock, compute_retention_until};

struct FixedClock {
    now: DateTime<Utc>,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

fn fixed_clock(timestamp: &str) -> FixedClock {
    FixedClock {
        now: DateTime::parse_from_rfc3339(timestamp)
            .expect("fixed timestamp should parse")
            .with_timezone(&Utc),
    }
}

#[test]
fn computes_retention_from_explicit_trashed_at_base() {
    let retention_until = compute_retention_until(
        Some("2026-01-01T00:00:00Z"),
        14,
        &fixed_clock("2026-04-20T10:00:00Z"),
    )
    .expect("retention timestamp should compute");

    assert_eq!(retention_until, "2026-01-15T00:00:00Z");
}

#[test]
fn uses_clock_now_when_trashed_at_is_missing() {
    let retention_until = compute_retention_until(None, 7, &fixed_clock("2026-04-20T10:00:00Z"))
        .expect("retention timestamp should compute");

    assert_eq!(retention_until, "2026-04-27T10:00:00Z");
}

#[test]
fn zero_retention_days_returns_base_timestamp_unchanged() {
    let retention_until = compute_retention_until(
        Some("2026-01-01T00:00:00Z"),
        0,
        &fixed_clock("2026-04-20T10:00:00Z"),
    )
    .expect("retention timestamp should compute");

    assert_eq!(retention_until, "2026-01-01T00:00:00Z");
}

#[test]
fn retention_days_saturates_to_ten_year_cap() {
    let retention_until = compute_retention_until(
        Some("2026-01-01T00:00:00Z"),
        10_000,
        &fixed_clock("2026-04-20T10:00:00Z"),
    )
    .expect("retention timestamp should compute");

    assert_eq!(retention_until, "2035-12-30T00:00:00Z");
}

#[test]
fn malformed_trashed_at_returns_config_validation_error() {
    let err = compute_retention_until(
        Some("not-a-timestamp"),
        7,
        &fixed_clock("2026-04-20T10:00:00Z"),
    )
    .expect_err("invalid trashed_at should fail");

    assert!(matches!(
        err,
        AppError::ConfigValidation(ConfigValidationError::InvalidTrashedAtTimestamp {
            ref trashed_at,
            ..
        }) if trashed_at == "not-a-timestamp"
    ));
    assert!(
        err.to_string()
            .contains("while attempting to parse trashed_at timestamp"),
        "error should keep the parse context"
    );
}
