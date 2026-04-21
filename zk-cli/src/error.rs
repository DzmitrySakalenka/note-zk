//! Error types and exit-code mapping for the CLI entry point.

use std::fmt;

/// Successful CLI exit code.
pub const EXIT_OK: i32 = 0;
/// CLI exit code for issue-reporting commands.
pub const EXIT_ISSUES: i32 = 1;
/// CLI exit code for usage and bootstrap failures.
pub const EXIT_USAGE: i32 = 2;

/// Error categories surfaced by the CLI entry point.
#[derive(Debug)]
pub enum CliError {
    /// The invocation arguments were invalid.
    Argument(String),
    /// Config/bootstrap setup failed before the command could run.
    Bootstrap(anyhow::Error),
    /// The command completed but reported actionable issues.
    Issues,
    /// The command failed for an expected user-facing lifecycle reason.
    Issue(String),
    /// The command failed unexpectedly.
    Internal(anyhow::Error),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Argument(message) | Self::Issue(message) => formatter.write_str(message),
            Self::Bootstrap(source) => {
                formatter.write_str("bootstrap error: ")?;
                for (index, cause) in source.chain().enumerate() {
                    if index > 0 {
                        formatter.write_str(": ")?;
                    }
                    write!(formatter, "{cause}")?;
                }
                Ok(())
            }
            Self::Issues => formatter.write_str("issues detected"),
            Self::Internal(source) => write!(formatter, "internal error: {source}"),
        }
    }
}

/// Maps a structured CLI error to the process exit code contract.
#[must_use]
pub fn exit_code(error: &CliError) -> i32 {
    match error {
        CliError::Argument(_) | CliError::Bootstrap(_) => EXIT_USAGE,
        CliError::Issues | CliError::Issue(_) | CliError::Internal(_) => EXIT_ISSUES,
    }
}
