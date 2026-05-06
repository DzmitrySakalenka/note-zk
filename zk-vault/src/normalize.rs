use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::{DiagnosticLevel, NoteFrontmatter, RawNoteFrontmatter, ScanDiagnostic},
};

use crate::{
    normalize_reorder::requires_field_reorder,
    parse::{BodyAnalysis, RawNoteDocument, analyze_body},
    scan::NoteCandidate,
};

/// Source used when deriving a missing effective note title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    /// The title already existed in frontmatter.
    Frontmatter,
    /// The title was derived from the first Markdown H1.
    H1,
    /// The title was derived from the file name stem.
    FilenameStem,
}

enum ChosenTitleSource<'a> {
    Frontmatter(&'a str),
    H1(&'a str),
    FilenameStem,
}

impl ChosenTitleSource<'_> {
    fn source(&self) -> TitleSource {
        match self {
            Self::Frontmatter(_) => TitleSource::Frontmatter,
            Self::H1(_) => TitleSource::H1,
            Self::FilenameStem => TitleSource::FilenameStem,
        }
    }
}

/// Inputs required to normalize one raw note frontmatter payload.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizationContext<'a> {
    /// Vault-relative path of the note being normalized.
    pub relative_path: &'a RelativePath,
    /// Precomputed body analysis for the note body.
    pub body_analysis: &'a BodyAnalysis,
    /// Effective configuration controlling normalization behavior.
    pub config: &'a EffectiveConfig,
    /// Whether the note path is protected by immutable-folder policy.
    pub is_immutable: bool,
}

/// Planned normalization changes and informational diagnostics for one note.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct NormalizationPlan {
    /// Whether a missing stable note ID should be inserted.
    pub needs_id_injection: bool,
    /// Whether the effective title must be derived.
    pub needs_title_derivation: bool,
    /// Chosen source for any derived title.
    pub title_source: TitleSource,
    /// Whether an entire frontmatter block must be inserted.
    pub needs_frontmatter_insertion: bool,
    /// Whether canonical field ordering should be restored.
    pub needs_field_reorder: bool,
    /// Whether immutable-folder policy prevents writeback.
    pub is_immutable: bool,
    /// Informational scan diagnostics emitted while planning normalization.
    pub diagnostics: Vec<ScanDiagnostic>,
}

/// Normalizes one raw frontmatter payload using the default note ID generator.
#[must_use]
pub fn normalize_frontmatter(
    raw: RawNoteFrontmatter,
    ctx: &NormalizationContext<'_>,
) -> (NoteFrontmatter, Vec<ScanDiagnostic>) {
    normalize_frontmatter_with_id_generator(raw, ctx, NoteId::generate)
}

/// Normalizes one raw frontmatter payload using a caller-provided note ID generator.
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

/// Plans the normalization changes required for one parsed note candidate.
#[must_use]
pub fn plan_normalization(
    raw_doc: &RawNoteDocument,
    candidate: &NoteCandidate,
    config: &EffectiveConfig,
) -> NormalizationPlan {
    let body_analysis = analyze_body(&raw_doc.body_markdown);
    plan_normalization_with_body_analysis(raw_doc, candidate, config, &body_analysis)
}

pub(crate) fn plan_normalization_with_body_analysis(
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
    let title_source = choose_title_source(raw_frontmatter, &ctx).source();

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

pub(crate) fn empty_raw_frontmatter() -> RawNoteFrontmatter {
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

fn derive_title(raw: &RawNoteFrontmatter, ctx: &NormalizationContext<'_>) -> (String, TitleSource) {
    match choose_title_source(raw, ctx) {
        ChosenTitleSource::Frontmatter(title) => (title.to_string(), TitleSource::Frontmatter),
        ChosenTitleSource::H1(title) => (title.to_string(), TitleSource::H1),
        ChosenTitleSource::FilenameStem => (
            derive_title_from_filename_stem(ctx.relative_path),
            TitleSource::FilenameStem,
        ),
    }
}

fn choose_title_source<'a>(
    raw: &'a RawNoteFrontmatter,
    ctx: &'a NormalizationContext<'_>,
) -> ChosenTitleSource<'a> {
    if let Some(title) = raw
        .title
        .as_deref()
        .filter(|title| has_non_whitespace_content(title))
    {
        return ChosenTitleSource::Frontmatter(title);
    }

    if ctx.config.derive_title_from_h1
        && let Some(title) = ctx
            .body_analysis
            .first_h1
            .as_deref()
            .filter(|title| has_non_whitespace_content(title))
    {
        return ChosenTitleSource::H1(title);
    }

    ChosenTitleSource::FilenameStem
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
