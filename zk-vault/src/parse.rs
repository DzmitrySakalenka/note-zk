use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;
use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, NoteId, RelativePath},
    note::{DiagnosticLevel, NoteDocument, NoteFrontmatter, RawNoteFrontmatter, ScanDiagnostic},
};

use crate::{
    error::VaultError,
    scan::{AssetCandidate, NoteCandidate, scan_vault_files},
};

#[derive(Debug, Clone, PartialEq)]
pub struct RawNoteDocument {
    pub frontmatter: Option<RawNoteFrontmatter>,
    pub raw_frontmatter_yaml: Option<String>,
    pub body_markdown: String,
    pub has_frontmatter_delimiters: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyAnalysis {
    pub first_h1: Option<String>,
    pub has_mermaid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    Frontmatter,
    H1,
    FilenameStem,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizationContext<'a> {
    pub relative_path: &'a RelativePath,
    pub body_analysis: &'a BodyAnalysis,
    pub config: &'a EffectiveConfig,
    pub is_immutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct NormalizationPlan {
    pub needs_id_injection: bool,
    pub needs_title_derivation: bool,
    pub title_source: TitleSource,
    pub needs_frontmatter_insertion: bool,
    pub needs_field_reorder: bool,
    pub is_immutable: bool,
    pub diagnostics: Vec<ScanDiagnostic>,
}

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
    pub duplicate_ids: Vec<(NoteId, Vec<RelativePath>)>,
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

/// Parses a markdown file into raw frontmatter and body sections.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when file reading fails, [`VaultError::Utf8Decode`]
/// when bytes are not valid UTF-8 text, and [`VaultError::FrontmatterParse`]
/// when a detected frontmatter block contains invalid YAML.
pub fn parse_note_file(path: &Path) -> Result<RawNoteDocument, VaultError> {
    let file_bytes = fs::read(path).map_err(|source| VaultError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_content = String::from_utf8(file_bytes).map_err(|source| VaultError::Utf8Decode {
        path: path.to_path_buf(),
        source,
    })?;

    let Some((raw_yaml, body_markdown)) = split_frontmatter_sections(&file_content) else {
        return Ok(RawNoteDocument {
            frontmatter: None,
            raw_frontmatter_yaml: None,
            body_markdown: file_content,
            has_frontmatter_delimiters: false,
        });
    };

    let parsed_frontmatter =
        serde_yaml::from_str::<RawNoteFrontmatter>(raw_yaml).map_err(|source| {
            VaultError::FrontmatterParse {
                path: path.to_path_buf(),
                source,
            }
        })?;

    Ok(RawNoteDocument {
        frontmatter: Some(parsed_frontmatter),
        raw_frontmatter_yaml: Some(raw_yaml.to_string()),
        body_markdown: body_markdown.to_string(),
        has_frontmatter_delimiters: true,
    })
}

#[must_use]
pub fn extract_first_h1(body: &str) -> Option<String> {
    analyze_body(body).first_h1
}

#[must_use]
pub fn detect_mermaid(body: &str) -> bool {
    analyze_body(body).has_mermaid
}

#[must_use]
pub fn analyze_body(body: &str) -> BodyAnalysis {
    analyze_body_internal(body)
}

#[must_use]
pub fn normalize_frontmatter(
    raw: RawNoteFrontmatter,
    ctx: &NormalizationContext<'_>,
) -> (NoteFrontmatter, Vec<ScanDiagnostic>) {
    normalize_frontmatter_with_id_generator(raw, ctx, NoteId::generate)
}

pub fn normalize_frontmatter_with_id_generator<F>(
    raw: RawNoteFrontmatter,
    ctx: &NormalizationContext<'_>,
    generate_id: F,
) -> (NoteFrontmatter, Vec<ScanDiagnostic>)
where
    F: Fn() -> NoteId,
{
    let mut diagnostics = Vec::new();

    let note_id = if let Some(existing_id) = raw.id.clone() {
        existing_id
    } else {
        diagnostics.push(ScanDiagnostic {
            level: DiagnosticLevel::Info,
            path: ctx.relative_path.clone(),
            message: "injected missing id into frontmatter".to_string(),
        });
        generate_id()
    };

    let (title, title_source) = derive_title(&raw, ctx);
    if title_source != TitleSource::Frontmatter {
        diagnostics.push(ScanDiagnostic {
            level: DiagnosticLevel::Info,
            path: ctx.relative_path.clone(),
            message: format!(
                "derived missing title from {}",
                title_source_label(title_source)
            ),
        });
    }

    if is_untagged_default(&raw.tags) {
        diagnostics.push(ScanDiagnostic {
            level: DiagnosticLevel::Info,
            path: ctx.relative_path.clone(),
            message: "note uses default untagged tag".to_string(),
        });
    }

    (
        NoteFrontmatter {
            id: note_id,
            title,
            tags: raw.tags,
            note_type: raw.note_type,
            aliases: raw.aliases,
            created: raw.created,
            updated: raw.updated,
            source: raw.source,
            status: raw.status,
            sources: raw.sources,
            extra: raw.extra,
        },
        diagnostics,
    )
}

#[must_use]
pub fn plan_normalization(
    raw_doc: &RawNoteDocument,
    candidate: &NoteCandidate,
    config: &EffectiveConfig,
) -> NormalizationPlan {
    let body_analysis = analyze_body(&raw_doc.body_markdown);
    plan_normalization_with_body_analysis(raw_doc, candidate, config, &body_analysis)
}

#[must_use]
pub fn serialize_frontmatter(frontmatter: &NoteFrontmatter) -> String {
    let mut output = String::new();

    push_yaml_scalar_line(&mut output, "id", &frontmatter.id.to_string());
    push_yaml_scalar_line(&mut output, "title", &frontmatter.title);
    push_yaml_string_list(&mut output, "tags", &frontmatter.tags);

    if let Some(note_type) = &frontmatter.note_type {
        push_yaml_scalar_line(&mut output, "type", note_type);
    }
    if !frontmatter.aliases.is_empty() {
        push_yaml_string_list(&mut output, "aliases", &frontmatter.aliases);
    }
    if let Some(created) = &frontmatter.created {
        push_yaml_scalar_line(&mut output, "created", created);
    }
    if let Some(updated) = &frontmatter.updated {
        push_yaml_scalar_line(&mut output, "updated", updated);
    }
    if let Some(source) = &frontmatter.source {
        push_yaml_scalar_line(&mut output, "source", source);
    }
    if let Some(status) = &frontmatter.status {
        push_yaml_scalar_line(&mut output, "status", status);
    }
    if !frontmatter.sources.is_empty() {
        let source_values: Vec<String> = frontmatter
            .sources
            .iter()
            .map(ToString::to_string)
            .collect();
        push_yaml_string_list(&mut output, "sources", &source_values);
    }

    for (key, value) in &frontmatter.extra {
        push_yaml_extra_value(&mut output, key, value);
    }

    output
}

#[must_use]
pub fn render_note_file(frontmatter: &NoteFrontmatter, body: &str) -> String {
    let serialized_frontmatter = serialize_frontmatter(frontmatter);
    let mut output = String::new();
    output.push_str("---\n");
    output.push_str(&serialized_frontmatter);
    output.push_str("---\n");
    output.push_str(body);

    if !body.is_empty() && !body.ends_with('\n') {
        output.push('\n');
    }

    output
}

/// Writes content to a temp file in the same directory, fsyncs, and atomically
/// renames it over the destination.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when temp file creation, write, fsync, rename,
/// or cleanup operations fail.
pub fn atomic_write_file(path: &Path, content: &[u8]) -> Result<(), VaultError> {
    let parent_dir = path.parent().ok_or_else(|| VaultError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(
            ErrorKind::InvalidInput,
            "while attempting to resolve parent directory for atomic write",
        ),
    })?;

    let mut temp_file = NamedTempFile::new_in(parent_dir).map_err(|source| VaultError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    temp_file
        .write_all(content)
        .map_err(|source| VaultError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp_file
        .as_file()
        .sync_all()
        .map_err(|source| VaultError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    match temp_file.persist(path) {
        Ok(_persisted_file) => Ok(()),
        Err(persist_error) => {
            cleanup_temp_after_failed_persist(persist_error.file.path());
            Err(VaultError::Io {
                path: path.to_path_buf(),
                source: persist_error.error,
            })
        }
    }
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

    for candidate in &file_set.notes {
        let original_bytes =
            fs::read(&candidate.absolute_path).map_err(|source| VaultError::Io {
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
                diagnostics.extend(plan.diagnostics.clone());

                if raw_document.frontmatter.is_none() && !config.allow_insert_frontmatter {
                    notes.push(NormalizationResult {
                        relative_path: candidate.relative_path.clone(),
                        plan,
                        frontmatter: None,
                        document: None,
                        content_hash,
                    });
                    continue;
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

                notes.push(NormalizationResult {
                    relative_path: candidate.relative_path.clone(),
                    plan,
                    frontmatter: Some(normalized_frontmatter),
                    document: Some(document),
                    content_hash,
                });
            }
            Err(parse_error) => {
                diagnostics.push(ScanDiagnostic {
                    level: DiagnosticLevel::Error,
                    path: candidate.relative_path.clone(),
                    message: format!("failed to parse note during vault scan: {parse_error}"),
                });
                notes.push(NormalizationResult {
                    relative_path: candidate.relative_path.clone(),
                    plan: empty_plan(candidate.is_immutable),
                    frontmatter: None,
                    document: None,
                    content_hash,
                });
            }
        }
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
        duplicate_ids,
    })
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

fn split_frontmatter_sections(file_content: &str) -> Option<(&str, &str)> {
    const OPENING_DELIMITER: &str = "---\n";
    if !file_content.starts_with(OPENING_DELIMITER) {
        return None;
    }

    let frontmatter_and_body = &file_content[OPENING_DELIMITER.len()..];
    let mut line_start_index = 0usize;

    while line_start_index < frontmatter_and_body.len() {
        let search_space = &frontmatter_and_body[line_start_index..];
        if let Some(relative_newline_index) = search_space.find('\n') {
            let line_end_index = line_start_index + relative_newline_index;
            if frontmatter_and_body[line_start_index..line_end_index] == *"---" {
                let raw_frontmatter = &frontmatter_and_body[..line_start_index];
                let body_markdown = &frontmatter_and_body[line_end_index + 1..];
                return Some((raw_frontmatter, body_markdown));
            }
            line_start_index = line_end_index + 1;
            continue;
        }

        if frontmatter_and_body[line_start_index..] == *"---" {
            let raw_frontmatter = &frontmatter_and_body[..line_start_index];
            return Some((raw_frontmatter, ""));
        }
        break;
    }

    None
}

fn derive_title(raw: &RawNoteFrontmatter, ctx: &NormalizationContext<'_>) -> (String, TitleSource) {
    match choose_title_source(raw, ctx) {
        TitleSource::Frontmatter => (
            raw.title.clone().unwrap_or_else(|| {
                unreachable!(
                    "choose_title_source returned Frontmatter without a non-empty raw title"
                )
            }),
            TitleSource::Frontmatter,
        ),
        TitleSource::H1 => (
            ctx.body_analysis.first_h1.clone().unwrap_or_else(|| {
                unreachable!("choose_title_source returned H1 without a non-empty first heading")
            }),
            TitleSource::H1,
        ),
        TitleSource::FilenameStem => (
            derive_title_from_filename_stem(ctx.relative_path),
            TitleSource::FilenameStem,
        ),
    }
}

fn choose_title_source(raw: &RawNoteFrontmatter, ctx: &NormalizationContext<'_>) -> TitleSource {
    if raw.title.as_deref().is_some_and(has_non_whitespace_content) {
        return TitleSource::Frontmatter;
    }

    if ctx.config.derive_title_from_h1
        && ctx
            .body_analysis
            .first_h1
            .as_deref()
            .is_some_and(has_non_whitespace_content)
    {
        return TitleSource::H1;
    }

    TitleSource::FilenameStem
}

fn derive_title_from_filename_stem(path: &RelativePath) -> String {
    let relative_text = path.as_str();
    let filename = relative_text.rsplit('/').next().unwrap_or(relative_text);
    let stem =
        filename.rsplit_once('.').map_or(
            filename,
            |(head, _)| if head.is_empty() { filename } else { head },
        );
    let normalized = stem.replace(['-', '_'], " ");
    let words: Vec<String> = normalized
        .split_whitespace()
        .map(title_case_word)
        .filter(|word| !word.is_empty())
        .collect();

    if words.is_empty() {
        return "Untitled".to_string();
    }
    words.join(" ")
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first_char) = chars.next() else {
        return String::new();
    };

    let mut title_cased = String::new();
    title_cased.extend(first_char.to_uppercase());
    title_cased.push_str(chars.as_str());
    title_cased
}

fn title_source_label(source: TitleSource) -> &'static str {
    match source {
        TitleSource::Frontmatter => "frontmatter",
        TitleSource::H1 => "h1",
        TitleSource::FilenameStem => "filename",
    }
}

fn has_non_whitespace_content(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_untagged_default(tags: &[String]) -> bool {
    tags.len() == 1 && tags.first().is_some_and(|tag| tag == "untagged")
}

fn empty_raw_frontmatter() -> RawNoteFrontmatter {
    RawNoteFrontmatter {
        id: None,
        title: None,
        tags: vec!["untagged".to_string()],
        note_type: None,
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: std::collections::BTreeMap::default(),
    }
}

fn plan_normalization_with_body_analysis(
    raw_doc: &RawNoteDocument,
    candidate: &NoteCandidate,
    config: &EffectiveConfig,
    body_analysis: &BodyAnalysis,
) -> NormalizationPlan {
    if raw_doc.frontmatter.is_none() && !config.allow_insert_frontmatter {
        return NormalizationPlan {
            needs_id_injection: false,
            needs_title_derivation: false,
            title_source: TitleSource::Frontmatter,
            needs_frontmatter_insertion: false,
            needs_field_reorder: false,
            is_immutable: candidate.is_immutable,
            diagnostics: vec![ScanDiagnostic {
                level: DiagnosticLevel::Warning,
                path: candidate.relative_path.clone(),
                message: "cannot insert frontmatter because allow_insert_frontmatter=false"
                    .to_string(),
            }],
        };
    }

    let normalized_default_frontmatter;
    let raw_frontmatter = if let Some(raw_frontmatter) = raw_doc.frontmatter.as_ref() {
        raw_frontmatter
    } else {
        normalized_default_frontmatter = empty_raw_frontmatter();
        &normalized_default_frontmatter
    };
    let ctx = NormalizationContext {
        relative_path: &candidate.relative_path,
        body_analysis,
        config,
        is_immutable: candidate.is_immutable,
    };
    let title_source = choose_title_source(raw_frontmatter, &ctx);

    NormalizationPlan {
        needs_id_injection: raw_frontmatter.id.is_none(),
        needs_title_derivation: title_source != TitleSource::Frontmatter,
        title_source,
        needs_frontmatter_insertion: raw_doc.frontmatter.is_none(),
        needs_field_reorder: raw_doc
            .raw_frontmatter_yaml
            .as_deref()
            .is_some_and(requires_field_reorder),
        is_immutable: candidate.is_immutable,
        diagnostics: Vec::new(),
    }
}

fn requires_field_reorder(raw_frontmatter_yaml: &str) -> bool {
    let observed = collect_top_level_yaml_keys(raw_frontmatter_yaml);
    if observed.len() < 2 {
        return false;
    }

    let mut expected = observed.clone();
    expected.sort_by(|left, right| {
        let left_rank = canonical_key_rank(left);
        let right_rank = canonical_key_rank(right);
        left_rank.cmp(&right_rank).then_with(|| left.cmp(right))
    });

    observed != expected
}

fn collect_top_level_yaml_keys(raw_frontmatter_yaml: &str) -> Vec<String> {
    raw_frontmatter_yaml
        .lines()
        .filter_map(extract_top_level_key)
        .collect()
}

fn extract_top_level_key(line: &str) -> Option<String> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent != 0 {
        return None;
    }

    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }

    let colon_index = trimmed.find(':')?;
    let key = trimmed[..colon_index].trim();
    if key.is_empty() {
        return None;
    }

    Some(strip_yaml_quotes(key))
}

fn strip_yaml_quotes(key: &str) -> String {
    if key.len() >= 2
        && ((key.starts_with('"') && key.ends_with('"'))
            || (key.starts_with('\'') && key.ends_with('\'')))
    {
        return key[1..key.len() - 1].to_string();
    }
    key.to_string()
}

fn canonical_key_rank(key: &str) -> usize {
    match key {
        "id" => 0,
        "title" => 1,
        "tags" => 2,
        "type" => 3,
        "aliases" => 4,
        "created" => 5,
        "updated" => 6,
        "source" => 7,
        "status" => 8,
        "sources" => 9,
        _ => 10,
    }
}

fn push_yaml_scalar_line(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push_str(": ");
    output.push_str(value);
    output.push('\n');
}

fn push_yaml_string_list(output: &mut String, key: &str, values: &[String]) {
    output.push_str(key);
    output.push_str(":\n");
    for value in values {
        output.push_str("  - ");
        output.push_str(value);
        output.push('\n');
    }
}

fn push_yaml_extra_value(output: &mut String, key: &str, value: &serde_yaml::Value) {
    let serialized_value = sanitize_serde_yaml_value(value);
    if !serialized_value.contains('\n') {
        output.push_str(key);
        output.push_str(": ");
        output.push_str(&serialized_value);
        output.push('\n');
        return;
    }

    output.push_str(key);
    output.push_str(":\n");
    for line in serialized_value.lines() {
        output.push_str("  ");
        output.push_str(line);
        output.push('\n');
    }
}

fn sanitize_serde_yaml_value(value: &serde_yaml::Value) -> String {
    let serialized = serde_yaml::to_string(value).expect("extra value should serialize");
    let without_document_start = serialized
        .strip_prefix("---\n")
        .or_else(|| serialized.strip_prefix("---"))
        .unwrap_or(&serialized);

    without_document_start.trim_end().to_string()
}

fn cleanup_temp_after_failed_persist(temp_path: &Path) {
    let _ = fs::remove_file(temp_path);
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

fn analyze_body_internal(body: &str) -> BodyAnalysis {
    let mut inside_fence = false;
    let mut opening_fence_len = 0usize;
    let mut first_h1 = None;
    let mut has_mermaid = false;

    for line in body.lines() {
        if let Some((backtick_count, info)) = parse_fence_line(line) {
            if inside_fence {
                if backtick_count >= opening_fence_len && info.is_empty() {
                    inside_fence = false;
                    opening_fence_len = 0;
                }
                continue;
            }

            inside_fence = true;
            opening_fence_len = backtick_count;
            if info == "mermaid" {
                has_mermaid = true;
            }
            continue;
        }

        if inside_fence || first_h1.is_some() {
            continue;
        }

        first_h1 = extract_h1_from_line(line);
    }

    BodyAnalysis {
        first_h1,
        has_mermaid,
    }
}

fn parse_fence_line(line: &str) -> Option<(usize, &str)> {
    let indent_width = line.chars().take_while(|ch| *ch == ' ').count();
    if indent_width > 3 {
        return None;
    }

    let trimmed = &line[indent_width..];
    let backtick_count = trimmed.chars().take_while(|ch| *ch == '`').count();
    if backtick_count < 3 {
        return None;
    }

    let remainder = trimmed[backtick_count..].trim();
    Some((backtick_count, remainder))
}

fn extract_h1_from_line(line: &str) -> Option<String> {
    let indent_width = line.chars().take_while(|ch| *ch == ' ').count();
    if indent_width > 3 {
        return None;
    }

    let candidate = &line[indent_width..];
    if !candidate.starts_with('#') || candidate.starts_with("##") {
        return None;
    }

    let mut heading_chars = candidate[1..].chars();
    let first_after_hash = heading_chars.next()?;
    if !first_after_hash.is_whitespace() {
        return None;
    }
    let heading_tail = heading_chars.as_str();
    Some(heading_tail.trim_end().to_string())
}
