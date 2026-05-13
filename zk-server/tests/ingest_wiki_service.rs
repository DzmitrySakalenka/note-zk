// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::{stream, stream::BoxStream};
use insta::assert_snapshot;
use zk_ai::{
    AiError, AiResult,
    provider::{
        ChatCapability, ChatChunk, ChatMessage, ChatProvider, ChatRequest, ChatUsage, FinishReason,
    },
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_domain::{
    EmbeddingModelId, ExecutionMode, MessageRole, ProviderSelectorPolicy,
    id::{ContentHash, NoteId, RelativePath, ServerRevision},
    note::{NoteFrontmatter, NoteRecord},
    write::{CreateNoteRequest, UpdateNoteRequest},
};
use zk_server::ai::ingest_wiki::{
    IngestSourceService, WikiIngestError, WikiIngestPlan, WikiVocabulary,
};

#[tokio::test]
async fn assembled_prompt_matches_snapshot() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = service_with_provider(RecordingChatProvider::success(
        requests.clone(),
        r#"{"creates":[],"updates":[]}"#,
    ));

    service
        .ingest_source(
            &source_record(),
            "# Source\n\nAlpha cites beta.",
            &["Beta".to_string(), "Alpha".to_string()],
            &wiki_vocabulary(),
        )
        .await
        .expect("ingest plan should parse");
    let request = take_request(&requests);

    assert!(!request.stream);
    assert_eq!(request.request_id, "req-ingest-source-service");
    assert_eq!(request.temperature, Some(0.0));
    assert_snapshot!(render_request(&request), @r##"
request_id: req-ingest-source-service
stream: false
temperature: Some(0.0)
max_output_tokens: Some(1024)

[system]
You are the LLM Wiki ingest planner.

Return only valid JSON matching the documented WikiIngestPlan schema. Do not include markdown fences, prose, comments, or trailing text outside the JSON object.

[user]
Wiki vocabulary:
- root_dir: wiki
- entity_type: wiki-entity
- synthesis_type: wiki-synthesis
- index_type: wiki-index
- log_type: wiki-log

Existing wiki page titles:
- Alpha
- Beta

Source note:
- note_id: 01SOURCE000000000000000001
- path: raw/source.md

Source body:
```markdown
# Source

Alpha cites beta.
```

Output JSON schema:
```json
{
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
}
```
"##);
}

#[tokio::test]
async fn fixture_json_parses_to_ingest_plan() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = service_with_provider(RecordingChatProvider::success(
        requests,
        r##"{
  "creates": [
    {
      "path": "wiki/alpha.md",
      "frontmatter": {
        "id": "01WIKICREATE000000000000001",
        "title": "Alpha",
        "tags": ["wiki"],
        "type": "wiki-entity",
        "sources": ["01SOURCE000000000000000001"]
      },
      "body_markdown": "# Alpha\n\nNew page."
    }
  ],
  "updates": [
    {
      "note_id": "01WIKIUPDATE000000000000001",
      "frontmatter": null,
      "body_markdown": "# Existing\n\nUpdated page."
    }
  ]
}"##,
    ));

    let plan = service
        .ingest_source(
            &source_record(),
            "# Source\n\nAlpha cites beta.",
            &["Alpha".to_string()],
            &wiki_vocabulary(),
        )
        .await
        .expect("ingest plan should parse");

    assert_eq!(
        plan,
        WikiIngestPlan {
            creates: vec![CreateNoteRequest {
                path: RelativePath::new("wiki/alpha.md"),
                frontmatter: NoteFrontmatter {
                    id: NoteId::new("01WIKICREATE000000000000001"),
                    title: "Alpha".to_string(),
                    tags: vec!["wiki".to_string()],
                    note_type: Some("wiki-entity".to_string()),
                    aliases: Vec::new(),
                    created: None,
                    updated: None,
                    source: None,
                    status: None,
                    sources: vec![NoteId::new("01SOURCE000000000000000001")],
                    extra: BTreeMap::new(),
                },
                body_markdown: "# Alpha\n\nNew page.".to_string(),
            }],
            updates: vec![UpdateNoteRequest {
                note_id: NoteId::new("01WIKIUPDATE000000000000001"),
                frontmatter: None,
                body_markdown: Some("# Existing\n\nUpdated page.".to_string()),
            }],
        }
    );
}

#[tokio::test]
async fn truncated_json_maps_to_provider_output_malformed() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service =
        service_with_provider(RecordingChatProvider::success(requests, r#"{"creates":["#));

    let error = service
        .ingest_source(&source_record(), "Body", &[], &wiki_vocabulary())
        .await
        .expect_err("truncated JSON should fail");

    assert!(matches!(
        error,
        WikiIngestError::ProviderOutputMalformed { raw_response }
            if raw_response == r#"{"creates":["#
    ));
}

#[tokio::test]
async fn wrong_schema_maps_to_provider_output_malformed() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = service_with_provider(RecordingChatProvider::success(
        requests,
        r#"{"creates":[{"path":"wiki/missing.md"}],"updates":[]}"#,
    ));

    let error = service
        .ingest_source(&source_record(), "Body", &[], &wiki_vocabulary())
        .await
        .expect_err("wrong schema should fail");

    assert!(matches!(
        error,
        WikiIngestError::ProviderOutputMalformed { raw_response }
            if raw_response == r#"{"creates":[{"path":"wiki/missing.md"}],"updates":[]}"#
    ));
}

#[tokio::test]
async fn provider_error_surfaces_without_panic() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = service_with_provider(RecordingChatProvider::failure(
        requests,
        AiError::ProviderRequest("provider failed safely".to_string()),
    ));

    let error = service
        .ingest_source(&source_record(), "Body", &[], &wiki_vocabulary())
        .await
        .expect_err("provider failure should surface");

    assert!(matches!(
        error,
        WikiIngestError::Provider(AiError::ProviderRequest(message))
            if message == "provider failed safely"
    ));
}

struct RecordingChatProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    response: Mutex<Option<AiResult<String>>>,
}

impl RecordingChatProvider {
    fn success(requests: Arc<Mutex<Vec<ChatRequest>>>, response: &str) -> Self {
        Self {
            requests,
            response: Mutex::new(Some(Ok(response.to_string()))),
        }
    }

    fn failure(requests: Arc<Mutex<Vec<ChatRequest>>>, error: AiError) -> Self {
        Self {
            requests,
            response: Mutex::new(Some(Err(error))),
        }
    }
}

#[async_trait]
impl ChatProvider for RecordingChatProvider {
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        self.requests
            .lock()
            .expect("request lock should not be poisoned")
            .push(request);
        let response = self
            .response
            .lock()
            .expect("response lock should not be poisoned")
            .take()
            .expect("test provider should be called once")?;

        Ok(Box::pin(stream::iter([
            Ok(ChatChunk::Delta { text: response }),
            Ok(ChatChunk::Done {
                finish_reason: FinishReason::Stop,
                usage: ChatUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            }),
        ])))
    }

    fn capability(&self) -> ChatCapability {
        ChatCapability {
            model: chat_model_id(),
            max_input_tokens: 8192,
            max_output_tokens: 1024,
            supports_streaming: false,
        }
    }
}

fn service_with_provider(provider: RecordingChatProvider) -> IngestSourceService {
    let mut registry = ProviderRegistry::new();
    registry.register_chat(Arc::new(provider));
    let selector = Arc::new(ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Hybrid,
            remote_provider: None,
            local_provider: None,
            stub_provider: None,
        },
    ));

    IngestSourceService::new(
        selector,
        ExecutionMode::Hybrid,
        "req-ingest-source-service".to_string(),
    )
}

fn take_request(requests: &Arc<Mutex<Vec<ChatRequest>>>) -> ChatRequest {
    let mut requests = requests
        .lock()
        .expect("request lock should not be poisoned");

    assert_eq!(requests.len(), 1);
    requests.pop().expect("request should exist")
}

fn render_request(request: &ChatRequest) -> String {
    let mut rendered = format!(
        "request_id: {}\nstream: {}\ntemperature: {:?}\nmax_output_tokens: {:?}\n",
        request.request_id, request.stream, request.temperature, request.max_output_tokens
    );

    for message in &request.messages {
        rendered.push('\n');
        rendered.push('[');
        rendered.push_str(message_role_name(message));
        rendered.push_str("]\n");
        rendered.push_str(&message.content);
        rendered.push('\n');
    }

    rendered
}

fn message_role_name(message: &ChatMessage) -> &'static str {
    match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    }
}

fn source_record() -> NoteRecord {
    NoteRecord {
        note_id: NoteId::new("01SOURCE000000000000000001"),
        current_path: RelativePath::new("raw/source.md"),
        content_hash: ContentHash::from_bytes(b"source"),
        created_at: "2026-05-05T10:00:00Z".to_string(),
        updated_at: "2026-05-05T10:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: ServerRevision::new(1),
    }
}

fn wiki_vocabulary() -> WikiVocabulary {
    WikiVocabulary::new(RelativePath::new("wiki/"))
}

fn chat_model_id() -> EmbeddingModelId {
    "test/wiki-ingest-chat"
        .parse()
        .expect("chat model id should parse")
}
