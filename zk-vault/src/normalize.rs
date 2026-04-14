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
