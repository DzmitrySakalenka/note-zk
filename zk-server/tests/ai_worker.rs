// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    fmt::Write as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::{
    sync::oneshot,
    time::{advance, timeout},
};
use tokio_util::sync::CancellationToken;
use zk_ai::{
    AiError, AiResult,
    provider::{EmbeddingsCapability, EmbeddingsProvider, EmbeddingsRequest, EmbeddingsResponse},
    selector::{ProviderRegistry, ProviderSelector},
};
use zk_app::{RebuildOptions, bootstrap_index_connection, rebuild_index};
use zk_domain::{
    EmbeddingModelId, EmbeddingVector, ExecutionMode, JobId, ProviderId, ProviderSelectorPolicy,
    config::{EffectiveConfig, EffectiveWikiConfig},
    id::{ContentHash, DeviceId, NoteId, RelativePath},
};
use zk_server::{
    ServerDb,
    ai::lint_wiki::{WikiLintFindingKind, WikiLintSeverity},
    ai::worker::{AiWorkerDeps, spawn_ai_job_worker},
    config::{AiConfig, AiProviderKind},
    repo::{
        AiJob, AiJobFilter, AiJobKind, AiJobKindFilter, AiJobStatus, AiJobsRepo, ChunksRepo,
        EmbeddingsRepo, NoteRow, NotesRepo, WikiLintFindingsRepo, WikiLintScope,
    },
};

mod common;

use common::{unique_test_dir, write_text};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn recompute_note_job_transitions_in_flight_then_done() {
    let fixture = WorkerFixture::new("ai-worker-success");
    let note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6S");
    fixture.insert_note(&note_id, "notes/a.md", "Filesystem body unique");
    let job = fixture.enqueue(AiJobKind::RecomputeNote {
        note_id: note_id.clone(),
    });
    let (provider, started_rx, release_tx) = BlockingProvider::new();
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(selector_with_provider(Arc::new(provider)), ai_config(3)),
        shutdown.clone(),
    );

    started_rx
        .await
        .expect("provider call should start after job is claimed");
    let in_flight = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::InFlight).await;
    release_tx
        .send(())
        .expect("blocking provider should still be waiting");
    let done = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    assert_eq!(in_flight.attempts, 0);
    assert_eq!(done.attempts, 0);
    let chunks = fixture.chunks_for_note(&note_id);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "Filesystem body unique");
    let embeddings = fixture.embeddings_for_note(&note_id);
    assert_eq!(embeddings, 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn recompute_note_job_uses_server_row_body_when_vault_file_is_absent() {
    let fixture = WorkerFixture::new("ai-worker-row-body");
    let note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6ROW");
    fixture.insert_note_with_row_body(
        &note_id,
        "notes/synced.md",
        "obsolete filesystem body",
        "Server snapshot body unique",
    );
    std::fs::remove_file(fixture.vault_root.join("notes/synced.md"))
        .expect("server vault file should be absent");
    let job = fixture.enqueue(AiJobKind::RecomputeNote {
        note_id: note_id.clone(),
    });
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(
            selector_with_provider(Arc::new(BlockingProvider::ready())),
            ai_config(1),
        ),
        shutdown.clone(),
    );

    let done = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    assert_eq!(done.attempts, 0);
    let chunks = fixture.chunks_for_note(&note_id);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "Server snapshot body unique");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn failing_provider_marks_job_failed_after_max_attempts() {
    let fixture = WorkerFixture::new("ai-worker-provider-fails");
    let note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6T");
    fixture.insert_note(&note_id, "notes/fails.md", "Body that fails");
    let job = fixture.enqueue(AiJobKind::RecomputeNote {
        note_id: note_id.clone(),
    });
    let provider = Arc::new(FailingProvider::default());
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(selector_with_provider(provider.clone()), ai_config(2)),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    assert_eq!(failed.attempts, 2);
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("transient failure")),
        "last_error was {:?}",
        failed.last_error
    );
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn shutdown_mid_job_leaves_current_in_flight_and_next_pending() {
    let fixture = WorkerFixture::new("ai-worker-shutdown-mid-job");
    let first_note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6V");
    let second_note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6W");
    fixture.insert_note(&first_note_id, "notes/first.md", "First body");
    fixture.insert_note(&second_note_id, "notes/second.md", "Second body");
    let first_job = fixture.enqueue_at(
        AiJobKind::RecomputeNote {
            note_id: first_note_id,
        },
        "2026-05-05T10:00:00Z",
    );
    let second_job = fixture.enqueue_at(
        AiJobKind::RecomputeNote {
            note_id: second_note_id,
        },
        "2026-05-05T10:01:00Z",
    );
    let (provider, started_rx, _release_tx) = BlockingProvider::new();
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(selector_with_provider(Arc::new(provider)), ai_config(3)),
        shutdown.clone(),
    );

    started_rx
        .await
        .expect("first provider call should start after claim");
    wait_for_status(&fixture.db, &first_job.job_id, AiJobStatus::InFlight).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after cancellation")
        .expect("worker task should not panic");

    assert_eq!(fixture.job(&first_job.job_id).status, AiJobStatus::InFlight);
    assert_eq!(fixture.job(&second_job.job_id).status, AiJobStatus::Pending);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn startup_recovery_requeues_orphaned_in_flight_jobs() {
    let fixture = WorkerFixture::new("ai-worker-recovery");
    let job = fixture.enqueue(AiJobKind::RecomputeVault);
    fixture
        .db
        .with_conn(|connection| {
            AiJobsRepo::new(connection)
                .claim_next(3, parse_timestamp("2026-05-05T10:01:00Z"))
                .map(|_| ())
        })
        .expect("job claim should succeed");
    let shutdown = CancellationToken::new();
    shutdown.cancel();

    let handle = spawn_ai_job_worker(
        fixture.deps(empty_selector(), ai_config(3)),
        shutdown.clone(),
    );
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after cancelled startup")
        .expect("worker task should not panic");

    let recovered = fixture.job(&job.job_id);
    assert_eq!(recovered.status, AiJobStatus::Pending);
    assert_eq!(recovered.started_at, None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn recompute_vault_enqueues_note_jobs_and_marks_vault_job_done() {
    let fixture = WorkerFixture::new("ai-worker-vault");
    let first_note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6X");
    let second_note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S6Y");
    fixture.insert_note(&first_note_id, "notes/first.md", "First vault body");
    fixture.insert_note(&second_note_id, "notes/second.md", "Second vault body");
    let vault_job = fixture.enqueue(AiJobKind::RecomputeVault);
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(empty_selector(), ai_config(1)),
        shutdown.clone(),
    );

    wait_for_status(&fixture.db, &vault_job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    let note_jobs = fixture.jobs_by_kind(AiJobKindFilter::RecomputeNote);
    assert_eq!(note_jobs.len(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ingest_source_job_fails_without_wiki_config() {
    let fixture = WorkerFixture::new("ai-worker-ingest-source-no-wiki");
    let source_note_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S71");
    let job = fixture.enqueue(AiJobKind::IngestSource {
        source_note_id,
        request_id: "req-ingest-source-worker".to_string(),
    });
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(empty_selector(), ai_config(1)),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    assert_eq!(failed.attempts, 1);
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("wiki config missing")),
        "last_error was {:?}",
        failed.last_error
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn lint_wiki_job_persists_findings_and_marks_done() {
    let fixture = WorkerFixture::new("ai-worker-lint-wiki-success");
    let app_config = fixture.wiki_app_config();
    let page_id = note_id("01JAR5A3PJEZ3SKG67PN8N6S72");
    fixture.insert_note(
        &page_id,
        "wiki/orphan.md",
        &note_file(
            page_id.as_str(),
            "Orphan Page",
            Some("wiki-entity"),
            "# Orphan Page\n",
        ),
    );
    fixture.rebuild_local_index(&app_config);
    let job = fixture.enqueue(AiJobKind::LintWiki {
        scope: WikiLintScope::Vault,
        request_id: "req-lint-wiki-worker-success".to_string(),
    });
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps_with_app_config(empty_selector(), ai_config(1), app_config),
        shutdown.clone(),
    );

    let done = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Done).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");
    let findings = fixture.findings_for_job(&job.job_id);

    assert_eq!(done.attempts, 0);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, WikiLintFindingKind::OrphanPage);
    assert_eq!(findings[0].page_id, Some(page_id));
    assert_eq!(findings[0].source_id, None);
    assert_eq!(findings[0].severity, WikiLintSeverity::Warning);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn lint_wiki_job_fails_without_wiki_config() {
    let fixture = WorkerFixture::new("ai-worker-lint-wiki-no-wiki");
    let job = fixture.enqueue(AiJobKind::LintWiki {
        scope: WikiLintScope::Folder {
            path: RelativePath::new("wiki"),
        },
        request_id: "req-lint-wiki-worker".to_string(),
    });
    let shutdown = CancellationToken::new();
    let handle = spawn_ai_job_worker(
        fixture.deps(empty_selector(), ai_config(1)),
        shutdown.clone(),
    );

    let failed = wait_for_status(&fixture.db, &job.job_id, AiJobStatus::Failed).await;
    shutdown.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("worker should join after shutdown")
        .expect("worker task should not panic");

    assert_eq!(failed.attempts, 1);
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("wiki config missing")),
        "last_error was {:?}",
        failed.last_error
    );
}

struct WorkerFixture {
    db: Arc<ServerDb>,
    vault_root: std::path::PathBuf,
}

impl WorkerFixture {
    fn new(prefix: &str) -> Self {
        let root = unique_test_dir(prefix);
        let vault_root = root.join("vault");
        std::fs::create_dir_all(&vault_root).expect("vault root should be created");

        Self {
            db: Arc::new(ServerDb::open_in_memory().expect("in-memory server db should open")),
            vault_root,
        }
    }

    fn deps(&self, selector: Arc<ProviderSelector>, config: AiConfig) -> AiWorkerDeps {
        self.deps_with_app_config(
            selector,
            config,
            EffectiveConfig {
                vault_path: self.vault_root.clone(),
                ..EffectiveConfig::default()
            },
        )
    }

    fn deps_with_app_config(
        &self,
        selector: Arc<ProviderSelector>,
        config: AiConfig,
        app_config: EffectiveConfig,
    ) -> AiWorkerDeps {
        AiWorkerDeps {
            db: Arc::clone(&self.db),
            selector,
            config,
            vault_root: self.vault_root.clone(),
            app_config,
            worker_device_id: DeviceId::new("server-ai-worker-test"),
        }
    }

    fn wiki_app_config(&self) -> EffectiveConfig {
        EffectiveConfig {
            vault_path: self.vault_root.clone(),
            local_db_path: self.vault_root.join(".zk/index.sqlite"),
            wiki: Some(EffectiveWikiConfig {
                root_dir: RelativePath::new("wiki"),
                raw_dir: RelativePath::new("raw"),
                index_path: RelativePath::new("wiki/index.md"),
                log_path: RelativePath::new("wiki/log.md"),
            }),
            ..EffectiveConfig::default()
        }
    }

    fn insert_note(&self, note_id: &NoteId, relative_path: &str, file_body: &str) {
        self.insert_note_with_row_body(
            note_id,
            relative_path,
            file_body,
            "stale server mirror body",
        );
    }

    fn insert_note_with_row_body(
        &self,
        note_id: &NoteId,
        relative_path: &str,
        file_body: &str,
        row_body: &str,
    ) {
        write_text(&self.vault_root.join(relative_path), file_body);
        let now = parse_timestamp("2026-05-05T10:00:00Z");
        let content_hash = ContentHash::from_bytes(file_body.as_bytes());

        self.db
            .with_conn(|connection| {
                NotesRepo::create(
                    connection,
                    &NoteRow {
                        note_id: note_id.to_string(),
                        current_path: relative_path.to_string(),
                        title: "fixture".to_string(),
                        content_hash: content_hash.to_string(),
                        body_markdown: row_body.to_string(),
                        frontmatter_json: "{}".to_string(),
                        created_at: now,
                        updated_at: now,
                        trashed_at: None,
                        deleted_at: None,
                        is_trashed: false,
                        is_deleted: false,
                        last_editor_device_id: None,
                        server_revision: 1,
                    },
                )
            })
            .expect("note row should insert");
    }

    fn rebuild_local_index(&self, app_config: &EffectiveConfig) {
        let mut connection =
            bootstrap_index_connection(app_config).expect("local index should open");
        rebuild_index(
            &self.vault_root,
            app_config,
            &mut connection,
            RebuildOptions::default(),
        )
        .expect("local index should rebuild");
    }

    fn enqueue(&self, kind: AiJobKind) -> AiJob {
        self.enqueue_at(kind, "2026-05-05T10:00:00Z")
    }

    fn enqueue_at(&self, kind: AiJobKind, timestamp: &str) -> AiJob {
        self.db
            .with_conn(|connection| {
                AiJobsRepo::new(connection).enqueue(kind, parse_timestamp(timestamp))
            })
            .expect("AI job should enqueue")
    }

    fn job(&self, job_id: &JobId) -> AiJob {
        self.db
            .with_conn(|connection| {
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

    fn jobs_by_kind(&self, kind: AiJobKindFilter) -> Vec<AiJob> {
        self.db
            .with_conn(|connection| {
                AiJobsRepo::new(connection).list(AiJobFilter {
                    status: None,
                    kind: Some(kind),
                    limit: None,
                })
            })
            .expect("job list should succeed")
    }

    fn chunks_for_note(&self, note_id: &NoteId) -> Vec<zk_domain::ChunkRecord> {
        self.db
            .with_conn(|connection| ChunksRepo::new(connection).list_for_note(note_id))
            .expect("chunks should list")
    }

    fn embeddings_for_note(&self, note_id: &NoteId) -> usize {
        self.db
            .with_conn(|connection| {
                let chunks = ChunksRepo::new(connection).list_for_note(note_id)?;
                let mut count = 0_usize;

                for chunk in chunks {
                    count += EmbeddingsRepo::new(connection)
                        .list_for_chunk(&chunk.chunk_id)?
                        .len();
                }

                Ok(count)
            })
            .expect("embeddings should list")
    }

    fn findings_for_job(&self, job_id: &JobId) -> Vec<zk_server::repo::WikiLintFindingRow> {
        self.db
            .with_conn(|connection| WikiLintFindingsRepo::new(connection).list_for_job(job_id))
            .expect("findings should list")
    }
}

struct BlockingProvider {
    id: ProviderId,
    capability: EmbeddingsCapability,
    started_tx: Mutex<Option<oneshot::Sender<()>>>,
    release_rx: Mutex<Option<oneshot::Receiver<()>>>,
}

impl BlockingProvider {
    fn new() -> (Self, oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();

        (
            Self {
                id: provider_id(),
                capability: capability(),
                started_tx: Mutex::new(Some(started_tx)),
                release_rx: Mutex::new(Some(release_rx)),
            },
            started_rx,
            release_tx,
        )
    }

    fn ready() -> Self {
        Self {
            id: provider_id(),
            capability: capability(),
            started_tx: Mutex::new(None),
            release_rx: Mutex::new(None),
        }
    }
}

#[async_trait]
impl EmbeddingsProvider for BlockingProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capability(&self) -> &EmbeddingsCapability {
        &self.capability
    }

    async fn embed_batch(&self, request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        if let Some(started_tx) = self
            .started_tx
            .lock()
            .expect("started lock should not be poisoned")
            .take()
        {
            let _ = started_tx.send(());
        }
        let release_rx = self
            .release_rx
            .lock()
            .expect("release lock should not be poisoned")
            .take();

        if let Some(release_rx) = release_rx {
            let _ = release_rx.await;
        }

        Ok(response_for_request(&request))
    }
}

#[derive(Default)]
struct FailingProvider {
    requests: AtomicUsize,
    id: std::sync::OnceLock<ProviderId>,
    capability: std::sync::OnceLock<EmbeddingsCapability>,
}

impl FailingProvider {
    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EmbeddingsProvider for FailingProvider {
    fn id(&self) -> &ProviderId {
        self.id.get_or_init(provider_id)
    }

    fn capability(&self) -> &EmbeddingsCapability {
        self.capability.get_or_init(capability)
    }

    async fn embed_batch(&self, _request: EmbeddingsRequest) -> AiResult<EmbeddingsResponse> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        Err(AiError::ProviderRequest("transient failure".to_string()))
    }
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

    let job = job_by_id(db, job_id);
    panic!(
        "job {job_id} did not reach status {status:?}; final status {:?}, last_error {:?}",
        job.status, job.last_error
    );
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

fn selector_with_provider(provider: Arc<dyn EmbeddingsProvider>) -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();
    registry.register_embeddings(provider);

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

fn empty_selector() -> Arc<ProviderSelector> {
    Arc::new(ProviderSelector::new(
        Arc::new(ProviderRegistry::new()),
        ProviderSelectorPolicy {
            mode: ExecutionMode::Connected,
            remote_provider: Some(provider_id()),
            local_provider: None,
            stub_provider: None,
        },
    ))
}

fn ai_config(max_attempts: u32) -> AiConfig {
    AiConfig {
        provider: AiProviderKind::Stub,
        remote: None,
        local: None,
        stub: None,
        chat: None,
        default_mode: ExecutionMode::Hybrid,
        recompute_concurrency: 1,
        max_attempts,
    }
}

fn response_for_request(request: &EmbeddingsRequest) -> EmbeddingsResponse {
    EmbeddingsResponse {
        vectors: request
            .texts
            .iter()
            .map(|_| EmbeddingVector {
                values: vec![1.0, 0.0],
                model: request.model.clone(),
            })
            .collect(),
    }
}

fn capability() -> EmbeddingsCapability {
    EmbeddingsCapability {
        model: model_id(),
        dim: 2,
        max_batch_size: 8,
        max_input_tokens: 512,
    }
}

fn provider_id() -> ProviderId {
    "test-provider"
        .parse()
        .expect("provider fixture should parse")
}

fn model_id() -> EmbeddingModelId {
    "test/model".parse().expect("model fixture should parse")
}

fn note_id(value: &str) -> NoteId {
    NoteId::new(value)
}

fn note_file(note_id: &str, title: &str, note_type: Option<&str>, body: &str) -> String {
    let mut yaml = format!(
        "id: {note_id}\ntitle: {title}\ntags:\n  - wiki\ncreated: 2026-05-05T10:00:00Z\nupdated: 2026-05-05T10:00:00Z\n"
    );
    if let Some(note_type) = note_type {
        let _ = writeln!(yaml, "type: {note_type}");
    }

    format!("---\n{yaml}---\n\n{body}")
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}
