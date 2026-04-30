//! Backup schedule parsing primitives.

use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::{BackupError, BackupResult};

/// Parsed UTC cron schedule for backup and maintenance jobs.
pub struct CronSchedule {
    inner: cron::Schedule,
    source: String,
}

impl CronSchedule {
    /// Parses a cron expression for UTC scheduling.
    ///
    /// Project configuration uses five-field POSIX cron
    /// (`minute hour day-of-month month day-of-week`). The underlying `cron`
    /// crate expects seconds first, so five-field expressions are parsed with
    /// an implicit leading zero seconds field. Crate-supported shorthands such
    /// as `@daily` are passed through unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`BackupError::InvalidSchedule`] when the expression is empty
    /// or rejected by the cron parser.
    pub fn parse(expression: &str) -> BackupResult<Self> {
        let trimmed = expression.trim();
        if trimmed.is_empty() {
            return Err(BackupError::InvalidSchedule("empty".to_string()));
        }

        let parse_expression = cron_parse_expression(trimmed);
        let inner = cron::Schedule::from_str(&parse_expression)
            .map_err(|error| BackupError::InvalidSchedule(format!("{trimmed}: {error}")))?;

        Ok(Self {
            inner,
            source: trimmed.to_string(),
        })
    }

    /// Returns the trimmed source expression supplied by the operator.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the next UTC tick strictly after `after`.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.inner.after(&after).next()
    }

    /// Returns up to `count` upcoming UTC ticks strictly after `after`.
    pub fn upcoming(&self, after: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
        self.inner.after(&after).take(count).collect()
    }
}

fn cron_parse_expression(expression: &str) -> String {
    if expression.starts_with('@') || expression.split_whitespace().count() != 5 {
        expression.to_string()
    } else {
        format!("0 {expression}")
    }
}
