//! AI background job identifiers.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable identifier for an AI background job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct JobId(String);

impl JobId {
    /// Generates a new ULID-backed job identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(ulid::Ulid::new().to_string())
    }

    /// Parses and validates a canonical ULID job identifier.
    ///
    /// # Errors
    ///
    /// Returns [`JobIdParseError`] when the value is not a canonical ULID.
    pub fn new(value: &str) -> Result<Self, JobIdParseError> {
        ulid::Ulid::from_string(value).map_err(|_| JobIdParseError)?;
        Ok(Self(value.to_string()))
    }

    /// Returns the identifier as a borrowed string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for JobId {
    type Err = JobIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Error returned when parsing a job identifier fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("job id must be a canonical ULID")]
pub struct JobIdParseError;
