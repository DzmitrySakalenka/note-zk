// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::BTreeMap;

use zk_domain::{
    id::{ContentHash, NoteId, RelativePath},
    note::{DiagnosticLevel, NoteDocument, NoteFrontmatter, ScanDiagnostic},
};
use zk_vault::{
    NormalizationPlan, NormalizationResult, NoteLookupIndex, NoteLookupIndexBuild, ResolveResult,
    TitleSource, TrashState, VaultScanResult, build_from_scan,
};

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

fn make_note(path: &str, id: &str, title: &str, aliases: &[&str]) -> NormalizationResult {
    let frontmatter = frontmatter_with(id, title, aliases);
    let document = NoteDocument {
        frontmatter: frontmatter.clone(),
        body_markdown: "Body".to_string(),
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

fn make_note_without_document(
    path: &str,
    id: &str,
    title: &str,
    aliases: &[&str],
) -> NormalizationResult {
    NormalizationResult {
        relative_path: RelativePath::new(path),
        note_id: Some(NoteId::new(id)),
        trash_state: TrashState::Live,
        trashed_at: None,
        plan: empty_plan(),
        frontmatter: Some(frontmatter_with(id, title, aliases)),
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

fn assert_resolved(index: &NoteLookupIndex, target: &str, expected_id: &str) {
    assert_eq!(
        index.resolve_target(target),
        ResolveResult::Resolved(NoteId::new(expected_id))
    );
}

fn build_lookup_index(scan: &VaultScanResult) -> NoteLookupIndex {
    build_from_scan(scan).index
}

fn duplicate_diagnostics(build: &NoteLookupIndexBuild) -> Vec<&ScanDiagnostic> {
    build
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("duplicate note id detected"))
        .collect()
}

#[test]
fn title_match_resolves() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/alpha.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAA",
            "Alpha Title",
            &[],
        )],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_resolved(&index, "Alpha Title", "01ARZ3NDEKTSV4RRFFQ69G5FAA");
}

#[test]
fn alias_resolves_when_no_title_match_exists() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/alpha.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAB",
            "Primary Title",
            &["Alias Name"],
        )],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_resolved(&index, "Alias Name", "01ARZ3NDEKTSV4RRFFQ69G5FAB");
}

#[test]
fn basename_resolves_when_no_title_or_alias_match_exists() {
    let scan = scan_with_notes(
        vec![make_note(
            "projects/my-note.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAC",
            "Different Title",
            &["Different Alias"],
        )],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_resolved(&index, "my-note", "01ARZ3NDEKTSV4RRFFQ69G5FAC");
}

#[test]
fn path_resolves_as_last_resort() {
    let scan = scan_with_notes(
        vec![make_note(
            "projects/my-note.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAD",
            "Different Title",
            &["Different Alias"],
        )],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_resolved(&index, "projects/my-note", "01ARZ3NDEKTSV4RRFFQ69G5FAD");
    assert_resolved(
        &index,
        "./projects/my-note.md",
        "01ARZ3NDEKTSV4RRFFQ69G5FAD",
    );
}

#[test]
fn path_lookup_handles_root_nested_and_dotless_paths_without_panic() {
    let scan = scan_with_notes(
        vec![
            make_note(
                "root-note.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAQ",
                "Root Note",
                &[],
            ),
            make_note(
                "notes/nested-note.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAR",
                "Nested Note",
                &[],
            ),
            make_note(
                "notes/dotless",
                "01ARZ3NDEKTSV4RRFFQ69G5FAS",
                "Dotless Note",
                &[],
            ),
        ],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_resolved(&index, "root-note", "01ARZ3NDEKTSV4RRFFQ69G5FAQ");
    assert_resolved(&index, "notes/nested-note", "01ARZ3NDEKTSV4RRFFQ69G5FAR");
    assert_resolved(&index, "notes/dotless", "01ARZ3NDEKTSV4RRFFQ69G5FAS");
}

#[test]
fn ambiguous_when_two_notes_share_same_title() {
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/a.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAE",
                "Shared Title",
                &[],
            ),
            make_note(
                "notes/b.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAF",
                "Shared Title",
                &[],
            ),
        ],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_eq!(
        index.resolve_target("Shared Title"),
        ResolveResult::Ambiguous
    );
}

#[test]
fn returns_not_found_when_no_match_exists() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/alpha.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAG",
            "Alpha Title",
            &[],
        )],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_eq!(index.resolve_target("Missing"), ResolveResult::NotFound);
}

#[test]
fn notes_without_document_are_excluded_from_index() {
    let scan = scan_with_notes(
        vec![make_note_without_document(
            "notes/alpha.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAH",
            "Alpha Title",
            &["Alias Name"],
        )],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert!(index.titles.is_empty());
    assert!(index.aliases.is_empty());
    assert!(index.basenames.is_empty());
    assert!(index.paths.is_empty());
    assert_eq!(index.resolve_target("Alpha Title"), ResolveResult::NotFound);
}

#[test]
fn duplicate_id_notes_are_excluded_from_index() {
    let duplicate_id = NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAJ");
    let scan = scan_with_notes(
        vec![make_note(
            "notes/alpha.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAJ",
            "Alpha Title",
            &["Alias Name"],
        )],
        vec![(duplicate_id, vec![RelativePath::new("notes/alpha.md")])],
    );

    let index = build_lookup_index(&scan);

    assert!(index.titles.is_empty());
    assert_eq!(index.resolve_target("Alpha Title"), ResolveResult::NotFound);
}

#[test]
fn duplicate_id_notes_detected_from_notes_are_excluded_and_reported() {
    let duplicate_id = NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAT");
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/alpha.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAT",
                "Alpha Title",
                &["Alias Name"],
            ),
            make_note(
                "notes/beta.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAT",
                "Beta Title",
                &["Other Alias"],
            ),
        ],
        Vec::new(),
    );

    let build = build_from_scan(&scan);
    let diagnostics = duplicate_diagnostics(&build);

    assert!(build.index.titles.is_empty());
    assert_eq!(
        build.index.resolve_target("Alpha Title"),
        ResolveResult::NotFound
    );
    assert_eq!(
        build.duplicate_ids,
        vec![(
            duplicate_id.clone(),
            vec![
                RelativePath::new("notes/alpha.md"),
                RelativePath::new("notes/beta.md"),
            ],
        )]
    );
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic.level == DiagnosticLevel::Error
            && diagnostic.message == format!("duplicate note id detected: {duplicate_id}")
    }));
}

#[test]
fn existing_duplicate_id_diagnostics_are_not_emitted_twice() {
    let duplicate_id = NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAV");
    let duplicate_path = RelativePath::new("notes/alpha.md");
    let scan = VaultScanResult {
        notes: vec![make_note(
            "notes/alpha.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "Alpha Title",
            &["Alias Name"],
        )],
        assets: Vec::new(),
        diagnostics: vec![ScanDiagnostic {
            level: DiagnosticLevel::Error,
            path: duplicate_path.clone(),
            message: format!("duplicate note id detected: {duplicate_id}"),
        }],
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: vec![(duplicate_id, vec![duplicate_path])],
        trashed_note_count: 0,
    };

    let build = build_from_scan(&scan);

    assert!(duplicate_diagnostics(&build).is_empty());
}

#[test]
fn matching_is_case_insensitive_for_all_lookup_levels() {
    let scan = scan_with_notes(
        vec![
            make_note(
                "notes/title-note.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAK",
                "Title Case",
                &[],
            ),
            make_note(
                "notes/alias-note.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAL",
                "Alias Owner",
                &["Alias Value"],
            ),
            make_note(
                "folder/basename-note.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAM",
                "Basename Owner",
                &[],
            ),
            make_note(
                "folder/path-note.md",
                "01ARZ3NDEKTSV4RRFFQ69G5FAN",
                "Path Owner",
                &[],
            ),
        ],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_resolved(&index, "title case", "01ARZ3NDEKTSV4RRFFQ69G5FAK");
    assert_resolved(&index, "ALIAS VALUE", "01ARZ3NDEKTSV4RRFFQ69G5FAL");
    assert_resolved(&index, "BASENAME-NOTE", "01ARZ3NDEKTSV4RRFFQ69G5FAM");
    assert_resolved(
        &index,
        "./FOLDER/PATH-NOTE.MD",
        "01ARZ3NDEKTSV4RRFFQ69G5FAN",
    );
}

#[test]
fn same_note_id_is_deduplicated_within_each_bucket() {
    let scan = scan_with_notes(
        vec![make_note(
            "notes/same-key.md",
            "01ARZ3NDEKTSV4RRFFQ69G5FAP",
            "Same Key",
            &["same key"],
        )],
        Vec::new(),
    );

    let index = build_lookup_index(&scan);

    assert_eq!(
        index.titles.get("same key"),
        Some(&vec![NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAP")])
    );
    assert_eq!(
        index.aliases.get("same key"),
        Some(&vec![NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAP")])
    );
    assert_eq!(
        index.resolve_target("Same Key"),
        ResolveResult::Resolved(NoteId::new("01ARZ3NDEKTSV4RRFFQ69G5FAP"))
    );
}
