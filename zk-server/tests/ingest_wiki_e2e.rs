// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::{self, BoxStream};
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;
use zk_ai::{
    AiError, AiResult,
    provider::{ChatCapability, ChatChunk, ChatProvider, ChatRequest, StubEmbeddingsProvider},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_app::{RebuildOptions, bootstrap_index_connection, rebuild_index};
use zk_domain::{
    EmbeddingModelId, ExecutionMode, JobId, ProviderId, ProviderSelectorPolicy,
    config::{EffectiveConfig, EffectiveWikiConfig, FolderPolicy},
    id::{ContentHash, DeviceId, NoteId, RelativePath},
    note::NoteFrontmatter,
};
use zk_server::{
    ServerDb,
    ai::worker::{AiWorkerDeps, spawn_ai_job_worker},
    config::{AiChatConfig, AiConfig, AiProviderKind},
    repo::{
        AiJob, AiJobFilter, AiJobKind, AiJobKindFilter, AiJobStatus, AiJobsRepo, NoteRow, NotesRepo,
    },
};
use zk_vault::{NormalizationContext, analyze_body, normalize_frontmatter, parse_note_file};

mod common;

use common::{unique_test_dir, write_text};

const SOURCE_ID: &str = "01SOURCE000000000000000001";
const OTHER_SOURCE_ID: &str = "01SOURCE000000000000000002";
const EXISTING_ID: &str = "01WIKIEXISTING00000000001";
const CREATED_ID: &str = "01WIKICREATED000000000001";
const INDEX_ID: &str = "01WIKIINDEX0000000000001";
const LOG_ID: &str = "01WIKILOG00000000000001";

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ingest_source_worker_applies_plan_updates_meta_and_enqueues_recompute() {
    let fixture = IngestFixture::new("ingest-wiki-worker-success");
    fixture.seed_standard_vault();
    fixture.rebuild_local_index();
    fixture.seed_server_notes(&[
        "raw/source.md",
        "raw/other.md",
        "wiki/existing.md",
        "wiki/index.md",
        "wiki/log.md",
    ]);
    let job = fixture.enqueue_ingest("req-ingest-source-e2e");
    let shutdown = CancellationToken::new();
    let handle = fixture.spawn_worker(
        selector_with_chat(Arc::new(ScriptedChatProvider::success(success_plan_json()))),
        shutdown.clone(),
    );

    let done = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    join_worker(handle).await;

    assert_eq!(done.attempts, 0);
    assert_eq!(
        fixture.frontmatter("wiki/beta.md").sources,
        vec![NoteId::new(SOURCE_ID)]
    );
    assert_eq!(
        fixture.frontmatter("wiki/existing.md").sources,
        vec![NoteId::new(OTHER_SOURCE_ID), NoteId::new(SOURCE_ID)]
    );
    assert_eq!(
        fixture.local_sources_for(&NoteId::new(CREATED_ID)),
        vec![NoteId::new(SOURCE_ID)]
    );
    assert_eq!(
        fixture.local_sources_for(&NoteId::new(EXISTING_ID)),
        vec![NoteId::new(OTHER_SOURCE_ID), NoteId::new(SOURCE_ID)]
    );

    let log_body = fixture.body("wiki/log.md");
    assert!(log_body.contains("ingest Source Article -> 2 pages"));
    let index_body = fixture.body("wiki/index.md");
    assert!(index_body.contains("Beta Entity"));
    assert!(index_body.contains("Existing Entity"));
    assert!(index_body.contains(SOURCE_ID));

    let recompute_note_ids = fixture.recompute_note_ids();
    assert_eq!(
        recompute_note_ids,
        vec![NoteId::new(CREATED_ID), NoteId::new(EXISTING_ID)]
    );
    assert!(fixture.server_note(&NoteId::new(CREATED_ID)).is_some());
    assert!(fixture.server_note(&NoteId::new(INDEX_ID)).is_some());

    let duplicate = fixture.enqueue_ingest("req-ingest-source-e2e");
    assert_eq!(duplicate.job_id, job.job_id);
    assert_eq!(duplicate.status, AiJobStatus::Done);
    assert_eq!(fixture.ingest_jobs().len(), 1);
    assert_eq!(
        fixture
            .body("wiki/log.md")
            .matches("ingest Source Article")
            .count(),
        1
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn provider_error_fails_without_writing_wiki_pages() {
    let fixture = IngestFixture::new("ingest-wiki-worker-provider-error");
    fixture.seed_standard_vault();
    fixture.rebuild_local_index();
    fixture.seed_server_notes(&[
        "raw/source.md",
        "raw/other.md",
        "wiki/existing.md",
        "wiki/index.md",
        "wiki/log.md",
    ]);
    let job = fixture.enqueue_ingest("req-provider-error");
    let shutdown = CancellationToken::new();
    let handle = fixture.spawn_worker(
        selector_with_chat(Arc::new(ScriptedChatProvider::failure())),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    join_worker(handle).await;

    assert_eq!(failed.attempts, 1);
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("provider failed safely")),
        "last_error was {:?}",
        failed.last_error
    );
    assert!(!fixture.vault_path("wiki/beta.md").exists());
    assert_eq!(fixture.recompute_note_ids(), Vec::<NoteId>::new());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn malformed_plan_fails_without_writing_wiki_pages() {
    let fixture = IngestFixture::new("ingest-wiki-worker-malformed-plan");
    fixture.seed_standard_vault();
    fixture.rebuild_local_index();
    fixture.seed_server_notes(&[
        "raw/source.md",
        "raw/other.md",
        "wiki/existing.md",
        "wiki/index.md",
        "wiki/log.md",
    ]);
    let job = fixture.enqueue_ingest("req-malformed-plan");
    let shutdown = CancellationToken::new();
    let handle = fixture.spawn_worker(
        selector_with_chat(Arc::new(ScriptedChatProvider::success(r#"{"creates":["#))),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    join_worker(handle).await;

    assert_eq!(failed.attempts, 1);
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("provider output malformed")),
        "last_error was {:?}",
        failed.last_error
    );
    assert!(!fixture.vault_path("wiki/beta.md").exists());
    assert_eq!(fixture.recompute_note_ids(), Vec::<NoteId>::new());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn immutable_wiki_folder_failure_surfaces_without_meta_updates() {
    let mut fixture = IngestFixture::new("ingest-wiki-worker-immutable");
    fixture.mark_wiki_immutable();
    fixture.seed_standard_vault();
    fixture.rebuild_local_index();
    fixture.seed_server_notes(&[
        "raw/source.md",
        "raw/other.md",
        "wiki/existing.md",
        "wiki/index.md",
        "wiki/log.md",
    ]);
    let job = fixture.enqueue_ingest("req-immutable-wiki");
    let shutdown = CancellationToken::new();
    let handle = fixture.spawn_worker(
        selector_with_chat(Arc::new(ScriptedChatProvider::success(success_plan_json()))),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    join_worker(handle).await;

    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("folder is immutable")),
        "last_error was {:?}",
        failed.last_error
    );
    assert!(!fixture.vault_path("wiki/beta.md").exists());
    assert!(
        !fixture
            .body("wiki/log.md")
            .contains("ingest Source Article")
    );
    assert_eq!(fixture.recompute_note_ids(), Vec::<NoteId>::new());
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn vault_permission_denied_surfaces_without_recompute_jobs() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = IngestFixture::new("ingest-wiki-worker-permission-denied");
    fixture.seed_standard_vault();
    fixture.rebuild_local_index();
    fixture.seed_server_notes(&[
        "raw/source.md",
        "raw/other.md",
        "wiki/existing.md",
        "wiki/index.md",
        "wiki/log.md",
    ]);
    fs::set_permissions(
        fixture.vault_path("wiki"),
        fs::Permissions::from_mode(0o500),
    )
    .expect("wiki directory should become read-only");
    let job = fixture.enqueue_ingest("req-permission-denied");
    let shutdown = CancellationToken::new();
    let handle = fixture.spawn_worker(
        selector_with_chat(Arc::new(ScriptedChatProvider::success(success_plan_json()))),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    join_worker(handle).await;
    fs::set_permissions(
        fixture.vault_path("wiki"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("wiki directory should become writable again");

    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("batch write failed")),
        "last_error was {:?}",
        failed.last_error
    );
    assert!(!fixture.vault_path("wiki/beta.md").exists());
    assert!(
        !fixture
            .body("wiki/log.md")
            .contains("ingest Source Article")
    );
    assert_eq!(fixture.recompute_note_ids(), Vec::<NoteId>::new());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn vault_write_error_surfaces_without_recompute_jobs() {
    let fixture = IngestFixture::new("ingest-wiki-worker-write-error");
    fixture.seed_standard_vault();
    write_text(&fixture.vault_path("wiki/blocked"), "not a directory");
    fixture.rebuild_local_index();
    fixture.seed_server_notes(&[
        "raw/source.md",
        "raw/other.md",
        "wiki/existing.md",
        "wiki/index.md",
        "wiki/log.md",
    ]);
    let job = fixture.enqueue_ingest("req-write-error");
    let shutdown = CancellationToken::new();
    let handle = fixture.spawn_worker(
        selector_with_chat(Arc::new(ScriptedChatProvider::success(
            blocked_path_plan_json(),
        ))),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    join_worker(handle).await;

    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("batch write failed")),
        "last_error was {:?}",
        failed.last_error
    );
    assert_eq!(fixture.recompute_note_ids(), Vec::<NoteId>::new());
}

struct IngestFixture {
    db: Arc<ServerDb>,
    root: std::path::PathBuf,
    vault_root: std::path::PathBuf,
    app_config: EffectiveConfig,
}

impl IngestFixture {
    fn new(prefix: &str) -> Self {
        let root = unique_test_dir(prefix);
        let vault_root = root.join("vault");
        fs::create_dir_all(&vault_root).expect("vault root should be created");
        let app_config = app_config(&root, &vault_root, false);

        Self {
            db: Arc::new(ServerDb::open_in_memory().expect("in-memory server db should open")),
            root,
            vault_root,
            app_config,
        }
    }

    fn mark_wiki_immutable(&mut self) {
        self.app_config = app_config(&self.root, &self.vault_root, true);
    }

    fn spawn_worker(
        &self,
        selector: Arc<ProviderSelector>,
        shutdown: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        spawn_ai_job_worker(
            AiWorkerDeps {
                db: Arc::clone(&self.db),
                selector,
                config: ai_config(),
                vault_root: self.vault_root.clone(),
                app_config: self.app_config.clone(),
                worker_device_id: DeviceId::new("server-ai-worker-test"),
            },
            shutdown,
        )
    }

    fn seed_standard_vault(&self) {
        write_text(
            &self.vault_path("raw/source.md"),
            &note_file(
                SOURCE_ID,
                "Source Article",
                None,
                &[],
                "# Source Article\n\nRaw facts.",
            ),
        );
        write_text(
            &self.vault_path("raw/other.md"),
            &note_file(
                OTHER_SOURCE_ID,
                "Other Source",
                None,
                &[],
                "# Other Source\n",
            ),
        );
        write_text(
            &self.vault_path("wiki/existing.md"),
            &note_file(
                EXISTING_ID,
                "Existing Entity",
                Some("wiki-entity"),
                &[OTHER_SOURCE_ID],
                "# Existing Entity\n\nOld body.",
            ),
        );
        write_text(
            &self.vault_path("wiki/index.md"),
            &note_file(
                INDEX_ID,
                "Wiki Index",
                Some("wiki-index"),
                &[],
                "# Wiki Index\n",
            ),
        );
        write_text(
            &self.vault_path("wiki/log.md"),
            &note_file(LOG_ID, "Wiki Log", Some("wiki-log"), &[], "# Wiki Log\n"),
        );
    }

    fn rebuild_local_index(&self) {
        let mut connection =
            bootstrap_index_connection(&self.app_config).expect("local index should open");
        rebuild_index(
            &self.vault_root,
            &self.app_config,
            &mut connection,
            RebuildOptions::default(),
        )
        .expect("local index should rebuild");
    }

    fn seed_server_notes(&self, paths: &[&str]) {
        for (index, path) in paths.iter().enumerate() {
            self.seed_server_note(
                path,
                i64::try_from(index + 1).expect("index should fit i64"),
            );
        }
    }

    fn seed_server_note(&self, path: &str, revision: i64) {
        let row = note_row_from_file(&self.vault_root, &self.app_config, path, revision);
        self.db
            .with_conn(|connection| NotesRepo::create(connection, &row))
            .expect("server note row should insert");
    }

    fn enqueue_ingest(&self, request_id: &str) -> AiJob {
        self.db
            .with_conn(|connection| {
                AiJobsRepo::new(connection).enqueue(
                    AiJobKind::IngestSource {
                        source_note_id: NoteId::new(SOURCE_ID),
                        request_id: request_id.to_string(),
                    },
                    parse_timestamp("2026-05-05T10:01:00Z"),
                )
            })
            .expect("ingest-source job should enqueue")
    }

    fn ingest_jobs(&self) -> Vec<AiJob> {
        self.db
            .with_conn(|connection| {
                AiJobsRepo::new(connection).list(AiJobFilter {
                    status: None,
                    kind: Some(AiJobKindFilter::IngestSource),
                    limit: None,
                })
            })
            .expect("ingest-source jobs should list")
    }

    fn recompute_note_ids(&self) -> Vec<NoteId> {
        let mut ids = self
            .db
            .with_conn(|connection| {
                AiJobsRepo::new(connection)
                    .list(AiJobFilter {
                        status: None,
                        kind: Some(AiJobKindFilter::RecomputeNote),
                        limit: None,
                    })
                    .map(|jobs| {
                        jobs.into_iter()
                            .filter_map(|job| match job.kind {
                                AiJobKind::RecomputeNote { note_id } => Some(note_id),
                                AiJobKind::RecomputeVault
                                | AiJobKind::ChatTurn { .. }
                                | AiJobKind::IngestSource { .. } => None,
                            })
                            .collect::<Vec<_>>()
                    })
            })
            .expect("recompute jobs should list");
        ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        ids
    }

    fn local_sources_for(&self, note_id: &NoteId) -> Vec<NoteId> {
        let connection =
            bootstrap_index_connection(&self.app_config).expect("local index should open");
        let mut statement = connection
            .prepare("SELECT source_note_id FROM note_sources WHERE note_id = ?1 ORDER BY rowid")
            .expect("source query should prepare");
        let rows = statement
            .query_map([note_id.as_str()], |row| row.get::<_, String>(0))
            .expect("source query should run");

        rows.map(|row| NoteId::new(&row.expect("source row should decode")))
            .collect()
    }

    fn server_note(&self, note_id: &NoteId) -> Option<NoteRow> {
        self.db
            .with_conn(|connection| NotesRepo::get(connection, note_id.as_str()))
            .expect("server note lookup should succeed")
    }

    fn frontmatter(&self, path: &str) -> NoteFrontmatter {
        normalize_note_file(&self.vault_root, &self.app_config, path).0
    }

    fn body(&self, path: &str) -> String {
        parse_note_file(&self.vault_path(path))
            .expect("note should parse")
            .body_markdown
    }

    fn vault_path(&self, path: &str) -> std::path::PathBuf {
        self.vault_root.join(path)
    }
}

struct ScriptedChatProvider {
    calls: AtomicUsize,
    response: AiResult<String>,
}

impl ScriptedChatProvider {
    fn success(response: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            response: Ok(response.to_string()),
        }
    }

    fn failure() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            response: Err(AiError::ProviderRequest(
                "provider failed safely".to_string(),
            )),
        }
    }
}

#[async_trait]
impl ChatProvider for ScriptedChatProvider {
    async fn complete(
        &self,
        _request: ChatRequest,
    ) -> AiResult<BoxStream<'static, AiResult<ChatChunk>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = match &self.response {
            Ok(response) => response.clone(),
            Err(error) => return Err(AiError::ProviderRequest(error.to_string())),
        };

        Ok(Box::pin(stream::iter([
            Ok(ChatChunk::Delta { text: response }),
            Ok(ChatChunk::Done {
                finish_reason: zk_ai::provider::FinishReason::Stop,
                usage: zk_ai::provider::ChatUsage {
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

fn app_config(root: &Path, vault_root: &Path, wiki_immutable: bool) -> EffectiveConfig {
    let mut folders = BTreeMap::new();
    folders.insert(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
            derived: None,
        },
    );
    folders.insert(
        "wiki/index.md".to_string(),
        FolderPolicy {
            immutable: None,
            derived: Some(true),
        },
    );
    folders.insert(
        "wiki/log.md".to_string(),
        FolderPolicy {
            immutable: None,
            derived: Some(true),
        },
    );
    if wiki_immutable {
        folders.insert(
            "wiki".to_string(),
            FolderPolicy {
                immutable: Some(true),
                derived: None,
            },
        );
    }

    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        local_db_path: std::env::current_dir()
            .expect("current directory should resolve")
            .join(root)
            .join("index.sqlite3"),
        folders,
        wiki: Some(EffectiveWikiConfig {
            root_dir: RelativePath::new("wiki"),
            raw_dir: RelativePath::new("raw"),
            index_path: RelativePath::new("wiki/index.md"),
            log_path: RelativePath::new("wiki/log.md"),
        }),
        ..EffectiveConfig::default()
    }
}

fn note_row_from_file(
    vault_root: &Path,
    config: &EffectiveConfig,
    path: &str,
    revision: i64,
) -> NoteRow {
    let absolute_path = vault_root.join(path);
    let file_bytes = fs::read(&absolute_path).expect("note bytes should read");
    let (frontmatter, body_markdown) = normalize_note_file(vault_root, config, path);
    let now = parse_timestamp("2026-05-05T10:00:00Z");

    NoteRow {
        note_id: frontmatter.id.to_string(),
        current_path: path.to_string(),
        title: frontmatter.title.clone(),
        content_hash: ContentHash::from_bytes(&file_bytes).to_string(),
        body_markdown,
        frontmatter_json: serde_json::to_string(&frontmatter)
            .expect("frontmatter JSON should serialize"),
        created_at: now,
        updated_at: now,
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: revision,
    }
}

fn normalize_note_file(
    vault_root: &Path,
    config: &EffectiveConfig,
    path: &str,
) -> (NoteFrontmatter, String) {
    let relative_path = RelativePath::new(path);
    let parsed = parse_note_file(&vault_root.join(path)).expect("note should parse");
    let raw_frontmatter = parsed.frontmatter.expect("note should have frontmatter");
    let body_analysis = analyze_body(&parsed.body_markdown);
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config,
        is_immutable: false,
    };
    let (frontmatter, _) = normalize_frontmatter(raw_frontmatter, &context);

    (frontmatter, parsed.body_markdown)
}

fn note_file(
    note_id: &str,
    title: &str,
    note_type: Option<&str>,
    sources: &[&str],
    body: &str,
) -> String {
    let mut yaml = format!("id: {note_id}\ntitle: {title}\ntags:\n  - wiki\n");
    if let Some(note_type) = note_type {
        let _ = writeln!(yaml, "type: {note_type}");
    }
    if !sources.is_empty() {
        yaml.push_str("sources:\n");
        for source in sources {
            let _ = writeln!(yaml, "  - {source}");
        }
    }

    format!("---\n{yaml}---\n{body}\n")
}

fn success_plan_json() -> &'static str {
    r##"{
  "creates": [
    {
      "path": "wiki/beta.md",
      "frontmatter": {
        "id": "01WIKICREATED000000000001",
        "title": "Beta Entity",
        "tags": ["wiki"],
        "type": "wiki-entity",
        "sources": []
      },
      "body_markdown": "# Beta Entity\n\nNew body."
    }
  ],
  "updates": [
    {
      "note_id": "01WIKIEXISTING00000000001",
      "frontmatter": {
        "id": "01WIKIEXISTING00000000001",
        "title": "Existing Entity",
        "tags": ["wiki"],
        "type": "wiki-entity",
        "sources": []
      },
      "body_markdown": "# Existing Entity\n\nUpdated body."
    }
  ]
}"##
}

fn blocked_path_plan_json() -> &'static str {
    r##"{
  "creates": [
    {
      "path": "wiki/blocked/beta.md",
      "frontmatter": {
        "id": "01WIKICREATED000000000001",
        "title": "Beta Entity",
        "tags": ["wiki"],
        "type": "wiki-entity",
        "sources": []
      },
      "body_markdown": "# Beta Entity\n\nNew body."
    }
  ],
  "updates": []
}"##
}

async fn wait_for_status(db: &Arc<ServerDb>, job_id: &JobId, status: AiJobStatus) -> AiJob {
    for _ in 0..100 {
        let job = job_by_id(db, job_id);

        if job.status == status {
            return job;
        }

        tokio::task::yield_now().await;
        advance(Duration::from_millis(10)).await;
    }

    panic!("job {job_id} did not reach status {status:?}");
}

fn job_by_id(db: &Arc<ServerDb>, job_id: &JobId) -> AiJob {
    db.with_conn(|connection| {
        AiJobsRepo::new(connection)
            .list(AiJobFilter {
                status: None,
                kind: None,
                limit: None,
            })
            .map(|jobs| {
                jobs.into_iter()
                    .find(|job| job.job_id == *job_id)
                    .expect("job should exist")
            })
    })
    .expect("job list should succeed")
}

async fn join_worker(handle: tokio::task::JoinHandle<()>) {
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");
}

fn selector_with_chat(provider: Arc<dyn ChatProvider>) -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(2)));
    registry.register_chat(provider);

    Arc::new(ProviderSelector::new(
        Arc::new(registry),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Hybrid,
            remote_provider: None,
            local_provider: None,
            stub_provider: Some(provider_id()),
        },
    ))
}

fn ai_config() -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: None,
        chat: Some(AiChatConfig::default()),
        default_mode: ExecutionMode::Hybrid,
        recompute_concurrency: 1,
        max_attempts: 1,
    }
}

fn provider_id() -> ProviderId {
    "stub".parse().expect("provider fixture should parse")
}

fn chat_model_id() -> EmbeddingModelId {
    "test/wiki-ingest-chat"
        .parse()
        .expect("chat model fixture should parse")
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
