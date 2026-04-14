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
pub struct NoteFrontmatter {
    pub id: NoteId,    // required
    pub title: String, // required
    #[serde(default = "default_tags", deserialize_with = "string_or_list")]
    pub tags: Vec<String>, // required, default `["untagged"]`
    #[serde(rename = "type")]
    pub note_type: Option<String>,
    #[serde(default, deserialize_with = "string_or_list")]
    pub aliases: Vec<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub sources: Vec<NoteId>, // LLM Wiki provenance
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>, // preserve unknown keys
}

// ==========================================================================
// RawNoteFrontmatter
// ==========================================================================

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RawNoteFrontmatter {
    pub id: Option<NoteId>,
    pub title: Option<String>,
    #[serde(default = "default_tags", deserialize_with = "string_or_list")]
    pub tags: Vec<String>,
    #[serde(rename = "type")]
    pub note_type: Option<String>,
    #[serde(default, deserialize_with = "string_or_list")]
    pub aliases: Vec<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub sources: Vec<NoteId>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ScanDiagnostic {
    pub level: DiagnosticLevel,
    pub path: RelativePath,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[non_exhaustive]
pub enum RuleId {
    InvalidFrontmatter,
    MissingTitle,
    DuplicateId,
    EmptyBody,
    NoH1InBody,
    TitleH1Mismatch,
    SuspiciousPathNaming,
    MermaidBlockDetected,
    TitleDerivedFromH1,
    TitleDerivedFromFilename,
    IdInjected,
    DefaultUntaggedTag,
    FrontmatterInserted,
    IdFormatSuspicious,
}

impl RuleId {
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ValidationFinding {
    pub rule: RuleId,
    pub level: DiagnosticLevel,
    pub path: RelativePath,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ValidationSummary {
    pub error_count: u32,
    pub warning_count: u32,
    pub info_count: u32,
    pub rules_triggered: Vec<(RuleId, u32)>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ValidationReport {
    pub findings: Vec<ValidationFinding>,
    pub summary: ValidationSummary,
    pub scan_diagnostics: Vec<ScanDiagnostic>,
}

// ==========================================================================
// NoteDocument
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct NoteDocument {
    pub frontmatter: NoteFrontmatter,
    pub body_markdown: String,
}

// ==========================================================================
// NoteRecord
// ==========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct NoteRecord {
    pub note_id: NoteId,
    pub current_path: RelativePath,
    pub content_hash: ContentHash,
    pub created_at: String,
    pub updated_at: String,
    pub trashed_at: Option<String>,
    pub deleted_at: Option<String>,
    pub is_trashed: bool,
    pub is_deleted: bool,
    pub last_editor_device_id: Option<DeviceId>,
    pub server_revision: ServerRevision,
}
