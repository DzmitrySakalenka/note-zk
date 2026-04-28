// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::BTreeMap;

use zk_domain::{
    config::EffectiveConfig,
    derived::{AssetReference, ExternalLink, ParsedNoteIndex, ResolvedNoteLink},
    id::{ContentHash, NoteId, RelativePath},
    note::{DiagnosticLevel, NoteDocument, NoteFrontmatter},
};
use zk_vault::{
    NormalizationPlan, NormalizationResult, TitleSource, TrashState, VaultScanResult,
    resolve_vault_links, resolve_vault_links_with_diagnostics,
};

fn base_config() -> EffectiveConfig {
    EffectiveConfig {
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

fn empty_plan() -> NormalizationPlan {
    NormalizationPlan {
        needs_id_injection: false,
        needs_title_derivation: false,
        title_source: TitleSource::Frontmatter,
        needs_frontmatter_insertion: false,
        needs_field_reorder: false,
        is_immutable: false,
        diagnostics: Vec::new(),
    }
}

fn frontmatter_with(id: &str, title: &str, aliases: &[&str]) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::new(id),
        title: title.to_string(),
        tags: vec!["tag".to_string()],
        note_type: None,
        aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: BTreeMap::new(),
    }
}

fn make_note(
    path: &str,
    id: &str,
    title: &str,
    aliases: &[&str],
    body: &str,
) -> NormalizationResult {
    let frontmatter = frontmatter_with(id, title, aliases);
    let document = NoteDocument {
        frontmatter: frontmatter.clone(),
        body_markdown: body.to_string(),
    };

    NormalizationResult {
        relative_path: RelativePath::new(path),
        note_id: Some(frontmatter.id.clone()),
        trash_state: TrashState::Live,
        trashed_at: None,
        plan: empty_plan(),
        frontmatter: Some(frontmatter),
        document: Some(document),
        content_hash: ContentHash::from_bytes(path.as_bytes()),
    }
}

fn make_note_without_frontmatter(path: &str) -> NormalizationResult {
    NormalizationResult {
        relative_path: RelativePath::new(path),
        note_id: None,
        trash_state: TrashState::Live,
        trashed_at: None,
        plan: empty_plan(),
        frontmatter: None,
        document: None,
        content_hash: ContentHash::from_bytes(path.as_bytes()),
    }
}

fn scan_with_notes(
    notes: Vec<NormalizationResult>,
    duplicate_ids: Vec<(NoteId, Vec<RelativePath>)>,
) -> VaultScanResult {
    VaultScanResult {
        notes,
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids,
        trashed_note_count: 0,
    }
}

fn find_index<'a>(indexes: &'a [ParsedNoteIndex], note_id: &str) -> &'a ParsedNoteIndex {
    let expected_note_id = NoteId::new(note_id);
    indexes
        .iter()
        .find(|index| index.note_id == expected_note_id)
        .expect("expected parsed note index should exist")
}

#[test]
fn wiki_link_resolves_to_target_note_id() {
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/source.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAA",
                "Source",
                &[],
                "See [[Target Note]].",
            ),
            make_note(
                "notes/target.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAB",
                "Target Note",
                &[],
                "Body",
            ),
        ],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());
    let source = find_index(&resolved, "01ARZ3NDEKTSV4RRFFQ69G5FAA");

    assert_eq!(
        source.outbound_note_links,
        vec![ResolvedNoteLink {
            source_note_id: NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAA"),
            raw_target: "Target Note".to_string(),
            target_note_id: Some(NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAB")),
            link_text: None,
            is_ambiguous: false,
        }]
    );
}

#[test]
fn external_markdown_link_produces_external_link() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/source.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAC",
            "Source",
            &[],
            "Read [Docs](https://example.com/docs).",
        )],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());
    let source = find_index(&resolved, "01ARZ3NDEKTSV4RRFFQ69G5FAC");

    assert_eq!(
        source.outbound_external_links,
        vec![ExternalLink {
            source_note_id: NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAC"),
            url: "https://example.com/docs".to_string(),
            link_text: Some("Docs".to_string()),
        }]
    );
}

#[test]
fn image_reference_produces_asset_reference_with_no_target_asset_id() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/source.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAD",
            "Source",
            &[],
            "![Diagram](./assets/diagram.png)",
        )],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());
    let source = find_index(&resolved, "01ARZ3NDEKTSV4RRFFQ69G5FAD");

    assert_eq!(
        source.asset_references,
        vec![AssetReference {
            source_note_id: NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAD"),
            raw_path: "./assets/diagram.png".to_string(),
            target_asset_id: None,
        }]
    );
}

#[test]
fn unresolved_wiki_link_sets_none_and_non_ambiguous() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/source.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAE",
            "Source",
            &[],
            "See [[Missing Note]].",
        )],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());
    let source = find_index(&resolved, "01ARZ3NDEKTSV4RRFFQ69G5FAE");

    assert_eq!(
        source.outbound_note_links,
        vec![ResolvedNoteLink {
            source_note_id: NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAE"),
            raw_target: "Missing Note".to_string(),
            target_note_id: None,
            link_text: None,
            is_ambiguous: false,
        }]
    );
}

#[test]
fn ambiguous_wiki_link_sets_ambiguous_flag() {
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/source.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAF",
                "Source",
                &[],
                "See [[Shared Target]].",
            ),
            make_note(
                "notes/a.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAG",
                "Shared Target",
                &[],
                "A",
            ),
            make_note(
                "notes/b.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAH",
                "Shared Target",
                &[],
                "B",
            ),
        ],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());
    let source = find_index(&resolved, "01ARZ3NDEKTSV4RRFFQ69G5FAF");

    assert_eq!(
        source.outbound_note_links,
        vec![ResolvedNoteLink {
            source_note_id: NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAF"),
            raw_target: "Shared Target".to_string(),
            target_note_id: None,
            link_text: None,
            is_ambiguous: true,
        }]
    );
}

#[test]
fn note_path_markdown_link_resolves_to_target_note_id() {
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/source.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAJ",
                "Source",
                &[],
                "See [Target](./notes/target.md).",
            ),
            make_note(
                "notes/target.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAK",
                "Different Title",
                &[],
                "Body",
            ),
        ],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());
    let source = find_index(&resolved, "01ARZ3NDEKTSV4RRFFQ69G5FAJ");

    assert_eq!(
        source.outbound_note_links,
        vec![ResolvedNoteLink {
            source_note_id: NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAJ"),
            raw_target: "./notes/target.md".to_string(),
            target_note_id: Some(NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAK")),
            link_text: Some("Target".to_string()),
            is_ambiguous: false,
        }]
    );
}

#[test]
fn notes_without_frontmatter_are_excluded_from_output() {
    let scan = scan_with_notes(
        vec![
            make_note_without_frontmatter("notes/missing-frontmatter.md"),
            make_note(
                "notes/valid.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAL",
                "Valid",
                &[],
                "Body",
            ),
        ],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());

    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].note_id,
        NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAL")
    );
}

#[test]
fn duplicate_id_notes_are_excluded_from_output() {
    let duplicate_id = NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAM");
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/a.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAM",
                "Duplicate A",
                &[],
                "Body",
            ),
            make_note(
                "notes/b.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAM",
                "Duplicate B",
                &[],
                "Body",
            ),
            make_note(
                "notes/unique.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAN",
                "Unique",
                &[],
                "Body",
            ),
        ],
        vec![(
            duplicate_id,
            vec![
                RelativePath::new("notes/a.md"),
                RelativePath::new("notes/b.md"),
            ],
        )],
    );

    let resolved = resolve_vault_links(&scan, &base_config());

    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].note_id,
        NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAN")
    );
}

#[test]
fn duplicate_id_notes_detected_during_resolution_return_diagnostics() {
    let duplicate_id = NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAT");
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/a.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAT",
                "Duplicate A",
                &[],
                "Body",
            ),
            make_note(
                "notes/b.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAT",
                "Duplicate B",
                &[],
                "Body",
            ),
            make_note(
                "notes/unique.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAU",
                "Unique",
                &[],
                "Body",
            ),
        ],
        Vec::new(),
    );

    let outcome = resolve_vault_links_with_diagnostics(&scan, &base_config());

    assert_eq!(outcome.parsed_indexes.len(), 1);
    assert_eq!(
        outcome.parsed_indexes[0].note_id,
        NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAU")
    );
    assert_eq!(
        outcome.duplicate_ids,
        vec![(
            duplicate_id.clone(),
            vec![
                RelativePath::new("notes/a.md"),
                RelativePath::new("notes/b.md"),
            ],
        )]
    );
    assert_eq!(outcome.diagnostics.len(), 2);
    assert!(outcome.diagnostics.iter().all(|diagnostic| {
        diagnostic.level == DiagnosticLevel::Error
            && diagnostic.message == format!("duplicate note id detected: {duplicate_id}")
    }));
}

#[test]
fn has_mermaid_is_populated_from_body_analysis() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/mermaid.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAP",
            "Mermaid",
            &[],
            "# Mermaid\n```mermaid\ngraph TD\nA --> B\n```\n",
        )],
        Vec::new(),
    );

    let resolved = resolve_vault_links(&scan, &base_config());
    let source = find_index(&resolved, "01ARZ3NDEKTSV4RRFFQ69G5FAP");

    assert!(source.has_mermaid);
}

#[test]
fn empty_vault_returns_empty_output() {
    let scan = scan_with_notes(Vec::new(), Vec::new());

    assert!(resolve_vault_links(&scan, &base_config()).is_empty());
}
