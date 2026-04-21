use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use serde::Serialize;
use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, NoteId, RelativePath},
    note::{DiagnosticLevel, NoteDocument, NoteFrontmatter, RuleId, ScanDiagnostic},
};

use crate::{
    error::VaultError,
    normalize::{
        NormalizationContext, NormalizationPlan, TitleSource, empty_raw_frontmatter,
        normalize_frontmatter, plan_normalization_with_body_analysis,
    },
    parse::{
        TrashedUnknownIdentityReason, analyze_body, parse_note_file, parse_note_file_lenient,
        parse_note_frontmatter,
    },
    scan::{AssetCandidate, NoteCandidate, TrashState, scan_vault_files},
    serialize::render_note_file,
    write::atomic_write_file,
};

/// Parsed note data and normalization plan produced for one scanned note file.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizationResult {
    /// Vault-relative note path at scan time.
    pub relative_path: RelativePath,
    /// Stable note ID when the note could be identified.
    pub note_id: Option<NoteId>,
    /// Whether the note is live or inside the configured trash tree.
    pub trash_state: TrashState,
    /// Best-effort trash timestamp derived from filesystem metadata.
    pub trashed_at: Option<String>,
    /// Planned normalization changes for this note.
    pub plan: NormalizationPlan,
    /// Normalized frontmatter when parsing succeeded well enough to produce one.
    pub frontmatter: Option<NoteFrontmatter>,
    /// Parsed note document when body and frontmatter were usable.
    pub document: Option<NoteDocument>,
    /// Content hash of the original note bytes on disk.
    pub content_hash: ContentHash,
}

/// Full vault scan output used by validation, indexing, and normalization writes.
#[derive(Debug, Clone, PartialEq)]
pub struct VaultScanResult {
    /// Parsed note scan results in deterministic path order.
    pub notes: Vec<NormalizationResult>,
    /// Non-note files discovered during the vault walk.
    pub assets: Vec<AssetCandidate>,
    /// Scan diagnostics emitted during file discovery and parsing.
    pub diagnostics: Vec<ScanDiagnostic>,
    /// Strict frontmatter parse failures kept separate for validation rules.
    pub frontmatter_parse_failures: Vec<FrontmatterParseFailure>,
    /// Duplicate note IDs grouped with the paths that share each ID.
    pub duplicate_ids: Vec<(NoteId, Vec<RelativePath>)>,
    /// Number of scanned notes currently living under the configured trash tree.
    pub trashed_note_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Structured frontmatter parse failure captured during vault scan.
pub struct FrontmatterParseFailure {
    /// Vault-relative path of the note whose frontmatter failed to parse.
    pub path: RelativePath,
    /// Human-readable YAML parser error message.
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Reusable note-id to path lookup used by higher-level write flows.
pub struct NotePathIndex {
    paths_by_id: HashMap<NoteId, RelativePath>,
    ambiguous_ids: HashSet<NoteId>,
}

impl NotePathIndex {
    /// Creates an empty lookup index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the resolved relative path for a unique note ID.
    #[must_use]
    pub fn get(&self, note_id: &NoteId) -> Option<&RelativePath> {
        self.paths_by_id.get(note_id)
    }

    /// Inserts a note ID to path mapping, marking duplicate IDs as ambiguous.
    pub fn insert(&mut self, note_id: NoteId, relative_path: RelativePath) {
        if self.ambiguous_ids.contains(&note_id) {
            return;
        }

        match self.paths_by_id.get(&note_id) {
            None => {
                self.paths_by_id.insert(note_id, relative_path);
            }
            Some(existing_path) if existing_path == &relative_path => {}
            Some(_) => {
                let removed_note_id = note_id.clone();
                let _ = self.paths_by_id.remove(&removed_note_id);
                let _ = self.ambiguous_ids.insert(note_id);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of building a reusable note-path lookup index.
pub struct NotePathIndexBuild {
    /// Unique note-id to path mappings that can be reused by callers.
    pub index: NotePathIndex,
    /// Non-fatal diagnostics emitted while skipping malformed notes.
    pub diagnostics: Vec<ScanDiagnostic>,
}

/// Summary counts returned after applying normalization writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NormalizationSummary {
    /// Number of files rewritten with normalized content.
    pub files_written: u32,
    /// Number of files skipped because immutable-folder policy blocked writes.
    pub files_skipped_immutable: u32,
    /// Number of files skipped because rendered output matched existing bytes.
    pub files_skipped_no_change: u32,
    /// Number of files skipped because duplicate note IDs made writes unsafe.
    pub files_skipped_duplicate_id: u32,
    /// Number of files that failed before a normalized write completed.
    pub files_errored: u32,
    /// Diagnostics emitted while attempting writeback.
    pub diagnostics: Vec<ScanDiagnostic>,
}

/// Scans vault files, parses notes, plans normalization, and reports duplicate IDs.
///
/// # Errors
///
/// Returns [`VaultError`] when vault scanning or original-byte reads fail.
pub fn scan_vault(
    vault_root: &Path,
    config: &EffectiveConfig,
) -> Result<VaultScanResult, VaultError> {
    let file_set = scan_vault_files(vault_root, config)?;
    let mut notes = Vec::new();
    let mut diagnostics = file_set.diagnostics;
    let mut frontmatter_parse_failures = Vec::new();

    for candidate in &file_set.notes {
        let note_scan = scan_note_candidate(candidate, config)?;
        diagnostics.extend(note_scan.diagnostics);
        if let Some(frontmatter_parse_failure) = note_scan.frontmatter_parse_failure {
            frontmatter_parse_failures.push(frontmatter_parse_failure);
        }
        notes.push(note_scan.note);
    }

    let duplicate_ids = collect_duplicate_ids(&notes);
    for (duplicate_id, paths) in &duplicate_ids {
        for duplicate_path in paths {
            diagnostics.push(ScanDiagnostic {
                level: DiagnosticLevel::Error,
                path: duplicate_path.clone(),
                message: format!("duplicate note id detected: {duplicate_id}"),
            });
        }
    }

    Ok(VaultScanResult {
        notes,
        assets: file_set.assets,
        diagnostics,
        frontmatter_parse_failures,
        duplicate_ids,
        trashed_note_count: file_set.trashed_note_count,
    })
}

/// Builds a reusable note-id lookup index without parsing note bodies.
///
/// # Errors
///
/// Returns [`VaultError`] when the vault walk itself cannot be established.
pub fn build_note_path_index(
    vault_root: &Path,
    config: &EffectiveConfig,
) -> Result<NotePathIndexBuild, VaultError> {
    let file_set = scan_vault_files(vault_root, config)?;
    let mut diagnostics = file_set.diagnostics;
    let mut paths_by_id: BTreeMap<String, Vec<RelativePath>> = BTreeMap::new();

    for candidate in &file_set.notes {
        match parse_note_frontmatter(&candidate.absolute_path) {
            Ok(Some(frontmatter)) => {
                let Some(note_id) = frontmatter.id else {
                    continue;
                };
                paths_by_id
                    .entry(note_id.to_string())
                    .or_default()
                    .push(candidate.relative_path.clone());
            }
            Ok(None) => {}
            Err(error) => diagnostics.push(note_path_index_error(candidate, &error)),
        }
    }

    let mut index = NotePathIndex::new();
    for (note_id_text, paths) in paths_by_id {
        let note_id = NoteId::new(note_id_text.as_str());
        let has_duplicates = paths.len() > 1;

        for path in paths {
            index.insert(note_id.clone(), path.clone());
            if has_duplicates {
                diagnostics.push(ScanDiagnostic {
                    level: DiagnosticLevel::Error,
                    path,
                    message: format!("duplicate note id detected: {note_id}"),
                });
            }
        }
    }

    Ok(NotePathIndexBuild { index, diagnostics })
}

struct NoteScanOutcome {
    note: NormalizationResult,
    diagnostics: Vec<ScanDiagnostic>,
    frontmatter_parse_failure: Option<FrontmatterParseFailure>,
}

fn scan_note_candidate(
    candidate: &NoteCandidate,
    config: &EffectiveConfig,
) -> Result<NoteScanOutcome, VaultError> {
    let original_bytes = fs::read(&candidate.absolute_path).map_err(|source| VaultError::Io {
        path: candidate.absolute_path.clone(),
        source,
    })?;
    let content_hash = ContentHash::from_bytes(&original_bytes);

    if candidate.trash_state == TrashState::Trashed {
        return scan_trashed_note_candidate(candidate, config, content_hash);
    }

    match parse_note_file(&candidate.absolute_path) {
        Ok(raw_document) => {
            let body_analysis = analyze_body(&raw_document.body_markdown);
            let plan = plan_normalization_with_body_analysis(
                &raw_document,
                candidate,
                config,
                &body_analysis,
            );
            let mut diagnostics = plan.diagnostics.clone();

            if raw_document.frontmatter.is_none() && !config.allow_insert_frontmatter {
                return Ok(NoteScanOutcome {
                    note: empty_normalization_result(candidate, content_hash, plan),
                    diagnostics,
                    frontmatter_parse_failure: None,
                });
            }

            let normalization_context = NormalizationContext {
                relative_path: &candidate.relative_path,
                body_analysis: &body_analysis,
                config,
                is_immutable: candidate.is_immutable,
            };
            let raw_frontmatter = raw_document
                .frontmatter
                .unwrap_or_else(empty_raw_frontmatter);
            let (normalized_frontmatter, normalize_diagnostics) =
                normalize_frontmatter(raw_frontmatter, &normalization_context);
            diagnostics.extend(normalize_diagnostics);

            let document = NoteDocument {
                frontmatter: normalized_frontmatter.clone(),
                body_markdown: raw_document.body_markdown,
            };

            Ok(NoteScanOutcome {
                note: NormalizationResult {
                    relative_path: candidate.relative_path.clone(),
                    note_id: Some(normalized_frontmatter.id.clone()),
                    trash_state: candidate.trash_state,
                    trashed_at: candidate.trashed_at.clone(),
                    plan,
                    frontmatter: Some(normalized_frontmatter),
                    document: Some(document),
                    content_hash,
                },
                diagnostics,
                frontmatter_parse_failure: None,
            })
        }
        Err(VaultError::FrontmatterParse { path: _, source }) => Ok(NoteScanOutcome {
            note: empty_normalization_result(
                candidate,
                content_hash,
                empty_plan(candidate.is_immutable),
            ),
            diagnostics: Vec::new(),
            frontmatter_parse_failure: Some(FrontmatterParseFailure {
                path: candidate.relative_path.clone(),
                message: source.to_string(),
            }),
        }),
        Err(parse_error) => Ok(NoteScanOutcome {
            note: empty_normalization_result(
                candidate,
                content_hash,
                empty_plan(candidate.is_immutable),
            ),
            diagnostics: vec![ScanDiagnostic {
                level: DiagnosticLevel::Error,
                path: candidate.relative_path.clone(),
                message: format!("failed to parse note during vault scan: {parse_error}"),
            }],
            frontmatter_parse_failure: None,
        }),
    }
}

/// Applies planned normalization to notes and writes only changed files.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when the vault root cannot be accessed.
pub fn apply_normalization(
    vault_root: &Path,
    result: &VaultScanResult,
) -> Result<NormalizationSummary, VaultError> {
    let root_metadata = fs::metadata(vault_root).map_err(|source| VaultError::Io {
        path: vault_root.to_path_buf(),
        source,
    })?;
    if !root_metadata.is_dir() {
        return Err(VaultError::Io {
            path: vault_root.to_path_buf(),
            source: io::Error::new(
                ErrorKind::InvalidInput,
                "while attempting to apply normalization to a non-directory root",
            ),
        });
    }

    let duplicate_paths = collect_duplicate_path_set(&result.duplicate_ids);
    let mut summary = NormalizationSummary {
        files_written: 0,
        files_skipped_immutable: 0,
        files_skipped_no_change: 0,
        files_skipped_duplicate_id: 0,
        files_errored: 0,
        diagnostics: Vec::new(),
    };

    for note_result in &result.notes {
        if duplicate_paths.contains(&note_result.relative_path) {
            summary.files_skipped_duplicate_id += 1;
            continue;
        }
        if note_result.plan.is_immutable {
            summary.files_skipped_immutable += 1;
            continue;
        }
        if !plan_has_changes(&note_result.plan) {
            summary.files_skipped_no_change += 1;
            continue;
        }

        let Some(note_document) = &note_result.document else {
            summary.files_errored += 1;
            summary.diagnostics.push(ScanDiagnostic {
                level: DiagnosticLevel::Error,
                path: note_result.relative_path.clone(),
                message: "cannot apply normalization because parsed note document is missing"
                    .to_string(),
            });
            continue;
        };

        let rendered_content =
            match render_note_file(&note_document.frontmatter, &note_document.body_markdown) {
                Ok(rendered_content) => rendered_content,
                Err(render_error) => {
                    summary.files_errored += 1;
                    summary.diagnostics.push(ScanDiagnostic {
                        level: DiagnosticLevel::Error,
                        path: note_result.relative_path.clone(),
                        message: format!(
                            "failed to render normalized note content: {render_error}"
                        ),
                    });
                    continue;
                }
            };
        let target_path = resolve_note_path(vault_root, &note_result.relative_path);
        let current_bytes = match fs::read(&target_path) {
            Ok(bytes) => bytes,
            Err(source) => {
                summary.files_errored += 1;
                summary.diagnostics.push(ScanDiagnostic {
                    level: DiagnosticLevel::Error,
                    path: note_result.relative_path.clone(),
                    message: format!("failed to read note before write: {source}"),
                });
                continue;
            }
        };

        if current_bytes == rendered_content.as_bytes() {
            summary.files_skipped_no_change += 1;
            continue;
        }

        match atomic_write_file(&target_path, rendered_content.as_bytes()) {
            Ok(()) => summary.files_written += 1,
            Err(write_error) => {
                summary.files_errored += 1;
                summary.diagnostics.push(ScanDiagnostic {
                    level: DiagnosticLevel::Error,
                    path: note_result.relative_path.clone(),
                    message: format!("failed to apply normalized write: {write_error}"),
                });
            }
        }
    }

    Ok(summary)
}

fn empty_plan(is_immutable: bool) -> NormalizationPlan {
    NormalizationPlan {
        needs_id_injection: false,
        needs_title_derivation: false,
        title_source: TitleSource::Frontmatter,
        needs_frontmatter_insertion: false,
        needs_field_reorder: false,
        is_immutable,
        diagnostics: Vec::new(),
    }
}

fn note_path_index_error(candidate: &NoteCandidate, error: &VaultError) -> ScanDiagnostic {
    ScanDiagnostic {
        level: DiagnosticLevel::Error,
        path: candidate.relative_path.clone(),
        message: format!("failed to parse note while building note path index: {error}"),
    }
}

fn empty_normalization_result(
    candidate: &NoteCandidate,
    content_hash: ContentHash,
    plan: NormalizationPlan,
) -> NormalizationResult {
    NormalizationResult {
        relative_path: candidate.relative_path.clone(),
        note_id: None,
        trash_state: candidate.trash_state,
        trashed_at: candidate.trashed_at.clone(),
        plan,
        frontmatter: None,
        document: None,
        content_hash,
    }
}

fn collect_duplicate_ids(notes: &[NormalizationResult]) -> Vec<(NoteId, Vec<RelativePath>)> {
    let mut paths_by_id: BTreeMap<String, Vec<RelativePath>> = BTreeMap::new();

    for note in notes {
        let Some(note_id) = &note.note_id else {
            continue;
        };
        paths_by_id
            .entry(note_id.to_string())
            .or_default()
            .push(note.relative_path.clone());
    }

    let mut duplicates = Vec::new();
    for (id_text, mut paths) in paths_by_id {
        if paths.len() < 2 {
            continue;
        }
        paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        duplicates.push((NoteId::new(id_text.as_str()), paths));
    }

    duplicates
}

fn collect_duplicate_path_set(
    duplicate_ids: &[(NoteId, Vec<RelativePath>)],
) -> HashSet<RelativePath> {
    let mut duplicate_paths = HashSet::new();
    for (_, paths) in duplicate_ids {
        for path in paths {
            let _ = duplicate_paths.insert(path.clone());
        }
    }
    duplicate_paths
}

fn plan_has_changes(plan: &NormalizationPlan) -> bool {
    plan.needs_id_injection
        || plan.needs_title_derivation
        || plan.needs_frontmatter_insertion
        || plan.needs_field_reorder
}

fn resolve_note_path(vault_root: &Path, relative_path: &RelativePath) -> PathBuf {
    vault_root.join(relative_path.as_str())
}

fn scan_trashed_note_candidate(
    candidate: &NoteCandidate,
    config: &EffectiveConfig,
    content_hash: ContentHash,
) -> Result<NoteScanOutcome, VaultError> {
    let lenient_parse = parse_note_file_lenient(&candidate.absolute_path)?;

    if let Some(reason) = lenient_parse.unknown_identity_reason {
        return Ok(NoteScanOutcome {
            note: empty_normalization_result(
                candidate,
                content_hash,
                empty_plan(candidate.is_immutable),
            ),
            diagnostics: vec![trashed_unknown_identity_diagnostic(candidate, reason)],
            frontmatter_parse_failure: None,
        });
    }

    let body_analysis = analyze_body(&lenient_parse.document.body_markdown);
    let mut plan = plan_normalization_with_body_analysis(
        &lenient_parse.document,
        candidate,
        config,
        &body_analysis,
    );
    disable_plan_writes_for_trashed_note(&mut plan);
    let mut diagnostics = plan.diagnostics.clone();

    let normalization_context = NormalizationContext {
        relative_path: &candidate.relative_path,
        body_analysis: &body_analysis,
        config,
        is_immutable: candidate.is_immutable,
    };
    let raw_frontmatter = lenient_parse
        .document
        .frontmatter
        .unwrap_or_else(empty_raw_frontmatter);
    let (normalized_frontmatter, normalize_diagnostics) =
        normalize_frontmatter(raw_frontmatter, &normalization_context);
    diagnostics.extend(normalize_diagnostics);

    let document = NoteDocument {
        frontmatter: normalized_frontmatter.clone(),
        body_markdown: lenient_parse.document.body_markdown,
    };

    Ok(NoteScanOutcome {
        note: NormalizationResult {
            relative_path: candidate.relative_path.clone(),
            note_id: Some(normalized_frontmatter.id.clone()),
            trash_state: candidate.trash_state,
            trashed_at: candidate.trashed_at.clone(),
            plan,
            frontmatter: Some(normalized_frontmatter),
            document: Some(document),
            content_hash,
        },
        diagnostics,
        frontmatter_parse_failure: None,
    })
}

fn disable_plan_writes_for_trashed_note(plan: &mut NormalizationPlan) {
    plan.needs_id_injection = false;
    plan.needs_title_derivation = false;
    plan.needs_frontmatter_insertion = false;
    plan.needs_field_reorder = false;
}

fn trashed_unknown_identity_diagnostic(
    candidate: &NoteCandidate,
    reason: TrashedUnknownIdentityReason,
) -> ScanDiagnostic {
    let (level, reason_text) = match reason {
        TrashedUnknownIdentityReason::MissingFrontmatter => {
            (DiagnosticLevel::Info, "frontmatter is missing")
        }
        TrashedUnknownIdentityReason::InvalidFrontmatter => {
            (DiagnosticLevel::Warning, "frontmatter is invalid")
        }
        TrashedUnknownIdentityReason::MissingId => {
            (DiagnosticLevel::Warning, "frontmatter id is missing")
        }
    };

    ScanDiagnostic {
        level,
        path: candidate.relative_path.clone(),
        message: format!(
            "{}: {reason_text}",
            RuleId::TrashedUnknownIdentity.description()
        ),
    }
}
