//! Pure heuristic analysis for LLM Wiki health checks.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::Path,
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use thiserror::Error;
use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::{NoteFrontmatter, NoteRecord},
};
use zk_index::{IndexError, LinkRepo, NoteIndexedMetadata, NoteRepo, SourcesRepo};
use zk_vault::{error::VaultError, parse_note_file};

use crate::repo::WikiLintScope;

/// Pure service that analyzes indexed wiki state and returns lint findings.
#[derive(Debug, Clone, Copy, Default)]
pub struct LintWikiService;

impl LintWikiService {
    /// Creates a lint service with no runtime state.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Runs all V1 wiki lint rules for the requested scope.
    ///
    /// # Errors
    ///
    /// Returns [`WikiLintError`] when wiki config is missing, the requested
    /// scope is outside the configured wiki root, indexed metadata is malformed,
    /// local-index queries fail, or required vault body reads fail.
    pub fn lint(
        &self,
        conn: &Connection,
        vault_root: &Path,
        config: &EffectiveConfig,
        scope: &WikiLintScope,
    ) -> Result<Vec<WikiLintFinding>, WikiLintError> {
        let wiki = config
            .wiki
            .as_ref()
            .ok_or(WikiLintError::WikiConfigMissing)?;
        let scope_root = resolve_scope_root(scope, &wiki.root_dir)?;
        let context = load_lint_context(conn, vault_root, wiki, &scope_root, scope)?;

        let mut findings = Vec::new();
        findings.extend(orphan_page_findings(&context));
        findings.extend(stale_claim_findings(&context));
        findings.extend(missing_cross_reference_findings(&context));
        findings.extend(important_concept_findings(&context));
        sort_findings(&mut findings);

        Ok(findings)
    }
}

/// One lint finding emitted by [`LintWikiService`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLintFinding {
    /// Machine-readable finding kind.
    pub kind: WikiLintFindingKind,
    /// Wiki page touched by the finding, when applicable.
    pub page_id: Option<NoteId>,
    /// Source note touched by the finding, when applicable.
    pub source_id: Option<NoteId>,
    /// Human-readable finding message.
    pub message: String,
    /// Finding severity.
    pub severity: WikiLintSeverity,
}

/// V1 wiki lint rule identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WikiLintFindingKind {
    /// Authored wiki page has no inbound links and is absent from the wiki index.
    OrphanPage,
    /// Source note was updated after a wiki page that cites it.
    StaleClaim,
    /// Page body mentions another wiki page title without linking to it.
    MissingCrossReference,
    /// Repeated raw-source concept has no matching entity page.
    ImportantConceptWithoutPage,
}

/// Severity for wiki lint findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WikiLintSeverity {
    /// Informational finding.
    Info,
    /// Non-blocking issue that should be reviewed.
    Warning,
    /// Blocking issue.
    Error,
}

/// Error returned by wiki lint analysis.
#[derive(Debug, Error)]
pub enum WikiLintError {
    /// Wiki config is required to determine roots and meta-file paths.
    #[error("wiki config missing")]
    WikiConfigMissing,
    /// Folder scope must be the wiki root or a path below it.
    #[error("wiki lint folder scope {scope} is outside wiki root {wiki_root}")]
    ScopeOutsideWikiRoot {
        /// Requested folder scope.
        scope: RelativePath,
        /// Configured wiki root.
        wiki_root: RelativePath,
    },
    /// The local index returned malformed note frontmatter JSON.
    #[error("malformed indexed frontmatter for note {note_id}: {source}")]
    MalformedFrontmatter {
        /// Note with malformed indexed frontmatter.
        note_id: NoteId,
        /// JSON decode failure.
        source: serde_json::Error,
    },
    /// A note listed by the index had no matching metadata row.
    #[error("indexed metadata missing for note {0}")]
    MissingIndexedMetadata(NoteId),
    /// The local index returned a malformed timestamp.
    #[error("malformed updated_at for note {note_id}: {value}: {source}")]
    MalformedTimestamp {
        /// Note with malformed timestamp.
        note_id: NoteId,
        /// Raw timestamp value.
        value: String,
        /// Timestamp parse failure.
        source: chrono::ParseError,
    },
    /// Local index access failed.
    #[error(transparent)]
    Index(#[from] IndexError),
    /// Vault body read or parse failed.
    #[error(transparent)]
    Vault(#[from] VaultError),
}

struct WikiLintContext {
    all_pages: Vec<WikiPage>,
    in_scope_pages: Vec<WikiPage>,
    raw_sources: Vec<RawSource>,
    source_ids_by_page: HashMap<NoteId, Vec<NoteId>>,
    source_records_by_id: HashMap<NoteId, SourceRecord>,
    outbound_targets_by_page: HashMap<NoteId, HashSet<NoteId>>,
    backlinked_page_ids: HashSet<NoteId>,
    index_linked_page_ids: HashSet<NoteId>,
}

#[derive(Clone)]
struct WikiPage {
    note_id: NoteId,
    current_path: RelativePath,
    title: String,
    note_type: WikiPageType,
    updated_at: DateTime<Utc>,
    body: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WikiPageType {
    Entity,
    Synthesis,
}

struct RawSource {
    body: String,
}

struct SourceRecord {
    current_path: RelativePath,
    updated_at: DateTime<Utc>,
}

fn load_lint_context(
    conn: &Connection,
    vault_root: &Path,
    wiki: &zk_domain::config::EffectiveWikiConfig,
    scope_root: &RelativePath,
    scope: &WikiLintScope,
) -> Result<WikiLintContext, WikiLintError> {
    let live_notes = NoteRepo::list_notes(conn)?
        .into_iter()
        .filter(|note| !note.is_deleted && !note.is_trashed)
        .collect::<Vec<_>>();
    let live_notes_by_id = live_notes
        .iter()
        .map(|note| (note.note_id.clone(), note.clone()))
        .collect::<HashMap<_, _>>();
    let all_pages = load_wiki_pages(conn, &live_notes, &wiki.root_dir)?;
    let in_scope_pages = load_in_scope_page_bodies(vault_root, &all_pages, scope_root)?;
    let source_ids_by_page = load_source_ids_by_page(conn, &in_scope_pages)?;
    let raw_source_ids = raw_source_ids_for_scope(&live_notes, wiki, scope, &source_ids_by_page);
    let source_records_by_id = load_source_records(
        conn,
        &live_notes_by_id,
        &source_ids_by_page,
        &raw_source_ids,
    )?;
    let raw_sources = load_raw_source_bodies(vault_root, &raw_source_ids, &source_records_by_id)?;
    let outbound_targets_by_page = load_outbound_targets_by_page(conn, &in_scope_pages)?;
    let backlinked_page_ids = load_backlinked_page_ids(conn, &in_scope_pages)?;
    let index_linked_page_ids = load_index_linked_page_ids(conn, &wiki.index_path)?;

    Ok(WikiLintContext {
        all_pages,
        in_scope_pages,
        raw_sources,
        source_ids_by_page,
        source_records_by_id,
        outbound_targets_by_page,
        backlinked_page_ids,
        index_linked_page_ids,
    })
}

fn load_wiki_pages(
    conn: &Connection,
    live_notes: &[NoteRecord],
    wiki_root: &RelativePath,
) -> Result<Vec<WikiPage>, WikiLintError> {
    let mut pages = Vec::new();
    for note in live_notes {
        if !path_has_prefix(note.current_path.as_str(), wiki_root) {
            continue;
        }
        let metadata = load_note_metadata(conn, &note.note_id)?;
        let frontmatter = decode_frontmatter(&note.note_id, &metadata.frontmatter_json)?;
        let Some(note_type) = wiki_page_type(frontmatter.note_type.as_deref()) else {
            continue;
        };
        pages.push(WikiPage {
            note_id: note.note_id.clone(),
            current_path: note.current_path.clone(),
            title: metadata.title,
            note_type,
            updated_at: parse_updated_at(note)?,
            body: None,
        });
    }

    pages.sort_by(|left, right| left.note_id.as_str().cmp(right.note_id.as_str()));
    Ok(pages)
}

fn load_in_scope_page_bodies(
    vault_root: &Path,
    all_pages: &[WikiPage],
    scope_root: &RelativePath,
) -> Result<Vec<WikiPage>, WikiLintError> {
    let mut pages = Vec::new();
    for page in all_pages {
        if !path_has_prefix(page.current_path.as_str(), scope_root) {
            continue;
        }
        let mut page = page.clone();
        page.body = Some(read_note_body(vault_root, &page.current_path)?);
        pages.push(page);
    }

    Ok(pages)
}

fn load_note_metadata(
    conn: &Connection,
    note_id: &NoteId,
) -> Result<NoteIndexedMetadata, WikiLintError> {
    NoteRepo::get_note_metadata(conn, note_id)?
        .ok_or_else(|| WikiLintError::MissingIndexedMetadata(note_id.clone()))
}

fn decode_frontmatter(
    note_id: &NoteId,
    frontmatter_json: &str,
) -> Result<NoteFrontmatter, WikiLintError> {
    serde_json::from_str(frontmatter_json).map_err(|source| WikiLintError::MalformedFrontmatter {
        note_id: note_id.clone(),
        source,
    })
}

fn load_source_ids_by_page(
    conn: &Connection,
    pages: &[WikiPage],
) -> Result<HashMap<NoteId, Vec<NoteId>>, WikiLintError> {
    let mut sources = HashMap::with_capacity(pages.len());
    for page in pages {
        sources.insert(
            page.note_id.clone(),
            SourcesRepo::list_sources_for_note(conn, &page.note_id)?,
        );
    }
    Ok(sources)
}

fn raw_source_ids_for_scope(
    live_notes: &[NoteRecord],
    wiki: &zk_domain::config::EffectiveWikiConfig,
    scope: &WikiLintScope,
    source_ids_by_page: &HashMap<NoteId, Vec<NoteId>>,
) -> HashSet<NoteId> {
    match scope {
        WikiLintScope::Vault => live_notes
            .iter()
            .filter(|note| path_has_prefix(note.current_path.as_str(), &wiki.raw_dir))
            .map(|note| note.note_id.clone())
            .collect(),
        WikiLintScope::Folder { .. } => source_ids_by_page
            .values()
            .flat_map(|source_ids| source_ids.iter().cloned())
            .collect(),
    }
}

fn load_source_records(
    conn: &Connection,
    live_notes_by_id: &HashMap<NoteId, NoteRecord>,
    source_ids_by_page: &HashMap<NoteId, Vec<NoteId>>,
    raw_source_ids: &HashSet<NoteId>,
) -> Result<HashMap<NoteId, SourceRecord>, WikiLintError> {
    let source_ids = source_ids_by_page
        .values()
        .flat_map(|source_ids| source_ids.iter().cloned())
        .chain(raw_source_ids.iter().cloned())
        .collect::<HashSet<_>>();
    let mut source_records = HashMap::with_capacity(source_ids.len());

    for source_id in source_ids {
        let record = match live_notes_by_id.get(&source_id) {
            Some(record) => Some(record.clone()),
            None => NoteRepo::get_note(conn, &source_id)?,
        };
        if let Some(record) = record
            && !record.is_deleted
            && !record.is_trashed
        {
            let updated_at = parse_updated_at(&record)?;
            source_records.insert(
                source_id,
                SourceRecord {
                    current_path: record.current_path,
                    updated_at,
                },
            );
        }
    }

    Ok(source_records)
}

fn load_raw_source_bodies(
    vault_root: &Path,
    raw_source_ids: &HashSet<NoteId>,
    source_records_by_id: &HashMap<NoteId, SourceRecord>,
) -> Result<Vec<RawSource>, WikiLintError> {
    let mut raw_source_ids = raw_source_ids.iter().collect::<Vec<_>>();
    raw_source_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    let mut raw_sources = Vec::new();
    for source_id in raw_source_ids {
        let Some(record) = source_records_by_id.get(source_id) else {
            continue;
        };
        raw_sources.push(RawSource {
            body: read_note_body(vault_root, &record.current_path)?,
        });
    }

    Ok(raw_sources)
}

fn load_outbound_targets_by_page(
    conn: &Connection,
    pages: &[WikiPage],
) -> Result<HashMap<NoteId, HashSet<NoteId>>, WikiLintError> {
    let mut links = HashMap::with_capacity(pages.len());
    for page in pages {
        links.insert(page.note_id.clone(), outbound_targets(conn, &page.note_id)?);
    }
    Ok(links)
}

fn load_backlinked_page_ids(
    conn: &Connection,
    pages: &[WikiPage],
) -> Result<HashSet<NoteId>, WikiLintError> {
    let mut backlinked_page_ids = HashSet::new();
    for page in pages {
        if !LinkRepo::get_backlinks(conn, &page.note_id)?.is_empty() {
            backlinked_page_ids.insert(page.note_id.clone());
        }
    }

    Ok(backlinked_page_ids)
}

fn load_index_linked_page_ids(
    conn: &Connection,
    index_path: &RelativePath,
) -> Result<HashSet<NoteId>, WikiLintError> {
    let Some(index_note) = NoteRepo::get_note_by_path(conn, index_path)? else {
        return Ok(HashSet::new());
    };

    outbound_targets(conn, &index_note.note_id)
}

fn outbound_targets(conn: &Connection, note_id: &NoteId) -> Result<HashSet<NoteId>, WikiLintError> {
    Ok(LinkRepo::get_note_links(conn, note_id)?
        .into_iter()
        .filter_map(|link| link.target_note_id)
        .collect())
}

fn orphan_page_findings(context: &WikiLintContext) -> Vec<WikiLintFinding> {
    context
        .in_scope_pages
        .iter()
        .filter(|page| {
            !context.index_linked_page_ids.contains(&page.note_id)
                && !has_backlinks(context, &page.note_id)
        })
        .map(|page| WikiLintFinding {
            kind: WikiLintFindingKind::OrphanPage,
            page_id: Some(page.note_id.clone()),
            source_id: None,
            message: "wiki page is orphaned".to_string(),
            severity: WikiLintSeverity::Warning,
        })
        .collect()
}

fn stale_claim_findings(context: &WikiLintContext) -> Vec<WikiLintFinding> {
    let mut findings = Vec::new();
    for page in &context.in_scope_pages {
        let Some(source_ids) = context.source_ids_by_page.get(&page.note_id) else {
            continue;
        };
        for source_id in source_ids {
            let Some(source) = context.source_records_by_id.get(source_id) else {
                continue;
            };
            if source.updated_at > page.updated_at {
                findings.push(WikiLintFinding {
                    kind: WikiLintFindingKind::StaleClaim,
                    page_id: Some(page.note_id.clone()),
                    source_id: Some(source_id.clone()),
                    message: "source is newer than wiki page".to_string(),
                    severity: WikiLintSeverity::Warning,
                });
            }
        }
    }

    findings
}

fn missing_cross_reference_findings(context: &WikiLintContext) -> Vec<WikiLintFinding> {
    let mut findings = Vec::new();
    for page in &context.in_scope_pages {
        let body = page.body.as_deref().unwrap_or_default();
        let linked_targets = context
            .outbound_targets_by_page
            .get(&page.note_id)
            .cloned()
            .unwrap_or_default();

        for target in &context.all_pages {
            if target.note_id == page.note_id
                || target.title.is_empty()
                || !body.contains(&target.title)
                || linked_targets.contains(&target.note_id)
            {
                continue;
            }
            findings.push(WikiLintFinding {
                kind: WikiLintFindingKind::MissingCrossReference,
                page_id: Some(page.note_id.clone()),
                source_id: None,
                message: format!("body mentions \"{}\" without a note link", target.title),
                severity: WikiLintSeverity::Warning,
            });
        }
    }

    findings
}

fn important_concept_findings(context: &WikiLintContext) -> Vec<WikiLintFinding> {
    let entity_titles = context
        .all_pages
        .iter()
        .filter(|page| page.note_type == WikiPageType::Entity)
        .map(|page| page.title.as_str())
        .collect::<HashSet<_>>();
    let mut phrase_counts = BTreeMap::<String, u32>::new();

    for source in &context.raw_sources {
        for phrase in capitalized_phrases(&source.body) {
            *phrase_counts.entry(phrase).or_default() += 1;
        }
    }

    phrase_counts
        .into_iter()
        .filter(|(phrase, count)| *count >= 3 && !entity_titles.contains(phrase.as_str()))
        .map(|(phrase, count)| WikiLintFinding {
            kind: WikiLintFindingKind::ImportantConceptWithoutPage,
            page_id: None,
            source_id: None,
            message: format!(
                "concept \"{phrase}\" appears in {count} raw sources without a wiki-entity page"
            ),
            severity: WikiLintSeverity::Warning,
        })
        .collect()
}

fn has_backlinks(context: &WikiLintContext, note_id: &NoteId) -> bool {
    context.backlinked_page_ids.contains(note_id) || context.index_linked_page_ids.contains(note_id)
}

fn capitalized_phrases(body: &str) -> BTreeSet<String> {
    let mut phrases = BTreeSet::new();

    for line in body.lines() {
        let mut current_words = Vec::new();
        for raw_word in line.split_whitespace() {
            let word = raw_word.trim_matches(|value: char| !value.is_ascii_alphanumeric());
            if is_capitalized_word(word) {
                current_words.push(word.to_string());
                continue;
            }
            add_phrase(&mut phrases, &mut current_words);
        }
        add_phrase(&mut phrases, &mut current_words);
    }

    phrases
}

fn add_phrase(phrases: &mut BTreeSet<String>, current_words: &mut Vec<String>) {
    if current_words.len() >= 2 {
        let phrase = current_words.join(" ");
        if !is_stop_phrase(&phrase) {
            phrases.insert(phrase);
        }
    }
    current_words.clear();
}

fn is_capitalized_word(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    first.is_ascii_uppercase() && chars.all(|value| value.is_ascii_alphabetic())
}

fn is_stop_phrase(phrase: &str) -> bool {
    matches!(
        phrase,
        "Table Of Contents" | "Wiki Index" | "Wiki Log" | "Raw Source"
    )
}

fn resolve_scope_root(
    scope: &WikiLintScope,
    wiki_root: &RelativePath,
) -> Result<RelativePath, WikiLintError> {
    match scope {
        WikiLintScope::Vault => Ok(wiki_root.clone()),
        WikiLintScope::Folder { path } if path_has_prefix(path.as_str(), wiki_root) => {
            Ok(path.clone())
        }
        WikiLintScope::Folder { path } => Err(WikiLintError::ScopeOutsideWikiRoot {
            scope: path.clone(),
            wiki_root: wiki_root.clone(),
        }),
    }
}

fn parse_updated_at(record: &NoteRecord) -> Result<DateTime<Utc>, WikiLintError> {
    DateTime::parse_from_rfc3339(&record.updated_at)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|source| WikiLintError::MalformedTimestamp {
            note_id: record.note_id.clone(),
            value: record.updated_at.clone(),
            source,
        })
}

fn read_note_body(
    vault_root: &Path,
    relative_path: &RelativePath,
) -> Result<String, WikiLintError> {
    let document = parse_note_file(&vault_root.join(relative_path.as_str()))?;

    Ok(document.body_markdown)
}

fn path_has_prefix(path: &str, prefix: &RelativePath) -> bool {
    path == prefix.as_str()
        || path
            .strip_prefix(prefix.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
}

fn wiki_page_type(note_type: Option<&str>) -> Option<WikiPageType> {
    match note_type {
        Some("wiki-entity") => Some(WikiPageType::Entity),
        Some("wiki-synthesis") => Some(WikiPageType::Synthesis),
        _ => None,
    }
}

fn sort_findings(findings: &mut [WikiLintFinding]) {
    findings.sort_by(|left, right| {
        (
            kind_sort_value(left.kind),
            optional_note_id_sort_value(left.page_id.as_ref()),
            optional_note_id_sort_value(left.source_id.as_ref()),
            left.message.as_str(),
        )
            .cmp(&(
                kind_sort_value(right.kind),
                optional_note_id_sort_value(right.page_id.as_ref()),
                optional_note_id_sort_value(right.source_id.as_ref()),
                right.message.as_str(),
            ))
    });
}

fn kind_sort_value(kind: WikiLintFindingKind) -> u8 {
    match kind {
        WikiLintFindingKind::OrphanPage => 0,
        WikiLintFindingKind::StaleClaim => 1,
        WikiLintFindingKind::MissingCrossReference => 2,
        WikiLintFindingKind::ImportantConceptWithoutPage => 3,
    }
}

fn optional_note_id_sort_value(note_id: Option<&NoteId>) -> &str {
    note_id.map_or("", NoteId::as_str)
}
