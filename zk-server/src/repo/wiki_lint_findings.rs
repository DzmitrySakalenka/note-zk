use std::io::{self, ErrorKind};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, types::Type};
use zk_domain::{JobId, id::NoteId};

use super::timestamps::{decode_timestamp, encode_timestamp};
use crate::{
    ServerError, ServerResult,
    ai::lint_wiki::{WikiLintFinding, WikiLintFindingKind, WikiLintSeverity},
};

const DELETE_FINDINGS_FOR_JOB_SQL: &str = "DELETE FROM wiki_lint_findings WHERE job_id = ?1";
const INSERT_FINDING_SQL: &str = r"
INSERT INTO wiki_lint_findings (
    finding_id,
    job_id,
    kind,
    page_id,
    source_id,
    message,
    severity,
    created_at
) VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
)
";
const LIST_FINDINGS_FOR_JOB_SQL: &str = r"
SELECT
    finding_id,
    job_id,
    kind,
    page_id,
    source_id,
    message,
    severity,
    created_at
FROM wiki_lint_findings
WHERE job_id = ?1
ORDER BY created_at ASC, finding_id ASC
";
const LIST_FINDINGS_FOR_JOB_LIMITED_SQL: &str = r"
SELECT
    finding_id,
    job_id,
    kind,
    page_id,
    source_id,
    message,
    severity,
    created_at
FROM wiki_lint_findings
WHERE job_id = ?1
ORDER BY created_at ASC, finding_id ASC
LIMIT ?2
";
const COUNT_FINDINGS_FOR_JOB_SQL: &str =
    "SELECT COUNT(*) FROM wiki_lint_findings WHERE job_id = ?1";

/// Persisted wiki lint finding row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLintFindingRow {
    /// Stable finding identifier.
    pub finding_id: String,
    /// AI job that produced the finding set.
    pub job_id: JobId,
    /// Machine-readable finding kind.
    pub kind: WikiLintFindingKind,
    /// Wiki page touched by the finding, when applicable.
    pub page_id: Option<NoteId>,
    /// Source note touched by the finding, when applicable.
    pub source_id: Option<NoteId>,
    /// Human-readable finding message.
    pub message: String,
    /// Finding severity.
    pub severity: WikiLintSeverity,
    /// Time when the finding row was persisted.
    pub created_at: DateTime<Utc>,
}

/// Repository over the server `wiki_lint_findings` table.
pub struct WikiLintFindingsRepo<'a> {
    conn: &'a Connection,
}

impl<'a> WikiLintFindingsRepo<'a> {
    /// Creates a repository backed by one `SQLite` connection.
    #[must_use]
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Replaces the full finding set for one lint job.
    ///
    /// The caller owns transaction scope. Passing a transaction-backed
    /// connection makes retry replacement atomic with surrounding writes.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if the delete or any insert fails.
    pub fn insert_many(
        &self,
        job_id: &JobId,
        findings: &[WikiLintFinding],
        now: DateTime<Utc>,
    ) -> ServerResult<u64> {
        self.conn
            .execute(DELETE_FINDINGS_FOR_JOB_SQL, [job_id.as_str()])
            .map_err(ServerError::Database)?;

        let created_at = encode_timestamp(now);
        let mut statement = self
            .conn
            .prepare_cached(INSERT_FINDING_SQL)
            .map_err(ServerError::Database)?;
        let mut inserted = 0_u64;

        for finding in findings {
            let finding_id = ulid::Ulid::new().to_string();
            let page_id = finding.page_id.as_ref().map(NoteId::as_str);
            let source_id = finding.source_id.as_ref().map(NoteId::as_str);

            statement
                .execute(rusqlite::params![
                    finding_id,
                    job_id.as_str(),
                    kind_as_str(finding.kind),
                    page_id,
                    source_id,
                    &finding.message,
                    severity_as_str(finding.severity),
                    &created_at,
                ])
                .map_err(ServerError::Database)?;
            inserted += 1;
        }

        Ok(inserted)
    }

    /// Lists findings for one job in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the query fails or row decoding is invalid.
    pub fn list_for_job(&self, job_id: &JobId) -> ServerResult<Vec<WikiLintFindingRow>> {
        let mut statement = self
            .conn
            .prepare_cached(LIST_FINDINGS_FOR_JOB_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map([job_id.as_str()], decode_wiki_lint_finding_row)
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Lists a capped number of findings for one job in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the query fails or row decoding is invalid.
    pub fn list_for_job_limited(
        &self,
        job_id: &JobId,
        limit: u32,
    ) -> ServerResult<Vec<WikiLintFindingRow>> {
        let mut statement = self
            .conn
            .prepare_cached(LIST_FINDINGS_FOR_JOB_LIMITED_SQL)
            .map_err(ServerError::Database)?;
        let rows = statement
            .query_map(
                rusqlite::params![job_id.as_str(), i64::from(limit)],
                decode_wiki_lint_finding_row,
            )
            .map_err(ServerError::Database)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ServerError::Database)
    }

    /// Counts findings for one job.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the query fails or count decoding is invalid.
    pub fn count_for_job(&self, job_id: &JobId) -> ServerResult<u64> {
        self.conn
            .query_row(COUNT_FINDINGS_FOR_JOB_SQL, [job_id.as_str()], |row| {
                let value = row.get::<_, i64>(0)?;
                u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
            })
            .map_err(ServerError::Database)
    }
}

fn decode_wiki_lint_finding_row(row: &Row<'_>) -> rusqlite::Result<WikiLintFindingRow> {
    Ok(WikiLintFindingRow {
        finding_id: row.get(0)?,
        job_id: decode_job_id(row, 1)?,
        kind: decode_kind(row, 2)?,
        page_id: decode_optional_note_id(row, 3)?,
        source_id: decode_optional_note_id(row, 4)?,
        message: row.get(5)?,
        severity: decode_severity(row, 6)?,
        created_at: decode_timestamp(row, 7)?,
    })
}

fn decode_job_id(row: &Row<'_>, index: usize) -> rusqlite::Result<JobId> {
    let value = row.get::<_, String>(index)?;
    value.parse().map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(source))
    })
}

fn decode_optional_note_id(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<NoteId>> {
    row.get::<_, Option<String>>(index)
        .map(|value| value.map(|note_id| NoteId::new(&note_id)))
}

fn decode_kind(row: &Row<'_>, index: usize) -> rusqlite::Result<WikiLintFindingKind> {
    let value = row.get::<_, String>(index)?;

    match value.as_str() {
        "orphan_page" => Ok(WikiLintFindingKind::OrphanPage),
        "stale_claim" => Ok(WikiLintFindingKind::StaleClaim),
        "missing_cross_reference" => Ok(WikiLintFindingKind::MissingCrossReference),
        "important_concept_without_page" => Ok(WikiLintFindingKind::ImportantConceptWithoutPage),
        other => Err(invalid_text_failure(
            index,
            format!("unknown wiki lint finding kind: {other}"),
        )),
    }
}

fn decode_severity(row: &Row<'_>, index: usize) -> rusqlite::Result<WikiLintSeverity> {
    let value = row.get::<_, String>(index)?;

    match value.as_str() {
        "info" => Ok(WikiLintSeverity::Info),
        "warning" => Ok(WikiLintSeverity::Warning),
        "error" => Ok(WikiLintSeverity::Error),
        other => Err(invalid_text_failure(
            index,
            format!("unknown wiki lint severity: {other}"),
        )),
    }
}

fn kind_as_str(kind: WikiLintFindingKind) -> &'static str {
    match kind {
        WikiLintFindingKind::OrphanPage => "orphan_page",
        WikiLintFindingKind::StaleClaim => "stale_claim",
        WikiLintFindingKind::MissingCrossReference => "missing_cross_reference",
        WikiLintFindingKind::ImportantConceptWithoutPage => "important_concept_without_page",
    }
}

fn severity_as_str(severity: WikiLintSeverity) -> &'static str {
    match severity {
        WikiLintSeverity::Info => "info",
        WikiLintSeverity::Warning => "warning",
        WikiLintSeverity::Error => "error",
    }
}

fn invalid_text_failure(index: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        Type::Text,
        Box::new(io::Error::new(ErrorKind::InvalidData, message)),
    )
}
