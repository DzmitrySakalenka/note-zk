use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Days, SecondsFormat, TimeZone, Utc};

use crate::{AppError, config::ConfigValidationError};

/// Time source abstraction for retention calculations.
pub trait Clock {
    /// Returns the current UTC timestamp.
    fn now(&self) -> DateTime<Utc>;
}

/// System-backed clock implementation for production callers.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after the unix epoch");
        let seconds = i64::try_from(now.as_secs()).expect("unix seconds should fit in i64");

        Utc.timestamp_opt(seconds, now.subsec_nanos())
            .single()
            .expect("system time should convert into a single UTC timestamp")
    }
}

/// Computes the retention-expiry timestamp for one trashed item.
///
/// The base timestamp is `trashed_at` when present, or `clock.now()` otherwise.
/// The retention window is clamped to at most 3650 calendar days, so callers
/// cannot use this helper to express a never-expire policy.
///
/// # Errors
///
/// Returns [`AppError::ConfigValidation`] if `trashed_at` is present but not a
/// valid RFC-3339 timestamp.
///
/// # Panics
///
/// Panics if `trashed_at` is so close to chrono's maximum representable
/// timestamp that even the clamped retention window cannot be added safely.
pub fn compute_retention_until(
    trashed_at: Option<&str>,
    retention_days: u32,
    clock: &dyn Clock,
) -> Result<String, AppError> {
    let base = parse_base_timestamp(trashed_at, clock)?;
    let clamped_days = retention_days.min(3650);
    let retention_until = if clamped_days == 0 {
        base
    } else {
        base.checked_add_days(Days::new(u64::from(clamped_days)))
            .expect("retention helper clamps day arithmetic to a representable range")
    };

    Ok(retention_until.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn parse_base_timestamp(
    trashed_at: Option<&str>,
    clock: &dyn Clock,
) -> Result<DateTime<Utc>, AppError> {
    match trashed_at {
        Some(trashed_at) => DateTime::parse_from_rfc3339(trashed_at)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .map_err(|source| {
                AppError::ConfigValidation(ConfigValidationError::InvalidTrashedAtTimestamp {
                    trashed_at: trashed_at.to_string(),
                    reason: source.to_string(),
                })
            }),
        None => Ok(clock.now()),
    }
}
