use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, NoteId, RelativePath},
    note::{DiagnosticLevel, NoteDocument, NoteFrontmatter, ScanDiagnostic},
};

use crate::{
    error::VaultError,
    normalize::{
        NormalizationContext, NormalizationPlan, TitleSource, empty_raw_frontmatter,
        normalize_frontmatter, plan_normalization_with_body_analysis,
    },
    parse::{analyze_body, parse_note_file},
    scan::{AssetCandidate, NoteCandidate, scan_vault_files},
    serialize::render_note_file,
    write::atomic_write_file,
};

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizationResult {
    pub relative_path: RelativePath,
    pub plan: NormalizationPlan,
    pub frontmatter: Option<NoteFrontmatter>,
    pub document: Option<NoteDocument>,
    pub content_hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VaultScanResult {
    pub notes: Vec<NormalizationResult>,
    pub assets: Vec<AssetCandidate>,
    pub diagnostics: Vec<ScanDiagnostic>,
    pub frontmatter_parse_failures: Vec<FrontmatterParseFailure>,
    pub duplicate_ids: Vec<(NoteId, Vec<RelativePath>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Structured frontmatter parse failure captured during vault scan.
pub struct FrontmatterParseFailure {
    /// Vault-relative path of the note whose frontmatter failed to parse.
    pub path: RelativePath,
    /// Human-readable YAML parser error message.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationSummary {
    pub files_written: u32,
    pub files_skipped_immutable: u32,
    pub files_skipped_no_change: u32,
    pub files_skipped_duplicate_id: u32,
    pub files_errored: u32,
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
    })
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
            render_note_file(&note_document.frontmatter, &note_document.body_markdown);
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

fn empty_normalization_result(
    candidate: &NoteCandidate,
    content_hash: ContentHash,
    plan: NormalizationPlan,
) -> NormalizationResult {
    NormalizationResult {
        relative_path: candidate.relative_path.clone(),
        plan,
        frontmatter: None,
        document: None,
        content_hash,
    }
}

fn collect_duplicate_ids(notes: &[NormalizationResult]) -> Vec<(NoteId, Vec<RelativePath>)> {
    let mut paths_by_id: BTreeMap<String, Vec<RelativePath>> = BTreeMap::new();

    for note in notes {
        let Some(frontmatter) = &note.frontmatter else {
            continue;
        };
        paths_by_id
            .entry(frontmatter.id.to_string())
            .or_default()
            .push(note.relative_path.clone());
    }

    let mut duplicates = Vec::new();
    for (id_text, mut paths) in paths_by_id {
        if paths.len() < 2 {
            continue;
        }
        paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        duplicates.push((NoteId::from(id_text.as_str()), paths));
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
