//! Write-oriented facade helpers.
//!
//! Batch writes are non-transactional and reuse a single per-call note-path
//! index for updates. Successful creates publish their note IDs back into that
//! in-memory index so later updates in the same batch can target newly created
//! notes without triggering another vault-wide scan.

use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
};

use rusqlite::Connection;
use serde::Serialize;
use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, DeviceId, NoteId, RelativePath},
    note::NoteFrontmatter,
    sync::{PendingOperation, SyncStatus},
    sync_state::NoteSyncState,
    write::{BatchWriteRequest, CreateNoteRequest, UpdateNoteRequest},
};
use zk_index::{IndexError, NoteRepo, SyncStateRepo, UpsertNoteParams};

use crate::{
    AppError, Result, config::validate_effective_config, ensure_writable, is_path_immutable,
};

/// Batch writes are non-transactional at the filesystem level.
///
/// The API applies operations sequentially and does not roll back already-written files.
/// Callers are responsible for idempotency and for running a post-batch reindex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum BatchWriteKind {
    /// A `create_note` operation.
    Create,
    /// An `update_note` operation.
    Update,
}

/// Captures one failed entry from [`batch_write`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchWriteFailure {
    /// Zero-based position of the failed request within its input collection.
    pub request_index: usize,
    /// Operation type that failed.
    pub kind: BatchWriteKind,
    /// Rendered error message for the failed operation.
    pub error_message: String,
    /// Candidate note ID associated with the failed request when available.
    pub note_id: Option<NoteId>,
}

/// Aggregated success/failure outcome for [`batch_write`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchWriteOutcome {
    /// IDs successfully created in request order.
    pub created: Vec<NoteId>,
    /// IDs successfully updated in request order.
    pub updated: Vec<NoteId>,
    /// Failed create/update entries collected without rollback.
    pub failures: Vec<BatchWriteFailure>,
    /// Number of note-path index builds performed during this batch in debug builds.
    #[cfg(debug_assertions)]
    pub note_path_index_builds: usize,
}

/// Shared local index inputs required to persist note writes.
pub struct WriteContext<'a> {
    /// Mutable `SQLite` connection used for note-row and sync-state persistence.
    pub conn: &'a mut Connection,
    /// Local device identity that owns the refreshed `last_local_hash`.
    pub device_id: &'a DeviceId,
}

/// Creates a new note file under `vault_root` using atomic file replacement semantics.
///
/// # Errors
///
/// Returns [`AppError`] when the target folder is immutable, path existence checks
/// fail, parent directory creation fails, note rendering/write operations fail,
/// or local index persistence fails.
pub fn create_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    write_ctx: &mut WriteContext<'_>,
    request: CreateNoteRequest,
) -> Result<NoteId> {
    ensure_writable(&request.path, &config.folders)?;

    let absolute_path = vault_root.join(request.path.as_str());
    match fs::symlink_metadata(&absolute_path) {
        Ok(_) => {
            return Err(AppError::NotePathExists {
                path: request.path.clone(),
            });
        }
        Err(source) if source.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AppError::Io {
                path: absolute_path.clone(),
                source,
            });
        }
    }

    let parent_dir = absolute_path.parent().ok_or_else(|| AppError::Io {
        path: absolute_path.clone(),
        source: io::Error::new(
            ErrorKind::InvalidInput,
            "while attempting to resolve parent directory for note creation",
        ),
    })?;
    fs::create_dir_all(parent_dir).map_err(|source| AppError::Io {
        path: parent_dir.to_path_buf(),
        source,
    })?;

    let mut frontmatter = request.frontmatter;
    if frontmatter.id.as_str().is_empty() {
        frontmatter.id = NoteId::generate();
    }
    let final_note_id = frontmatter.id.clone();

    let rendered = zk_vault::render_note_file(&frontmatter, &request.body_markdown)
        .map_err(|source| AppError::NoteRender { source })?;
    zk_vault::atomic_write_file(&absolute_path, rendered.as_bytes())?;
    persist_written_note(write_ctx, config, &request.path, &frontmatter, &rendered)?;

    Ok(final_note_id)
}

/// Updates an existing note by stable note ID.
///
/// # Errors
///
/// Returns [`AppError`] when note lookup fails, IDs mismatch, folder policy blocks
/// writes, parse/write operations fail, or local index persistence fails.
pub fn update_note(
    vault_root: &Path,
    config: &EffectiveConfig,
    write_ctx: &mut WriteContext<'_>,
    request: UpdateNoteRequest,
    index: Option<&zk_vault::NotePathIndex>,
) -> Result<()> {
    if let Some(frontmatter) = &request.frontmatter {
        if frontmatter.id != request.note_id {
            return Err(AppError::NoteIdMismatch {
                request_note_id: request.note_id.clone(),
                frontmatter_note_id: frontmatter.id.clone(),
            });
        }
    }

    let (relative_path, current_frontmatter, current_body) =
        resolve_note_for_update(vault_root, config, &request.note_id, index)?;
    let final_frontmatter = request.frontmatter.unwrap_or(current_frontmatter);
    let final_body = request.body_markdown.unwrap_or(current_body);

    ensure_writable(&relative_path, &config.folders)?;

    let rendered = zk_vault::render_note_file(&final_frontmatter, &final_body)
        .map_err(|source| AppError::NoteRender { source })?;
    zk_vault::atomic_write_file(
        &vault_root.join(relative_path.as_str()),
        rendered.as_bytes(),
    )?;
    persist_written_note(
        write_ctx,
        config,
        &relative_path,
        &final_frontmatter,
        &rendered,
    )?;

    Ok(())
}

/// Applies create and update operations sequentially.
///
/// # Errors
///
/// Returns [`AppError`] only for catastrophic preflight failures (invalid config or
/// unreadable/non-directory vault root). Per-entry failures are captured in
/// [`BatchWriteOutcome::failures`].
pub fn batch_write(
    vault_root: &Path,
    config: &EffectiveConfig,
    write_ctx: &mut WriteContext<'_>,
    request: BatchWriteRequest,
) -> Result<BatchWriteOutcome> {
    validate_effective_config(config)?;
    validate_vault_root(vault_root)?;
    let mut note_path_index = zk_vault::build_note_path_index(vault_root, config)?.index;

    let mut outcome = BatchWriteOutcome {
        created: Vec::new(),
        updated: Vec::new(),
        failures: Vec::new(),
        #[cfg(debug_assertions)]
        note_path_index_builds: 1,
    };

    for (request_index, create_request) in request.creates.into_iter().enumerate() {
        let create_path = create_request.path.clone();
        let candidate_note_id = if create_request.frontmatter.id.as_str().is_empty() {
            None
        } else {
            Some(create_request.frontmatter.id.clone())
        };

        match create_note(vault_root, config, write_ctx, create_request) {
            Ok(note_id) => {
                note_path_index.insert(note_id.clone(), create_path);
                outcome.created.push(note_id);
            }
            Err(error) => outcome.failures.push(BatchWriteFailure {
                request_index,
                kind: BatchWriteKind::Create,
                error_message: error.to_string(),
                note_id: candidate_note_id,
            }),
        }
    }

    for (request_index, update_request) in request.updates.into_iter().enumerate() {
        let note_id = update_request.note_id.clone();
        match update_note(
            vault_root,
            config,
            write_ctx,
            update_request,
            Some(&note_path_index),
        ) {
            Ok(()) => outcome.updated.push(note_id),
            Err(error) => outcome.failures.push(BatchWriteFailure {
                request_index,
                kind: BatchWriteKind::Update,
                error_message: error.to_string(),
                note_id: Some(note_id),
            }),
        }
    }

    Ok(outcome)
}

fn persist_written_note(
    write_ctx: &mut WriteContext<'_>,
    config: &EffectiveConfig,
    relative_path: &RelativePath,
    frontmatter: &NoteFrontmatter,
    rendered: &str,
) -> Result<()> {
    let content_hash = ContentHash::from_bytes(rendered.as_bytes());
    let frontmatter_json = serde_json::to_string(frontmatter)
        .map_err(|source| AppError::FrontmatterSerialization { source })?;
    let transaction = write_ctx
        .conn
        .transaction()
        .map_err(|source| app_index_sqlite_error("begin note write transaction", source))?;
    let existing_note = NoteRepo::get_note(&transaction, &frontmatter.id)?;
    let is_trashed = path_is_under_trash(relative_path, config);
    let trashed_at = if is_trashed {
        existing_note
            .as_ref()
            .and_then(|note| note.trashed_at.as_deref())
    } else {
        None
    };

    NoteRepo::upsert_note(
        &transaction,
        UpsertNoteParams {
            note_id: &frontmatter.id,
            path: relative_path,
            title: &frontmatter.title,
            content_hash: &content_hash,
            frontmatter_json: &frontmatter_json,
            created_at: frontmatter.created.as_deref().unwrap_or_default(),
            updated_at: frontmatter.updated.as_deref().unwrap_or_default(),
            trashed_at,
            is_trashed,
        },
    )?;
    upsert_note_local_hash(
        &transaction,
        write_ctx.device_id,
        &frontmatter.id,
        &content_hash,
    )?;
    transaction
        .commit()
        .map_err(|source| app_index_sqlite_error("commit note write transaction", source))?;

    Ok(())
}

fn upsert_note_local_hash(
    conn: &Connection,
    device_id: &DeviceId,
    note_id: &NoteId,
    content_hash: &ContentHash,
) -> Result<()> {
    let next_state = match SyncStateRepo::get_note_sync_state(conn, note_id)? {
        Some(mut state) => {
            state.device_id = device_id.clone();
            state.last_local_hash = Some(content_hash.clone());
            state
        }
        None => NoteSyncState {
            item_id: note_id.clone(),
            device_id: device_id.clone(),
            last_local_hash: Some(content_hash.clone()),
            last_synced_hash: None,
            last_synced_path: None,
            last_known_server_revision: None,
            sync_status: SyncStatus::PendingUpload,
            last_sync_attempt_at: None,
            last_sync_error: None,
            pending_operation: Some(PendingOperation::Create),
        },
    };
    SyncStateRepo::upsert_note_sync_state(conn, &next_state)?;

    Ok(())
}

fn validate_vault_root(vault_root: &Path) -> Result<()> {
    let metadata = fs::metadata(vault_root).map_err(|source| AppError::Io {
        path: vault_root.to_path_buf(),
        source,
    })?;

    if metadata.is_dir() {
        return Ok(());
    }

    Err(AppError::Io {
        path: vault_root.to_path_buf(),
        source: io::Error::new(
            ErrorKind::InvalidInput,
            "while attempting to apply batch write to a non-directory root",
        ),
    })
}

fn resolve_note_for_update(
    vault_root: &Path,
    config: &EffectiveConfig,
    note_id: &NoteId,
    index: Option<&zk_vault::NotePathIndex>,
) -> Result<(RelativePath, NoteFrontmatter, String)> {
    let owned_index;
    let note_path_index = if let Some(index) = index {
        index
    } else {
        owned_index = zk_vault::build_note_path_index(vault_root, config)?.index;
        &owned_index
    };

    let Some(relative_path) = note_path_index.get(note_id).cloned() else {
        return Err(note_not_found(note_id));
    };
    let absolute_path = vault_root.join(relative_path.as_str());
    let parsed = zk_vault::parse_note_file(&absolute_path).map_err(|_| note_not_found(note_id))?;
    let Some(raw_frontmatter) = parsed.frontmatter else {
        return Err(note_not_found(note_id));
    };

    if raw_frontmatter.id.as_ref() != Some(note_id) {
        return Err(note_not_found(note_id));
    }

    let body_analysis = zk_vault::analyze_body(&parsed.body_markdown);
    let normalization_context = zk_vault::NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config,
        is_immutable: is_path_immutable(&relative_path, &config.folders),
    };
    let (current_frontmatter, _) =
        zk_vault::normalize_frontmatter(raw_frontmatter, &normalization_context);

    Ok((relative_path, current_frontmatter, parsed.body_markdown))
}

fn note_not_found(note_id: &NoteId) -> AppError {
    AppError::NoteNotFound {
        note_id: note_id.clone(),
    }
}

fn path_is_under_trash(path: &RelativePath, config: &EffectiveConfig) -> bool {
    let trash_root = config.trash_dir.as_str();
    let candidate = path.as_str();

    candidate == trash_root
        || candidate
            .strip_prefix(trash_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn app_index_sqlite_error(context: &str, source: rusqlite::Error) -> AppError {
    AppError::Index(IndexError::Sqlite {
        source,
        context: context.to_string(),
    })
}
