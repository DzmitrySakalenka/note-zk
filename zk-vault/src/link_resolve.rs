//! Internal link-classification and resolution types for the vault-aware pass.

use std::collections::{HashMap, HashSet};

use zk_domain::{
    config::EffectiveConfig,
    derived::{AssetReference, ExternalLink, ParsedNoteIndex, ResolvedNoteLink},
    id::{NoteId, RelativePath},
};

use crate::{
    ExtractedLinks, NormalizationResult, RawLink, VaultScanResult, analyze_body, extract_links,
};

/// A raw link classified by its likely target type, determined from syntax and
/// file extension heuristics before any resolution against the vault index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedLink {
    /// Wiki link or extensionless markdown link targeting another note by title/alias.
    NoteLink {
        raw_target: String,
        link_text: Option<String>,
    },
    /// Markdown link with an `http://` or `https://` scheme.
    ExternalUrl {
        url: String,
        link_text: Option<String>,
    },
    /// Markdown link whose extension does not match a known note extension.
    AssetPath {
        raw_path: String,
        link_text: Option<String>,
    },
    /// Markdown link whose extension matches a known note extension (e.g. `.md`).
    LocalNotePathLink {
        raw_path: String,
        link_text: Option<String>,
    },
}

/// Case-insensitive lookup index built from a vault scan, used to resolve
/// note link targets. Lookup precedence: exact title, then alias, then
/// filename basename, then relative path (with or without extension).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteLookupIndex {
    pub titles: HashMap<String, Vec<NoteId>>,
    pub aliases: HashMap<String, Vec<NoteId>>,
    pub basenames: HashMap<String, Vec<NoteId>>,
    pub paths: HashMap<String, Vec<NoteId>>,
}

/// Outcome of resolving a single link target against the lookup index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveResult {
    /// Exactly one note matched the target.
    Resolved(NoteId),
    /// Multiple notes matched at the same precedence level.
    Ambiguous,
    /// No note matched at any precedence level.
    NotFound,
}

#[must_use]
pub fn build_from_scan(scan_result: &VaultScanResult) -> NoteLookupIndex {
    let duplicate_ids: HashSet<NoteId> = scan_result
        .duplicate_ids
        .iter()
        .map(|(note_id, _)| note_id.clone())
        .collect();

    let mut index = NoteLookupIndex::default();

    for note in &scan_result.notes {
        if note.document.is_none() {
            continue;
        }

        let Some(frontmatter) = note.frontmatter.as_ref() else {
            continue;
        };
        if duplicate_ids.contains(&frontmatter.id) {
            continue;
        }

        insert_unique(
            &mut index.titles,
            normalize_lookup_key(&frontmatter.title),
            frontmatter.id.clone(),
        );

        for alias in &frontmatter.aliases {
            insert_unique(
                &mut index.aliases,
                normalize_lookup_key(alias),
                frontmatter.id.clone(),
            );
        }

        if let Some(basename) = basename_key(&note.relative_path) {
            insert_unique(&mut index.basenames, basename, frontmatter.id.clone());
        }

        insert_path_keys(&mut index.paths, note, &frontmatter.id);
    }

    index
}

impl NoteLookupIndex {
    #[must_use]
    pub fn resolve_target(&self, raw_target: &str) -> ResolveResult {
        let normalized_key = normalize_lookup_key(raw_target);

        if let Some(result) = resolve_bucket(self.titles.get(&normalized_key)) {
            return result;
        }
        if let Some(result) = resolve_bucket(self.aliases.get(&normalized_key)) {
            return result;
        }
        if let Some(result) = resolve_bucket(self.basenames.get(&normalized_key)) {
            return result;
        }

        let normalized_path = normalize_path_key(raw_target);
        if let Some(result) = resolve_bucket(self.paths.get(&normalized_path)) {
            return result;
        }

        ResolveResult::NotFound
    }
}

#[must_use]
pub fn resolve_vault_links(
    scan_result: &VaultScanResult,
    config: &EffectiveConfig,
) -> Vec<ParsedNoteIndex> {
    let duplicate_ids: HashSet<NoteId> = scan_result
        .duplicate_ids
        .iter()
        .map(|(note_id, _)| note_id.clone())
        .collect();
    let lookup_index = build_from_scan(scan_result);
    let mut parsed_indexes = Vec::with_capacity(scan_result.notes.len());

    for note in &scan_result.notes {
        let Some(frontmatter) = note.frontmatter.as_ref() else {
            continue;
        };
        let Some(document) = note.document.as_ref() else {
            continue;
        };
        if duplicate_ids.contains(&frontmatter.id) {
            continue;
        }

        let extracted_links = extract_links(&document.body_markdown);
        let classified_links = classify_links(&extracted_links, &config.allowed_note_extensions);
        let (outbound_note_links, outbound_external_links, asset_references) =
            assemble_note_outputs(&frontmatter.id, &classified_links, &lookup_index);
        let body_analysis = analyze_body(&document.body_markdown);

        parsed_indexes.push(ParsedNoteIndex {
            note_id: frontmatter.id.clone(),
            title: frontmatter.title.clone(),
            tags: frontmatter.tags.clone(),
            aliases: frontmatter.aliases.clone(),
            outbound_note_links,
            outbound_external_links,
            asset_references,
            has_mermaid: body_analysis.has_mermaid,
        });
    }

    parsed_indexes
}

#[must_use]
pub fn classify_links(
    extracted: &ExtractedLinks,
    note_extensions: &[String],
) -> Vec<ClassifiedLink> {
    let mut classified =
        Vec::with_capacity(extracted.wiki_links.len() + extracted.markdown_links.len());

    for link in &extracted.wiki_links {
        let RawLink::WikiLink { raw_target, alias } = link else {
            continue;
        };
        classified.push(ClassifiedLink::NoteLink {
            raw_target: raw_target.clone(),
            link_text: alias.clone(),
        });
    }

    for link in &extracted.markdown_links {
        let RawLink::MarkdownLink { text, destination } = link else {
            continue;
        };
        classified.push(classify_markdown_link(destination, text, note_extensions));
    }

    classified
}

fn classify_markdown_link(
    destination: &str,
    text: &str,
    note_extensions: &[String],
) -> ClassifiedLink {
    let link_text = Some(text.to_string());

    if has_http_scheme(destination) {
        return ClassifiedLink::ExternalUrl {
            url: destination.to_string(),
            link_text,
        };
    }

    if let Some(extension) = path_extension(destination) {
        if is_note_extension(extension, note_extensions) {
            return ClassifiedLink::LocalNotePathLink {
                raw_path: destination.to_string(),
                link_text,
            };
        }

        return ClassifiedLink::AssetPath {
            raw_path: destination.to_string(),
            link_text,
        };
    }

    ClassifiedLink::NoteLink {
        raw_target: destination.to_string(),
        link_text,
    }
}

fn has_http_scheme(destination: &str) -> bool {
    destination
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
        || destination
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
}

fn path_extension(destination: &str) -> Option<&str> {
    let path_segment = destination.rsplit('/').next().unwrap_or(destination);
    let extension_start = path_segment.rfind('.')?;
    let extension = &path_segment[extension_start + 1..];

    if extension.is_empty() {
        return None;
    }

    Some(extension)
}

fn is_note_extension(extension: &str, note_extensions: &[String]) -> bool {
    note_extensions.iter().any(|allowed_extension| {
        let normalized = allowed_extension.trim_start_matches('.');
        normalized.eq_ignore_ascii_case(extension)
    })
}

fn insert_path_keys(
    paths: &mut HashMap<String, Vec<NoteId>>,
    note: &NormalizationResult,
    note_id: &NoteId,
) {
    let with_extension = normalize_path_key(note.relative_path.as_str());
    insert_unique(paths, with_extension.clone(), note_id.clone());

    if let Some(without_extension) = path_without_extension(&with_extension) {
        insert_unique(paths, without_extension, note_id.clone());
    }
}

fn insert_unique(map: &mut HashMap<String, Vec<NoteId>>, key: String, note_id: NoteId) {
    let bucket = map.entry(key).or_default();
    if bucket.contains(&note_id) {
        return;
    }
    bucket.push(note_id);
}

fn normalize_lookup_key(raw_target: &str) -> String {
    raw_target.to_ascii_lowercase()
}

fn normalize_path_key(raw_target: &str) -> String {
    RelativePath::new(raw_target).as_str().to_ascii_lowercase()
}

fn basename_key(path: &RelativePath) -> Option<String> {
    let filename = path.as_str().rsplit('/').next()?;
    let basename = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);

    if basename.is_empty() {
        return None;
    }

    Some(basename.to_ascii_lowercase())
}

fn path_without_extension(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next()?;
    let (stem, _) = filename.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }

    let parent = path
        .strip_suffix(filename)
        .expect("filename should be a suffix of the path");
    Some(format!("{parent}{stem}"))
}

fn resolve_bucket(bucket: Option<&Vec<NoteId>>) -> Option<ResolveResult> {
    let note_ids = bucket?;
    match note_ids.len() {
        0 => None,
        1 => Some(ResolveResult::Resolved(note_ids[0].clone())),
        _ => Some(ResolveResult::Ambiguous),
    }
}

fn assemble_note_outputs(
    source_note_id: &NoteId,
    classified_links: &[ClassifiedLink],
    lookup_index: &NoteLookupIndex,
) -> (
    Vec<ResolvedNoteLink>,
    Vec<ExternalLink>,
    Vec<AssetReference>,
) {
    let mut outbound_note_links = Vec::new();
    let mut outbound_external_links = Vec::new();
    let mut asset_references = Vec::new();

    for classified_link in classified_links {
        match classified_link {
            ClassifiedLink::NoteLink {
                raw_target,
                link_text,
            } => outbound_note_links.push(resolve_note_link(
                source_note_id,
                raw_target,
                link_text.clone(),
                lookup_index,
            )),
            ClassifiedLink::LocalNotePathLink {
                raw_path,
                link_text,
            } => outbound_note_links.push(resolve_note_link(
                source_note_id,
                raw_path,
                link_text.clone(),
                lookup_index,
            )),
            ClassifiedLink::ExternalUrl { url, link_text } => {
                outbound_external_links.push(ExternalLink {
                    source_note_id: source_note_id.clone(),
                    url: url.clone(),
                    link_text: link_text.clone(),
                });
            }
            ClassifiedLink::AssetPath {
                raw_path,
                link_text: _,
            } => {
                asset_references.push(AssetReference {
                    source_note_id: source_note_id.clone(),
                    raw_path: raw_path.clone(),
                    target_asset_id: None,
                });
            }
        }
    }

    (
        outbound_note_links,
        outbound_external_links,
        asset_references,
    )
}

fn resolve_note_link(
    source_note_id: &NoteId,
    raw_target: &str,
    link_text: Option<String>,
    lookup_index: &NoteLookupIndex,
) -> ResolvedNoteLink {
    match lookup_index.resolve_target(raw_target) {
        ResolveResult::Resolved(target_note_id) => ResolvedNoteLink {
            source_note_id: source_note_id.clone(),
            raw_target: raw_target.to_string(),
            target_note_id: Some(target_note_id),
            link_text,
            is_ambiguous: false,
        },
        ResolveResult::Ambiguous => ResolvedNoteLink {
            source_note_id: source_note_id.clone(),
            raw_target: raw_target.to_string(),
            target_note_id: None,
            link_text,
            is_ambiguous: true,
        },
        ResolveResult::NotFound => ResolvedNoteLink {
            source_note_id: source_note_id.clone(),
            raw_target: raw_target.to_string(),
            target_note_id: None,
            link_text,
            is_ambiguous: false,
        },
    }
}
