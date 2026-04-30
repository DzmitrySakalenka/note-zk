// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, TimeZone, Utc};
use zk_backup::{BackupError, schedule::CronSchedule};

#[test]
fn daily_cron_returns_expected_next_utc_tick() {
    let schedule = CronSchedule::parse(" 0 3 * * * ").expect("daily schedule should parse");
    let after = utc_datetime(2026, 4, 30, 0, 0);

    assert_eq!(schedule.source(), "0 3 * * *");
    assert_eq!(
        schedule.next_after(after),
        Some(utc_datetime(2026, 4, 30, 3, 0))
    );
}

#[test]
fn daily_shorthand_parses() {
    let schedule = CronSchedule::parse("@daily").expect("@daily should parse");
    let after = utc_datetime(2026, 4, 30, 12, 0);

    assert_eq!(
        schedule.next_after(after),
        Some(utc_datetime(2026, 5, 1, 0, 0))
    );
}

#[test]
fn empty_schedule_returns_invalid_schedule_empty() {
    let result = CronSchedule::parse("   ");

    assert!(matches!(
        result,
        Err(BackupError::InvalidSchedule(message)) if message == "empty"
    ));
}

#[test]
fn invalid_schedule_error_contains_original_expression() {
    let result = CronSchedule::parse("not a cron expression");

    assert!(matches!(
        result,
        Err(BackupError::InvalidSchedule(message))
            if message.contains("not a cron expression")
    ));
}

#[test]
fn impossible_future_date_returns_none() {
    let schedule =
        CronSchedule::parse("0 0 30 2 *").expect("syntactically valid cron should parse");
    let after = utc_datetime(2026, 4, 30, 0, 0);

    assert_eq!(schedule.next_after(after), None);
}

#[test]
fn upcoming_returns_requested_strictly_increasing_ticks() {
    let schedule = CronSchedule::parse("*/15 * * * *").expect("interval schedule should parse");
    let after = utc_datetime(2026, 4, 30, 0, 0);

    let ticks = schedule.upcoming(after, 5);

    assert_eq!(ticks.len(), 5);
    assert!(ticks.windows(2).all(|pair| pair[0] < pair[1]));
}

fn utc_datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("test timestamp is valid")
}
