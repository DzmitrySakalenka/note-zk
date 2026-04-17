use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

use crate::id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision};

// ==========================================================================
// NoteFrontmatter
// ==========================================================================

fn string_or_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrList {
        Single(String),
        List(Vec<String>),
    }

    match StringOrList::deserialize(deserializer)? {
        StringOrList::Single(s) => Ok(vec![s]),
        StringOrList::List(v) => Ok(v),
    }
}

fn default_tags() -> Vec<String> {
    vec!["untagged".to_string()]
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Canonical note frontmatter after normalization.
pub struct NoteFrontmatter {
    /// Stable note identifier used across renames and sync.
    pub id: NoteId, // required
    /// Human-readable note title.
    pub title: String, // required
    #[serde(default = "default_tags", deserialize_with = "string_or_list")]
    /// Classification tags used for filtering and navigation.
    pub tags: Vec<String>, // required, default `["untagged"]`
    #[serde(rename = "type")]
    /// Optional note type for templates and workflows.
    pub note_type: Option<String>,
    #[serde(default, deserialize_with = "string_or_list")]
    /// Alternate titles that can be used for lookup.
    pub aliases: Vec<String>,
    /// Creation timestamp in user/frontmatter form.
    pub created: Option<String>,
    /// Last content update timestamp in user/frontmatter form.
    pub updated: Option<String>,
    /// Optional external source reference.
    pub source: Option<String>,
    /// Optional lifecycle/status marker.
    pub status: Option<String>,
    #[serde(default)]
    /// Upstream source note identifiers for provenance tracking.
    pub sources: Vec<NoteId>, // LLM Wiki provenance
    #[serde(flatten)]
    /// Unknown frontmatter keys preserved for round-trip safety.
    pub extra: BTreeMap<String, serde_yaml::Value>, // preserve unknown keys
}

// ==========================================================================
// RawNoteFrontmatter
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Raw parsed frontmatter before normalization and required-field filling.
pub struct RawNoteFrontmatter {
    /// Optional note identifier as read from input.
    pub id: Option<NoteId>,
    /// Optional title as read from input.
    pub title: Option<String>,
    #[serde(default = "default_tags", deserialize_with = "string_or_list")]
    /// Parsed tag values with default fallback for omitted field.
    pub tags: Vec<String>,
    #[serde(rename = "type")]
    /// Optional raw note type.
    pub note_type: Option<String>,
    #[serde(default, deserialize_with = "string_or_list")]
    /// Optional raw aliases.
    pub aliases: Vec<String>,
    /// Optional raw creation timestamp.
    pub created: Option<String>,
    /// Optional raw update timestamp.
    pub updated: Option<String>,
    /// Optional raw source reference.
    pub source: Option<String>,
    /// Optional raw status marker.
    pub status: Option<String>,
    #[serde(default)]
    /// Optional raw provenance source note identifiers.
    pub sources: Vec<NoteId>,
    #[serde(flatten)]
    /// Unknown raw frontmatter keys preserved during parse.
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
/// Severity level used by scan diagnostics and validation findings.
pub enum DiagnosticLevel {
    /// Blocks normal processing or requires explicit user intervention.
    Error,
    /// Highlights non-blocking issues that still need attention.
    Warning,
    /// Records useful observations about normalization/validation behavior.
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
/// Legacy scan-time diagnostic record preserved for compatibility.
pub struct ScanDiagnostic {
    /// Severity level of this diagnostic.
    pub level: DiagnosticLevel,
    /// Vault-relative path where the diagnostic occurred.
    pub path: RelativePath,
    /// Human-readable diagnostic message.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[non_exhaustive]
/// Stable identifier for machine-readable validation rule classification.
pub enum RuleId {
    /// Frontmatter could not be parsed into the expected model.
    InvalidFrontmatter,
    /// Title is still missing after derivation attempts.
    MissingTitle,
    /// Duplicate note ID appears in multiple notes.
    DuplicateId,
    /// Parsed note body has no content after trimming.
    EmptyBody,
    /// Note body does not include an H1 heading.
    NoH1InBody,
    /// Frontmatter title differs from first H1 content.
    TitleH1Mismatch,
    /// Filename contains suspicious characters or patterns.
    SuspiciousPathNaming,
    /// Note contains at least one Mermaid code block.
    MermaidBlockDetected,
    /// Title was derived from the first H1 heading.
    TitleDerivedFromH1,
    /// Title was derived from the filename stem.
    TitleDerivedFromFilename,
    /// Missing ID was injected during normalization.
    IdInjected,
    /// Default `untagged` tag is present.
    DefaultUntaggedTag,
    /// Frontmatter block was inserted during normalization.
    FrontmatterInserted,
    /// ID format is suspicious for ULID/Crockford expectations.
    IdFormatSuspicious,
    /// Outbound note link could not be resolved to any note.
    UnresolvedLink,
    /// Outbound note link matches more than one possible target.
    AmbiguousLink,
    /// Outbound asset reference points at a missing or invalid asset.
    BrokenAssetRef,
}

impl RuleId {
    /// Returns a short human-readable description for display and logs.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            RuleId::InvalidFrontmatter => "invalid frontmatter",
            RuleId::MissingTitle => "missing title after derivation",
            RuleId::DuplicateId => "duplicate note id",
            RuleId::EmptyBody => "empty note body",
            RuleId::NoH1InBody => "missing h1 in body",
            RuleId::TitleH1Mismatch => "frontmatter title and h1 mismatch",
            RuleId::SuspiciousPathNaming => "suspicious path naming",
            RuleId::MermaidBlockDetected => "mermaid block detected",
            RuleId::TitleDerivedFromH1 => "title derived from h1",
            RuleId::TitleDerivedFromFilename => "title derived from filename",
            RuleId::IdInjected => "id injected",
            RuleId::DefaultUntaggedTag => "default untagged tag",
            RuleId::FrontmatterInserted => "frontmatter inserted",
            RuleId::IdFormatSuspicious => "suspicious id format",
            RuleId::UnresolvedLink => "unresolved note link",
            RuleId::AmbiguousLink => "ambiguous note link",
            RuleId::BrokenAssetRef => "broken asset reference",
        }
    }

    /// Returns a stable kebab-case identifier for machine-readable output.
    #[must_use]
    pub fn as_kebab_case(&self) -> &'static str {
        match self {
            RuleId::InvalidFrontmatter => "invalid-frontmatter",
            RuleId::MissingTitle => "missing-title",
            RuleId::DuplicateId => "duplicate-id",
            RuleId::EmptyBody => "empty-body",
            RuleId::NoH1InBody => "no-h1-in-body",
            RuleId::TitleH1Mismatch => "title-h1-mismatch",
            RuleId::SuspiciousPathNaming => "suspicious-path-naming",
            RuleId::MermaidBlockDetected => "mermaid-block-detected",
            RuleId::TitleDerivedFromH1 => "title-derived-from-h1",
            RuleId::TitleDerivedFromFilename => "title-derived-from-filename",
            RuleId::IdInjected => "id-injected",
            RuleId::DefaultUntaggedTag => "default-untagged-tag",
            RuleId::FrontmatterInserted => "frontmatter-inserted",
            RuleId::IdFormatSuspicious => "id-format-suspicious",
            RuleId::UnresolvedLink => "unresolved-link",
            RuleId::AmbiguousLink => "ambiguous-link",
            RuleId::BrokenAssetRef => "broken-asset-ref",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
/// Structured validation finding emitted by a concrete rule.
pub struct ValidationFinding {
    /// Rule identifier that produced this finding.
    pub rule: RuleId,
    /// Effective severity level after overrides.
    pub level: DiagnosticLevel,
    /// Vault-relative path associated with the finding.
    pub path: RelativePath,
    /// Human-readable message describing the issue or observation.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
/// Aggregated counts over a full validation run.
pub struct ValidationSummary {
    /// Number of findings at error level.
    pub error_count: u32,
    /// Number of findings at warning level.
    pub warning_count: u32,
    /// Number of findings at info level.
    pub info_count: u32,
    /// Per-rule trigger counts in deterministic order.
    pub rules_triggered: Vec<(RuleId, u32)>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Complete validation output including findings and scan diagnostics.
pub struct ValidationReport {
    /// Structured rule findings generated by validation engine rules.
    pub findings: Vec<ValidationFinding>,
    /// Aggregated counts and per-rule totals.
    pub summary: ValidationSummary,
    /// Legacy scan diagnostics preserved for backward compatibility.
    pub scan_diagnostics: Vec<ScanDiagnostic>,
}

impl ValidationReport {
    /// Serializes this report as pretty JSON.
    ///
    /// # Panics
    ///
    /// Panics if serialization fails.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .expect("validation report should always serialize to pretty json")
    }

    /// Serializes this report as compact JSON.
    ///
    /// # Panics
    ///
    /// Panics if serialization fails.
    #[must_use]
    pub fn to_json_compact(&self) -> String {
        serde_json::to_string(self)
            .expect("validation report should always serialize to compact json")
    }
}

// ==========================================================================
// NoteDocument
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Full note document used by normalization and validation pipelines.
pub struct NoteDocument {
    /// Canonical normalized frontmatter for this document.
    pub frontmatter: NoteFrontmatter,
    /// Markdown body content excluding frontmatter delimiters.
    pub body_markdown: String,
}

// ==========================================================================
// NoteRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
/// Durable metadata row for a note in local index and sync state.
pub struct NoteRecord {
    /// Stable note identifier.
    pub note_id: NoteId,
    /// Current vault-relative location of the note.
    pub current_path: RelativePath,
    /// Hash of current note file content.
    pub content_hash: ContentHash,
    /// Server/client creation timestamp representation.
    pub created_at: String,
    /// Server/client updated timestamp representation.
    pub updated_at: String,
    /// Time when note was moved to trash, if applicable.
    pub trashed_at: Option<String>,
    /// Time when note was hard-deleted, if applicable.
    pub deleted_at: Option<String>,
    /// Whether the note is currently in trash state.
    pub is_trashed: bool,
    /// Whether the note is currently hard-deleted.
    pub is_deleted: bool,
    /// Optional device that last edited this note.
    pub last_editor_device_id: Option<DeviceId>,
    /// Last known server revision associated with this note.
    pub server_revision: ServerRevision,
}
