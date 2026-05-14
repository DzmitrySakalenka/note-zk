//! Pure LLM Wiki ingest prompt assembly and provider response parsing.

use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;
use zk_ai::{
    AiError,
    provider::{ChatChunk, ChatMessage, ChatRequest},
    selector::ProviderSelector,
};
use zk_domain::{
    ExecutionMode, MessageRole,
    id::RelativePath,
    note::NoteRecord,
    write::{CreateNoteRequest, UpdateNoteRequest},
};

const SYSTEM_PROMPT: &str = r"You are the LLM Wiki ingest planner.

Return only valid JSON matching the documented WikiIngestPlan schema. Do not include markdown fences, prose, comments, or trailing text outside the JSON object.";

/// JSON object returned by the provider.
///
/// The provider must emit only one JSON object with this shape:
///
/// ```json
/// {
///   "creates": [
///     {
///       "path": "wiki/example.md",
///       "frontmatter": {
///         "id": "01...",
///         "title": "Example",
///         "tags": ["wiki"],
///         "type": "wiki-entity",
///         "sources": ["01SOURCE..."]
///       },
///       "body_markdown": "# Example\n\nBody"
///     }
///   ],
///   "updates": [
///     {
///       "note_id": "01EXISTING...",
///       "frontmatter": null,
///       "body_markdown": "# Existing\n\nUpdated body"
///     }
///   ]
/// }
/// ```
pub const WIKI_INGEST_OUTPUT_SCHEMA: &str = r##"{
  "creates": [
    {
      "path": "wiki/example.md",
      "frontmatter": {
        "id": "01...",
        "title": "Example",
        "tags": ["wiki"],
        "type": "wiki-entity",
        "sources": ["01SOURCE..."]
      },
      "body_markdown": "# Example\n\nBody"
    }
  ],
  "updates": [
    {
      "note_id": "01EXISTING...",
      "frontmatter": null,
      "body_markdown": "# Existing\n\nUpdated body"
    }
  ]
}"##;

/// Deterministic vocabulary used to constrain wiki ingest planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiVocabulary {
    /// Effective vault-relative wiki root.
    pub root_dir: RelativePath,
    /// Type value for entity pages.
    pub entity_type: String,
    /// Type value for synthesis pages.
    pub synthesis_type: String,
    /// Type value for the derived index page.
    pub index_type: String,
    /// Type value for the derived log page.
    pub log_type: String,
}

impl WikiVocabulary {
    /// Creates a vocabulary with the standard LLM Wiki page type names.
    #[must_use]
    pub fn new(root_dir: RelativePath) -> Self {
        Self {
            root_dir,
            entity_type: "wiki-entity".to_string(),
            synthesis_type: "wiki-synthesis".to_string(),
            index_type: "wiki-index".to_string(),
            log_type: "wiki-log".to_string(),
        }
    }
}

/// Parsed provider plan for creating and updating wiki notes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WikiIngestPlan {
    /// New wiki notes to create.
    pub creates: Vec<CreateNoteRequest>,
    /// Existing wiki notes to update.
    pub updates: Vec<UpdateNoteRequest>,
}

/// Error returned by the pure wiki ingest service.
#[derive(Debug, thiserror::Error)]
pub enum WikiIngestError {
    /// Chat provider selection or completion failed.
    #[error(transparent)]
    Provider(#[from] AiError),
    /// The provider returned text that did not decode to `WikiIngestPlan`.
    #[error("provider output malformed")]
    ProviderOutputMalformed {
        /// Raw provider text that failed JSON parsing or schema validation.
        raw_response: String,
    },
}

/// Pure orchestration service for LLM Wiki source ingestion.
pub struct IngestSourceService {
    selector: Arc<ProviderSelector>,
    execution_mode: ExecutionMode,
    request_id: String,
}

impl IngestSourceService {
    /// Creates a service backed by an existing provider selector.
    #[must_use]
    pub fn new(
        selector: Arc<ProviderSelector>,
        execution_mode: ExecutionMode,
        request_id: String,
    ) -> Self {
        Self {
            selector,
            execution_mode,
            request_id,
        }
    }

    /// Builds an ingest plan from one source note and current wiki context.
    ///
    /// # Errors
    ///
    /// Returns [`WikiIngestError`] when provider selection/completion fails or
    /// when the provider output is not a valid [`WikiIngestPlan`].
    pub async fn ingest_source(
        &self,
        source: &NoteRecord,
        source_body: &str,
        existing_wiki_titles: &[String],
        vocabulary: &WikiVocabulary,
    ) -> Result<WikiIngestPlan, WikiIngestError> {
        let provider = self.selector.select_chat(self.execution_mode)?;
        let request = ChatRequest {
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: SYSTEM_PROMPT.to_string(),
                },
                ChatMessage {
                    role: MessageRole::User,
                    content: assemble_user_prompt(
                        source,
                        source_body,
                        existing_wiki_titles,
                        vocabulary,
                    ),
                },
            ],
            retrieval: None,
            max_output_tokens: Some(provider.capability().max_output_tokens),
            temperature: Some(0.0),
            stream: false,
            request_id: self.request_id.clone(),
        };
        let mut stream = provider.complete(request).await?;
        let mut raw_response = String::new();

        while let Some(chunk) = stream.next().await {
            match chunk? {
                ChatChunk::Delta { text } => raw_response.push_str(&text),
                ChatChunk::Citation(_) | ChatChunk::Done { .. } => {}
            }
        }

        serde_json::from_str(&raw_response).map_err(|_| WikiIngestError::ProviderOutputMalformed {
            raw_response: raw_response.clone(),
        })
    }
}

fn assemble_user_prompt(
    source: &NoteRecord,
    source_body: &str,
    existing_wiki_titles: &[String],
    vocabulary: &WikiVocabulary,
) -> String {
    format!(
        "\
Wiki vocabulary:
- root_dir: {root_dir}
- entity_type: {entity_type}
- synthesis_type: {synthesis_type}
- index_type: {index_type}
- log_type: {log_type}

Existing wiki page titles:
{titles}

Source note:
- note_id: {note_id}
- path: {source_path}

Source body:
```markdown
{source_body}
```

Output JSON schema:
```json
{schema}
```",
        root_dir = vocabulary.root_dir.as_str(),
        entity_type = vocabulary.entity_type,
        synthesis_type = vocabulary.synthesis_type,
        index_type = vocabulary.index_type,
        log_type = vocabulary.log_type,
        titles = render_titles(existing_wiki_titles),
        note_id = source.note_id,
        source_path = source.current_path.as_str(),
        schema = WIKI_INGEST_OUTPUT_SCHEMA,
    )
}

fn render_titles(existing_wiki_titles: &[String]) -> String {
    if existing_wiki_titles.is_empty() {
        return "- (none)".to_string();
    }

    let mut titles = existing_wiki_titles.to_vec();
    titles.sort();

    titles
        .into_iter()
        .map(|title| format!("- {title}"))
        .collect::<Vec<_>>()
        .join("\n")
}
