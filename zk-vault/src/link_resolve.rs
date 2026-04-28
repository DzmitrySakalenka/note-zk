//! Internal link-classification and resolution types for the vault-aware pass.

use std::collections::{BTreeMap, HashMap, HashSet};

use zk_domain::{
    config::EffectiveConfig,
    derived::{AssetReference, ExternalLink, ParsedNoteIndex, ResolvedNoteLink},
    id::{NoteId, RelativePath},
    note::{DiagnosticLevel, ScanDiagnostic},
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
        /// Unresolved note target text.
        raw_target: String,
        /// Optional rendered link text to preserve in derived output.
        link_text: Option<String>,
    },
    /// Markdown link with an `http://` or `https://` scheme.
    ExternalUrl {
        /// Raw URL destination.
        url: String,
        /// Optional rendered link text to preserve in derived output.
        link_text: Option<String>,
    },
    /// Markdown link whose extension does not match a known note extension.
    AssetPath {
        /// Raw vault-relative or relative asset path.
        raw_path: String,
        /// Optional rendered link text to preserve in derived output.
        link_text: Option<String>,
    },
    /// Markdown link whose extension matches a known note extension (e.g. `.md`).
    LocalNotePathLink {
        /// Raw note path before lookup resolution.
        raw_path: String,
        /// Optional rendered link text to preserve in derived output.
        link_text: Option<String>,
    },
}

/// Case-insensitive lookup index built from a vault scan, used to resolve
/// note link targets. Lookup precedence: exact title, then alias, then
/// filename basename, then relative path (with or without extension).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteLookupIndex {
    /// Case-insensitive title lookup buckets.
    pub titles: HashMap<String, Vec<NoteId>>,
    /// Case-insensitive alias lookup buckets.
    pub aliases: HashMap<String, Vec<NoteId>>,
    /// Case-insensitive filename-stem lookup buckets.
    pub basenames: HashMap<String, Vec<NoteId>>,
    /// Case-insensitive relative-path lookup buckets.
    pub paths: HashMap<String, Vec<NoteId>>,
}

/// Lookup-index build output, including diagnostics for IDs filtered out of link resolution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteLookupIndexBuild {
    /// Case-insensitive note lookup index built from unique note IDs.
    pub index: NoteLookupIndex,
    /// Additional scan diagnostics discovered while building the lookup index.
    pub diagnostics: Vec<ScanDiagnostic>,
    /// Duplicate note IDs grouped with the paths that share each ID.
    pub duplicate_ids: Vec<(NoteId, Vec<RelativePath>)>,
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

/// Builds a note lookup index from normalized scan results.
#[must_use]
pub fn build_from_scan(scan_result: &VaultScanResult) -> NoteLookupIndexBuild {
    let duplicate_ids = collect_duplicate_ids(scan_result);
    let duplicate_id_set: HashSet<NoteId> = duplicate_ids
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
        if duplicate_id_set.contains(&frontmatter.id) {
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

    NoteLookupIndexBuild {
        index,
        diagnostics: duplicate_id_diagnostics(scan_result, &duplicate_ids),
        duplicate_ids,
    }
}

fn collect_duplicate_ids(scan_result: &VaultScanResult) -> Vec<(NoteId, Vec<RelativePath>)> {
    let mut paths_by_id: BTreeMap<String, Vec<RelativePath>> = BTreeMap::new();

    for (note_id, paths) in &scan_result.duplicate_ids {
        let entry = paths_by_id.entry(note_id.to_string()).or_default();
        for path in paths {
            insert_sorted_unique_path(entry, path.clone());
        }
    }

    let mut detected_paths_by_id: BTreeMap<String, Vec<RelativePath>> = BTreeMap::new();
    for note in &scan_result.notes {
        let note_id = note
            .frontmatter
            .as_ref()
            .map(|frontmatter| &frontmatter.id)
            .or(note.note_id.as_ref());
        let Some(note_id) = note_id else {
            continue;
        };
        detected_paths_by_id
            .entry(note_id.to_string())
            .or_default()
            .push(note.relative_path.clone());
    }

    for (note_id, paths) in detected_paths_by_id {
        if paths.len() < 2 {
            continue;
        }
        let entry = paths_by_id.entry(note_id).or_default();
        for path in paths {
            insert_sorted_unique_path(entry, path);
        }
    }

    paths_by_id
        .into_iter()
        .map(|(note_id, paths)| (NoteId::new(&note_id), paths))
        .collect()
}

fn insert_sorted_unique_path(paths: &mut Vec<RelativePath>, path: RelativePath) {
    if paths.iter().any(|existing_path| existing_path == &path) {
        return;
    }
    paths.push(path);
    paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
}

fn duplicate_id_diagnostics(
    scan_result: &VaultScanResult,
    duplicate_ids: &[(NoteId, Vec<RelativePath>)],
) -> Vec<ScanDiagnostic> {
    let existing_diagnostics = scan_result
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.path.as_str().to_string(),
                diagnostic.message.clone(),
            )
        })
        .collect::<HashSet<_>>();
    let mut diagnostics = Vec::new();

    for (duplicate_id, paths) in duplicate_ids {
        let message = format!("duplicate note id detected: {duplicate_id}");
        for path in paths {
            if existing_diagnostics.contains(&(path.as_str().to_string(), message.clone())) {
                continue;
            }
            diagnostics.push(ScanDiagnostic {
                level: DiagnosticLevel::Error,
                path: path.clone(),
                message: message.clone(),
            });
        }
    }

    diagnostics
}

impl NoteLookupIndex {
    /// Resolves one raw note-link target using title, alias, basename, and path precedence.
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

/// Resolves note, external, and asset links for every uniquely identified parsed note.
#[must_use]
pub fn resolve_vault_links(
    scan_result: &VaultScanResult,
    config: &EffectiveConfig,
) -> Vec<ParsedNoteIndex> {
    resolve_vault_links_with_diagnostics(scan_result, config).parsed_indexes
}

/// Resolves vault links and returns diagnostics produced by the link-resolution stage.
#[must_use]
pub fn resolve_vault_links_with_diagnostics(
    scan_result: &VaultScanResult,
    config: &EffectiveConfig,
) -> LinkResolutionOutcome {
    let lookup_build = build_from_scan(scan_result);
    let duplicate_ids: HashSet<NoteId> = lookup_build
        .duplicate_ids
        .iter()
        .map(|(note_id, _)| note_id.clone())
        .collect();
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
            assemble_note_outputs(&frontmatter.id, &classified_links, &lookup_build.index);
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

    LinkResolutionOutcome {
        parsed_indexes,
        diagnostics: lookup_build.diagnostics,
        duplicate_ids: lookup_build.duplicate_ids,
    }
}

/// Link-resolution output plus diagnostics that should be merged into the scan outcome.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinkResolutionOutcome {
    /// Parsed per-note link/reference indexes derived from scan output.
    pub parsed_indexes: Vec<ParsedNoteIndex>,
    /// Additional scan diagnostics discovered while resolving links.
    pub diagnostics: Vec<ScanDiagnostic>,
    /// Duplicate note IDs grouped with the paths that share each ID.
    pub duplicate_ids: Vec<(NoteId, Vec<RelativePath>)>,
}

/// Classifies extracted raw links before vault-aware resolution.
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

    match path.rsplit_once('/') {
        Some((parent, _)) => Some(format!("{parent}/{stem}")),
        None => Some(stem.to_string()),
    }
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
