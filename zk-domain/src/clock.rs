//! Shared UTC clock abstraction for deterministic tests and production time.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};

/// Time source used by callers that need deterministic tests.
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
