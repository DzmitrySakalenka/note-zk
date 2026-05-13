//! Tokio task that drains server-side AI background jobs.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use futures::StreamExt;
use rusqlite::Connection;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Level};
use zk_ai::{
    AiError, AiResult,
    chunker::ParagraphChunker,
    provider::{
        LocalEmbeddingsProvider, RemoteEmbeddingsProvider, RemoteHttpChatProvider,
        StubChatProvider, StubEmbeddingsProvider,
    },
    selector::{ProviderRegistry, ProviderSelector},
    service::{
        chat::ChatTurnRequest,
        chunking::{ChunkingService, ChunksWriter},
        embeddings::{EmbeddingsService, EmbeddingsServiceOptions, EmbeddingsWriter},
    },
};
use zk_domain::{
    Capability, ChunkEmbeddingRecord, ChunkRecord, ConversationId, EmbeddingModelId, ProviderId,
    ProviderSelectorPolicy,
    config::{EffectiveConfig, GlobalConfig},
    id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision},
    note::{NoteFrontmatter, NoteRecord},
    write::{BatchWriteRequest, UpdateNoteRequest},
};
use zk_vault::{NormalizationContext, analyze_body, normalize_frontmatter, parse_note_file};

use crate::{
    ServerDb, ServerError,
    ai::{
        chat_runtime::{ChatRuntimeError, build_chat_service},
        ingest_wiki::{IngestSourceService, WikiIngestError, WikiIngestPlan, WikiVocabulary},
    },
    config::{AiChatConfig, AiChatProviderKind, AiConfig, AiProviderKind},
    repo::{
        AiJob, AiJobKind, AiJobsRepo, ChunksRepo, ConversationsRepo, EmbeddingsRepo, NoteRow,
        NotesRepo,
    },
    service::{RecordNoteChange, next_server_revision, record_note_change},
};

const IDLE_INTERVAL: Duration = Duration::from_secs(1);
const WORKER_EMBEDDING_ATTEMPTS: u32 = 1;
const DEFAULT_STUB_DIM: usize = 384;
const STUB_PROVIDER_ID: &str = "stub";
const LOG_TARGET: &str = "zk_server::ai";

/// Dependencies owned by the in-process AI job worker.
#[derive(Clone)]
pub struct AiWorkerDeps {
    /// Shared server database handle.
    pub db: Arc<ServerDb>,
    /// Runtime selector for embeddings providers.
    pub selector: Arc<ProviderSelector>,
    /// Loaded AI worker configuration.
    pub config: AiConfig,
    /// Filesystem vault root used for note-body reads.
    pub vault_root: PathBuf,
    /// Effective local vault/index configuration used for filesystem writes.
    pub app_config: EffectiveConfig,
    /// Stable device identity used for local sync-state rows written by `zk-app`.
    pub worker_device_id: DeviceId,
}

/// Spawns the in-process AI job worker.
#[must_use]
pub fn spawn_ai_job_worker(deps: AiWorkerDeps, shutdown: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_ai_job_worker(deps, shutdown).await;
    })
}

/// Builds a provider selector from server AI config.
#[must_use]
pub(crate) fn provider_selector_from_config(config: &AiConfig) -> Arc<ProviderSelector> {
    let mut registry = ProviderRegistry::new();

    if let Some(remote) = &config.remote {
        match RemoteEmbeddingsProvider::from_config(remote.clone()) {
            Ok(provider) => registry.register_embeddings(Arc::new(provider)),
            Err(error) => tracing::warn!(
                target: LOG_TARGET,
                error = %error,
                "remote embeddings provider is unavailable"
            ),
        }
    }
    if let Some(local) = &config.local {
        match LocalEmbeddingsProvider::from_config(local.clone()) {
            Ok(provider) => registry.register_embeddings(Arc::new(provider)),
            Err(error) => tracing::warn!(
                target: LOG_TARGET,
                error = %error,
                "local embeddings provider is unavailable"
            ),
        }
    }

    let stub_dim = config
        .stub
        .as_ref()
        .map_or(DEFAULT_STUB_DIM, |stub| stub.dim);
    registry.register_embeddings(Arc::new(StubEmbeddingsProvider::new(stub_dim)));
    if let Some(chat) = &config.chat {
        match chat.provider {
            AiChatProviderKind::Stub => {
                registry.register_chat(Arc::new(StubChatProvider::new(
                    chat.model_id.clone(),
                    chat.max_output_tokens,
                )));
            }
            AiChatProviderKind::RemoteHttp => {
                match RemoteHttpChatProvider::new(chat.to_remote_http_config()) {
                    Ok(provider) => registry.register_chat(Arc::new(provider)),
                    Err(error) => tracing::warn!(
                        target: LOG_TARGET,
                        error = %error,
                        "remote chat provider is unavailable"
                    ),
                }
            }
        }
    }

    Arc::new(ProviderSelector::new(
        Arc::new(registry),
        selector_policy_from_config(config),
    ))
}

/// Builds the local write/index config used by AI worker filesystem mutations.
#[must_use]
pub(crate) fn app_config_from_vault_root(vault_root: &Path) -> EffectiveConfig {
    let vault_config_path = zk_app::config::resolve_vault_config_path(vault_root);
    let cli = zk_domain::config::CliOverrides {
        vault_path: Some(vault_root.to_path_buf()),
        ..zk_domain::config::CliOverrides::default()
    };

    match zk_app::config::load_vault_config(&vault_config_path) {
        Ok(vault_config) => {
            zk_app::config::merge_effective_config(&GlobalConfig::default(), &vault_config, &cli)
        }
        Err(error) => {
            tracing::warn!(
                target: LOG_TARGET,
                error = %error,
                "failed to load vault config for AI worker; wiki jobs will fail until config is fixed"
            );
            EffectiveConfig {
                vault_path: vault_root.to_path_buf(),
                ..EffectiveConfig::default()
            }
        }
    }
}

fn selector_policy_from_config(config: &AiConfig) -> ProviderSelectorPolicy {
    match config.provider {
        AiProviderKind::Stub => {
            let stub_provider = provider_id(STUB_PROVIDER_ID);

            ProviderSelectorPolicy {
                mode: config.default_mode,
                remote_provider: Some(stub_provider.clone()),
                local_provider: None,
                stub_provider: Some(stub_provider),
            }
        }
        AiProviderKind::RemoteHttp | AiProviderKind::LocalCandle => ProviderSelectorPolicy {
            mode: config.default_mode,
            remote_provider: None,
            local_provider: None,
            stub_provider: None,
        },
    }
}

fn provider_id(value: &str) -> ProviderId {
    value
        .parse()
        .expect("hardcoded provider id should be valid")
}

async fn run_ai_job_worker(deps: AiWorkerDeps, shutdown: CancellationToken) {
    recover_in_flight_jobs(&deps);

    loop {
        if shutdown.is_cancelled() {
            return;
        }

        let claimed_count = drain_available_jobs(&deps, &shutdown).await;
        if claimed_count > 0 {
            continue;
        }

        tokio::select! {
            () = tokio::time::sleep(IDLE_INTERVAL) => {}
            () = shutdown.cancelled() => return,
        }
    }
}

async fn drain_available_jobs(deps: &AiWorkerDeps, shutdown: &CancellationToken) -> u32 {
    let mut claimed_count = 0_u32;

    for _ in 0..deps.config.recompute_concurrency {
        if shutdown.is_cancelled() {
            return claimed_count;
        }

        let Some(job) = claim_next_job(deps) else {
            return claimed_count;
        };
        claimed_count += 1;
        if !run_claimed_job(deps, shutdown, job).await {
            return claimed_count;
        }
    }

    claimed_count
}

fn recover_in_flight_jobs(deps: &AiWorkerDeps) {
    match deps
        .db
        .with_conn(|connection| AiJobsRepo::new(connection).recover_in_flight())
    {
        Ok(recovered) if recovered > 0 => tracing::info!(
            target: LOG_TARGET,
            recovered,
            "recovered orphaned AI jobs"
        ),
        Ok(_) => {}
        Err(error) => tracing::error!(
            target: LOG_TARGET,
            error = %error,
            "failed to recover orphaned AI jobs"
        ),
    }
}

fn claim_next_job(deps: &AiWorkerDeps) -> Option<AiJob> {
    match deps.db.with_conn(|connection| {
        // claim_next is kind-agnostic, so claim broadly and enforce per-kind retry caps on failure.
        AiJobsRepo::new(connection).claim_next(claim_max_attempts(deps), Utc::now())
    }) {
        Ok(job) => job,
        Err(error) => {
            tracing::error!(target: LOG_TARGET, error = %error, "failed to claim AI job");
            None
        }
    }
}

async fn run_claimed_job(deps: &AiWorkerDeps, shutdown: &CancellationToken, job: AiJob) -> bool {
    let span = tracing::span!(
        Level::INFO,
        "ai_worker_job",
        job_id = %job.job_id,
        kind = job_kind_label(&job.kind)
    );
    let result = tokio::select! {
        result = process_job(deps, &job).instrument(span) => result,
        () = shutdown.cancelled() => return false,
    };

    match result {
        Ok(()) => mark_job_done(deps, &job),
        Err(error) => mark_job_failed(deps, &job, &error),
    }

    true
}

async fn process_job(deps: &AiWorkerDeps, job: &AiJob) -> Result<(), AiWorkerError> {
    match &job.kind {
        AiJobKind::RecomputeNote { note_id } => recompute_note(deps, note_id).await,
        AiJobKind::RecomputeVault => enqueue_vault_recompute_jobs(deps),
        AiJobKind::ChatTurn {
            conversation_id,
            request_id,
            user_query,
        } => process_chat_turn(deps, conversation_id, request_id, user_query).await,
        AiJobKind::IngestSource {
            source_note_id,
            request_id,
        } => {
            let span = tracing::span!(
                Level::INFO,
                "ingest_source_job",
                job_id = %job.job_id,
                source_note_id = %source_note_id,
                request_id = %request_id
            );
            process_ingest_source(deps, source_note_id, request_id)
                .instrument(span)
                .await
        }
        AiJobKind::LintWiki { .. } => Err(AiWorkerError::NotYetImplemented("lint_wiki")),
    }
}

async fn recompute_note(deps: &AiWorkerDeps, note_id: &NoteId) -> Result<(), AiWorkerError> {
    let body = read_note_body(deps, note_id)?;
    let chunks = ServerChunksWriter {
        db: Arc::clone(&deps.db),
    };
    let chunking_service = ChunkingService::new(ParagraphChunker::default(), chunks);
    chunking_service.chunk_note(note_id, &body)?;

    let provider = deps.selector.select_embeddings(Capability::Embeddings)?;
    let embeddings = ServerEmbeddingsWriter {
        db: Arc::clone(&deps.db),
    };
    let embeddings_service = EmbeddingsService::new(
        provider,
        embeddings,
        EmbeddingsServiceOptions {
            max_attempts: WORKER_EMBEDDING_ATTEMPTS,
            initial_backoff: Duration::ZERO,
            backoff_max: Duration::ZERO,
        },
    );
    embeddings_service.recompute_note(note_id).await?;

    Ok(())
}

fn enqueue_vault_recompute_jobs(deps: &AiWorkerDeps) -> Result<(), AiWorkerError> {
    let note_ids = deps.db.with_conn(|connection| {
        NotesRepo::list_all(connection).map(|notes| {
            notes
                .into_iter()
                .filter(|note| !note.is_deleted && !note.is_trashed)
                .map(|note| NoteId::new(&note.note_id))
                .collect::<Vec<_>>()
        })
    })?;

    deps.db.with_conn(|connection| {
        let repo = AiJobsRepo::new(connection);
        for note_id in note_ids {
            repo.enqueue(AiJobKind::RecomputeNote { note_id }, Utc::now())?;
        }

        Ok(())
    })?;

    Ok(())
}

/// Runs one fire-and-forget chat job.
///
/// V1 consumes the stream to let `ChatService` persist the assistant message,
/// then discards emitted chunks because there is no `chat_job_outputs` table.
async fn process_chat_turn(
    deps: &AiWorkerDeps,
    conversation_id: &ConversationId,
    request_id: &str,
    user_query: &str,
) -> Result<(), AiWorkerError> {
    let chat_config = deps.config.chat.clone().unwrap_or_default();
    let conversation = deps
        .db
        .with_conn(|connection| ConversationsRepo::get(connection, conversation_id))?
        .ok_or_else(|| AiWorkerError::ConversationNotFound(conversation_id.clone()))?;
    let service = build_chat_service(Arc::clone(&deps.db), deps.selector.clone(), &chat_config)?;
    let mut stream = service
        .turn(ChatTurnRequest {
            conversation_id: conversation_id.clone(),
            user_id: conversation.user_id,
            user_query: user_query.to_string(),
            top_k: chat_config.retrieval_top_k,
            request_id: request_id.to_string(),
        })
        .await?;

    while let Some(chunk) = stream.next().await {
        chunk?;
    }

    Ok(())
}

async fn process_ingest_source(
    deps: &AiWorkerDeps,
    source_note_id: &NoteId,
    request_id: &str,
) -> Result<(), AiWorkerError> {
    let wiki = deps
        .app_config
        .wiki
        .as_ref()
        .ok_or(AiWorkerError::WikiConfigMissing)?;
    let source_row = source_row(deps, source_note_id)?;
    let source_title = source_row.title.clone();
    let source = note_record_from_row(&source_row);
    let source_body = read_note_body(deps, source_note_id)?;
    let existing_wiki_titles = existing_wiki_titles(deps, &wiki.root_dir)?;
    let vocabulary = WikiVocabulary::new(wiki.root_dir.clone());
    let service = IngestSourceService::new(
        Arc::clone(&deps.selector),
        deps.config.default_mode,
        request_id.to_string(),
    );
    let plan = service
        .ingest_source(&source, &source_body, &existing_wiki_titles, &vocabulary)
        .await?;
    let plan = plan_with_sources(deps, source_note_id, plan)?;

    let mut local_connection = zk_app::bootstrap_index_connection(&deps.app_config)?;
    let mut write_context = zk_app::WriteContext {
        conn: &mut local_connection,
        device_id: &deps.worker_device_id,
    };
    let write_outcome = zk_app::batch_write(
        &deps.vault_root,
        &deps.app_config,
        &mut write_context,
        BatchWriteRequest {
            creates: plan.creates,
            updates: plan.updates,
        },
    )?;
    if !write_outcome.failures.is_empty() {
        return Err(AiWorkerError::BatchWriteFailed {
            failures: write_outcome.failures,
        });
    }

    let changed_note_ids = changed_wiki_note_ids(&write_outcome);
    let changed_page_count = changed_note_ids.len();
    append_wiki_log(
        deps,
        &mut local_connection,
        &source_title,
        changed_page_count,
    )?;
    regenerate_wiki_index(deps, &mut local_connection)?;
    mirror_changed_notes_to_server(deps, &changed_note_ids)?;
    enqueue_recompute_for_changed_pages(deps, &changed_note_ids)?;

    Ok(())
}

fn read_note_body(deps: &AiWorkerDeps, note_id: &NoteId) -> Result<String, AiWorkerError> {
    let note_path = deps
        .db
        .with_conn(|connection| NotesRepo::get(connection, note_id.as_str()))?
        .ok_or_else(|| AiWorkerError::NoteNotFound(note_id.clone()))?
        .current_path;
    let relative_path = RelativePath::new(&note_path);
    let absolute_path = deps.vault_root.join(relative_path.as_str());
    let document = parse_note_file(&absolute_path)?;

    Ok(document.body_markdown)
}

fn source_row(deps: &AiWorkerDeps, note_id: &NoteId) -> Result<NoteRow, AiWorkerError> {
    deps.db
        .with_conn(|connection| NotesRepo::get(connection, note_id.as_str()))?
        .ok_or_else(|| AiWorkerError::NoteNotFound(note_id.clone()))
}

fn existing_wiki_titles(
    deps: &AiWorkerDeps,
    wiki_root: &RelativePath,
) -> Result<Vec<String>, AiWorkerError> {
    let mut titles = deps.db.with_conn(NotesRepo::list_all).map(|notes| {
        notes
            .into_iter()
            .filter(|note| !note.is_deleted && !note.is_trashed)
            .filter(|note| path_has_prefix(&note.current_path, wiki_root))
            .map(|note| note.title)
            .collect::<Vec<_>>()
    })?;
    titles.sort();

    Ok(titles)
}

fn plan_with_sources(
    deps: &AiWorkerDeps,
    source_note_id: &NoteId,
    mut plan: WikiIngestPlan,
) -> Result<WikiIngestPlan, AiWorkerError> {
    for create in &mut plan.creates {
        add_source_note_id(&mut create.frontmatter.sources, source_note_id);
    }

    for update in &mut plan.updates {
        let existing_frontmatter = frontmatter_for_note(deps, &update.note_id)?;
        let mut frontmatter = match update.frontmatter.take() {
            Some(mut frontmatter) => {
                let mut sources = existing_frontmatter.sources;
                for source_note_id in frontmatter.sources.drain(..) {
                    add_source_note_id(&mut sources, &source_note_id);
                }
                frontmatter.sources = sources;
                frontmatter
            }
            None => existing_frontmatter,
        };
        add_source_note_id(&mut frontmatter.sources, source_note_id);
        update.frontmatter = Some(frontmatter);
    }

    Ok(plan)
}

fn add_source_note_id(sources: &mut Vec<NoteId>, source_note_id: &NoteId) {
    if !sources.iter().any(|candidate| candidate == source_note_id) {
        sources.push(source_note_id.clone());
    }
}

fn frontmatter_for_note(
    deps: &AiWorkerDeps,
    note_id: &NoteId,
) -> Result<NoteFrontmatter, AiWorkerError> {
    let row = deps
        .db
        .with_conn(|connection| NotesRepo::get(connection, note_id.as_str()))?
        .ok_or_else(|| AiWorkerError::NoteNotFound(note_id.clone()))?;
    let relative_path = RelativePath::new(&row.current_path);

    Ok(normalized_note_file(deps, &relative_path)?.frontmatter)
}

fn changed_wiki_note_ids(outcome: &zk_app::BatchWriteOutcome) -> Vec<NoteId> {
    let mut note_ids = outcome
        .created
        .iter()
        .chain(outcome.updated.iter())
        .cloned()
        .collect::<Vec<_>>();
    note_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    note_ids.dedup();
    note_ids
}

fn append_wiki_log(
    deps: &AiWorkerDeps,
    local_connection: &mut Connection,
    source_title: &str,
    changed_page_count: usize,
) -> Result<(), AiWorkerError> {
    let log_path = deps
        .app_config
        .wiki_log_path()
        .ok_or(AiWorkerError::WikiConfigMissing)?;
    let parsed = normalized_note_file(deps, &log_path)?;
    let mut body = parsed.body_markdown;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    let _ = writeln!(
        body,
        "- {} ingest {} -> {} pages",
        Utc::now().to_rfc3339(),
        source_title,
        changed_page_count
    );

    update_note_by_frontmatter(deps, local_connection, parsed.frontmatter, body)
}

fn regenerate_wiki_index(
    deps: &AiWorkerDeps,
    local_connection: &mut Connection,
) -> Result<(), AiWorkerError> {
    let index_path = deps
        .app_config
        .wiki_index_path()
        .ok_or(AiWorkerError::WikiConfigMissing)?;
    let index_note = normalized_note_file(deps, &index_path)?;
    let pages = authored_wiki_pages(deps)?;
    let body = render_wiki_index(&pages);

    update_note_by_frontmatter(deps, local_connection, index_note.frontmatter, body)
}

fn update_note_by_frontmatter(
    deps: &AiWorkerDeps,
    local_connection: &mut Connection,
    frontmatter: NoteFrontmatter,
    body_markdown: String,
) -> Result<(), AiWorkerError> {
    let note_id = frontmatter.id.clone();
    let mut write_context = zk_app::WriteContext {
        conn: local_connection,
        device_id: &deps.worker_device_id,
    };
    zk_app::update_note(
        &deps.vault_root,
        &deps.app_config,
        &mut write_context,
        UpdateNoteRequest {
            note_id,
            frontmatter: Some(frontmatter),
            body_markdown: Some(body_markdown),
        },
        None,
    )?;

    Ok(())
}

fn authored_wiki_pages(deps: &AiWorkerDeps) -> Result<Vec<WikiIndexPage>, AiWorkerError> {
    let wiki = deps
        .app_config
        .wiki
        .as_ref()
        .ok_or(AiWorkerError::WikiConfigMissing)?;
    let scan = zk_vault::scan_vault(&deps.vault_root, &deps.app_config)?;
    let mut pages = Vec::new();

    for note in scan.notes {
        if !path_has_prefix(note.relative_path.as_str(), &wiki.root_dir) {
            continue;
        }
        let Some(frontmatter) = note.frontmatter else {
            continue;
        };
        if !is_authored_wiki_type(frontmatter.note_type.as_deref()) {
            continue;
        }
        pages.push(WikiIndexPage {
            title: frontmatter.title,
            sources: frontmatter.sources,
        });
    }

    pages.sort_by(|left, right| left.title.cmp(&right.title));

    Ok(pages)
}

fn render_wiki_index(pages: &[WikiIndexPage]) -> String {
    let mut body = "# Wiki Index\n\n".to_string();

    if pages.is_empty() {
        body.push_str("- (none)\n");
        return body;
    }

    for page in pages {
        let sources = if page.sources.is_empty() {
            "none".to_string()
        } else {
            page.sources
                .iter()
                .map(NoteId::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(body, "- [[{}]] <- {}", page.title, sources);
    }

    body
}

fn mirror_changed_notes_to_server(
    deps: &AiWorkerDeps,
    changed_note_ids: &[NoteId],
) -> Result<(), AiWorkerError> {
    let mut mirrored_ids = changed_note_ids.to_vec();
    if let Some(index_path) = deps.app_config.wiki_index_path() {
        mirrored_ids.push(normalized_note_file(deps, &index_path)?.frontmatter.id);
    }
    if let Some(log_path) = deps.app_config.wiki_log_path() {
        mirrored_ids.push(normalized_note_file(deps, &log_path)?.frontmatter.id);
    }
    mirrored_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    mirrored_ids.dedup();

    for note_id in mirrored_ids {
        let relative_path = path_for_note_id(deps, &note_id)?;
        let existing = deps
            .db
            .with_conn(|connection| NotesRepo::get(connection, note_id.as_str()))?;
        let event_type = if existing.is_some() {
            "update"
        } else {
            "create"
        };
        let current_revision = existing.as_ref().map_or(0, |row| row.server_revision);
        let row = note_row_from_file(
            deps,
            &relative_path,
            existing.as_ref(),
            next_server_revision(current_revision),
        )?;
        record_note_change(
            &deps.db,
            RecordNoteChange {
                device_id: Some(deps.worker_device_id.as_str()),
                event_type,
                note: &row,
            },
            Utc::now(),
        )?;
    }

    Ok(())
}

fn enqueue_recompute_for_changed_pages(
    deps: &AiWorkerDeps,
    changed_note_ids: &[NoteId],
) -> Result<(), AiWorkerError> {
    deps.db.with_conn(|connection| {
        let repo = AiJobsRepo::new(connection);
        for note_id in changed_note_ids {
            if !repo.pending_recompute_note_exists(note_id)? {
                repo.enqueue(
                    AiJobKind::RecomputeNote {
                        note_id: note_id.clone(),
                    },
                    Utc::now(),
                )?;
            }
        }

        Ok(())
    })?;

    Ok(())
}

fn path_for_note_id(deps: &AiWorkerDeps, note_id: &NoteId) -> Result<RelativePath, AiWorkerError> {
    let index = zk_vault::build_note_path_index(&deps.vault_root, &deps.app_config)?.index;
    index
        .get(note_id)
        .cloned()
        .ok_or_else(|| AiWorkerError::NoteNotFound(note_id.clone()))
}

fn note_row_from_file(
    deps: &AiWorkerDeps,
    relative_path: &RelativePath,
    existing: Option<&NoteRow>,
    server_revision: i64,
) -> Result<NoteRow, AiWorkerError> {
    let absolute_path = deps.vault_root.join(relative_path.as_str());
    let bytes = fs::read(&absolute_path).map_err(|source| AiWorkerError::Io {
        path: absolute_path.clone(),
        source,
    })?;
    let parsed = normalized_note_file(deps, relative_path)?;
    let now = Utc::now();

    Ok(NoteRow {
        note_id: parsed.frontmatter.id.to_string(),
        current_path: relative_path.to_string(),
        title: parsed.frontmatter.title.clone(),
        content_hash: ContentHash::from_bytes(&bytes).to_string(),
        body_markdown: parsed.body_markdown,
        frontmatter_json: serde_json::to_string(&parsed.frontmatter)?,
        created_at: existing.map_or(now, |row| row.created_at),
        updated_at: now,
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: Some(deps.worker_device_id.to_string()),
        server_revision,
    })
}

fn normalized_note_file(
    deps: &AiWorkerDeps,
    relative_path: &RelativePath,
) -> Result<NormalizedNoteFile, AiWorkerError> {
    let parsed = parse_note_file(&deps.vault_root.join(relative_path.as_str()))?;
    let raw_frontmatter = parsed
        .frontmatter
        .ok_or_else(|| AiWorkerError::MissingFrontmatter(relative_path.clone()))?;
    let body_analysis = analyze_body(&parsed.body_markdown);
    let context = NormalizationContext {
        relative_path,
        body_analysis: &body_analysis,
        config: &deps.app_config,
        is_immutable: false,
    };
    let (frontmatter, _) = normalize_frontmatter(raw_frontmatter, &context);

    Ok(NormalizedNoteFile {
        frontmatter,
        body_markdown: parsed.body_markdown,
    })
}

fn note_record_from_row(row: &NoteRow) -> NoteRecord {
    NoteRecord {
        note_id: NoteId::new(&row.note_id),
        current_path: RelativePath::new(&row.current_path),
        content_hash: ContentHash::new(&row.content_hash),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
        trashed_at: row.trashed_at.map(|timestamp| timestamp.to_rfc3339()),
        deleted_at: row.deleted_at.map(|timestamp| timestamp.to_rfc3339()),
        is_trashed: row.is_trashed,
        is_deleted: row.is_deleted,
        last_editor_device_id: row.last_editor_device_id.as_deref().map(DeviceId::new),
        server_revision: ServerRevision::new(row.server_revision),
    }
}

fn path_has_prefix(path: &str, prefix: &RelativePath) -> bool {
    path == prefix.as_str()
        || path
            .strip_prefix(prefix.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
}

fn is_authored_wiki_type(note_type: Option<&str>) -> bool {
    matches!(note_type, Some("wiki-entity" | "wiki-synthesis"))
}

struct NormalizedNoteFile {
    frontmatter: NoteFrontmatter,
    body_markdown: String,
}

struct WikiIndexPage {
    title: String,
    sources: Vec<NoteId>,
}

fn mark_job_done(deps: &AiWorkerDeps, job: &AiJob) {
    if let Err(error) = deps
        .db
        .with_conn(|connection| AiJobsRepo::new(connection).mark_done(&job.job_id, Utc::now()))
    {
        tracing::error!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %error,
            "failed to mark AI job done"
        );
    }
}

fn mark_job_failed(deps: &AiWorkerDeps, job: &AiJob, error: &AiWorkerError) {
    if matches!(error, AiWorkerError::Ai(AiError::ProviderUnavailable(_))) {
        tracing::warn!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %error,
            "AI provider unavailable"
        );
    } else {
        tracing::error!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %error,
            "AI job failed"
        );
    }

    if let Err(mark_error) = deps.db.with_conn(|connection| {
        AiJobsRepo::new(connection).mark_failed(
            &job.job_id,
            &error.to_string(),
            max_attempts_for_job(deps, &job.kind),
            Utc::now(),
        )
    }) {
        tracing::error!(
            target: LOG_TARGET,
            job_id = %job.job_id,
            error = %mark_error,
            "failed to mark AI job failed"
        );
    }
}

fn claim_max_attempts(deps: &AiWorkerDeps) -> u32 {
    deps.config.max_attempts.max(chat_max_attempts(deps))
}

fn max_attempts_for_job(deps: &AiWorkerDeps, kind: &AiJobKind) -> u32 {
    match kind {
        AiJobKind::ChatTurn { .. } => chat_max_attempts(deps),
        AiJobKind::RecomputeNote { .. }
        | AiJobKind::RecomputeVault
        | AiJobKind::IngestSource { .. }
        | AiJobKind::LintWiki { .. } => deps.config.max_attempts,
    }
}

fn chat_max_attempts(deps: &AiWorkerDeps) -> u32 {
    deps.config.chat.as_ref().map_or_else(
        || u32::from(AiChatConfig::default().max_attempts),
        |chat| u32::from(chat.max_attempts),
    )
}

fn job_kind_label(kind: &AiJobKind) -> &'static str {
    match kind {
        AiJobKind::RecomputeNote { .. } => "recompute_note",
        AiJobKind::RecomputeVault => "recompute_vault",
        AiJobKind::ChatTurn { .. } => "chat_turn",
        AiJobKind::IngestSource { .. } => "ingest_source",
        AiJobKind::LintWiki { .. } => "lint_wiki",
    }
}

#[derive(Clone)]
struct ServerChunksWriter {
    db: Arc<ServerDb>,
}

impl ChunksWriter for ServerChunksWriter {
    fn list_existing_hashes(&self, note_id: &NoteId) -> AiResult<Vec<String>> {
        self.with_conn(|connection| {
            ChunksWriter::list_existing_hashes(&ChunksRepo::new(connection), note_id)
        })
    }

    fn replace_for_note(&self, note_id: &NoteId, chunks: &[ChunkRecord]) -> AiResult<u64> {
        self.db
            .with_conn(|connection| ChunksRepo::new(connection).replace_for_note(note_id, chunks))
            .map_err(|error| server_to_ai_error("replace server chunks for note", &error))
    }
}

impl ServerChunksWriter {
    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
        with_conn_ai(&self.db, f)
    }
}

#[derive(Clone)]
struct ServerEmbeddingsWriter {
    db: Arc<ServerDb>,
}

impl EmbeddingsWriter for ServerEmbeddingsWriter {
    fn list_chunks_missing_embeddings(
        &self,
        note_id: &NoteId,
        model: &EmbeddingModelId,
    ) -> AiResult<Vec<ChunkRecord>> {
        self.with_conn(|connection| {
            EmbeddingsWriter::list_chunks_missing_embeddings(
                &EmbeddingsRepo::new(connection),
                note_id,
                model,
            )
        })
    }

    fn upsert_embeddings(&self, records: &[ChunkEmbeddingRecord]) -> AiResult<u64> {
        self.db
            .with_conn(|connection| {
                let repo = EmbeddingsRepo::new(connection);
                for record in records {
                    repo.upsert(record)?;
                }

                Ok(u64::try_from(records.len()).expect("record count should fit u64"))
            })
            .map_err(|error| server_to_ai_error("upsert server chunk embeddings", &error))
    }
}

impl ServerEmbeddingsWriter {
    fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
        with_conn_ai(&self.db, f)
    }
}

fn with_conn_ai<R>(db: &ServerDb, f: impl FnOnce(&Connection) -> AiResult<R>) -> AiResult<R> {
    db.with_conn(|connection| f(connection).map_err(|error| ai_to_server_error(&error)))
        .map_err(|error| server_to_ai_error("borrow server database", &error))
}

fn ai_to_server_error(error: &AiError) -> ServerError {
    match error {
        AiError::InvalidEmbeddingBlob { len } => ServerError::InvalidEmbeddingBlob { len: *len },
        other => ServerError::Internal(Box::new(std::io::Error::other(other.to_string()))),
    }
}

fn server_to_ai_error(context: &str, error: &ServerError) -> AiError {
    match error {
        ServerError::InvalidEmbeddingBlob { len } => AiError::InvalidEmbeddingBlob { len: *len },
        other => AiError::Internal(format!("{context}: {other}")),
    }
}

#[derive(Debug, thiserror::Error)]
enum AiWorkerError {
    #[error(transparent)]
    Server(#[from] ServerError),
    #[error(transparent)]
    Ai(#[from] AiError),
    #[error(transparent)]
    App(#[from] zk_app::AppError),
    #[error(transparent)]
    WikiIngest(#[from] WikiIngestError),
    #[error(transparent)]
    Vault(#[from] zk_vault::error::VaultError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("AI worker wiki config missing")]
    WikiConfigMissing,
    #[error("note not found: {0}")]
    NoteNotFound(NoteId),
    #[error("note is missing frontmatter: {0}")]
    MissingFrontmatter(RelativePath),
    #[error("conversation not found: {0}")]
    ConversationNotFound(ConversationId),
    #[error("{0} not yet implemented")]
    NotYetImplemented(&'static str),
    #[error("batch write failed: {failures:?}")]
    BatchWriteFailed {
        failures: Vec<zk_app::BatchWriteFailure>,
    },
    #[error("read AI worker note file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("read AI chat system prompt {path}: {source}")]
    ReadSystemPrompt {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<ChatRuntimeError> for AiWorkerError {
    fn from(error: ChatRuntimeError) -> Self {
        match error {
            ChatRuntimeError::Ai(error) => Self::Ai(error),
            ChatRuntimeError::ReadSystemPrompt { path, source } => {
                Self::ReadSystemPrompt { path, source }
            }
        }
    }
}
