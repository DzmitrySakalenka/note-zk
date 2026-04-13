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
