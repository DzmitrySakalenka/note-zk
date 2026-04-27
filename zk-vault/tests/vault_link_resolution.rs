// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fmt::Write as _, fs, path::Path};

use insta::assert_debug_snapshot;
use tempfile::TempDir;
use zk_domain::{config::EffectiveConfig, derived::ParsedNoteIndex, id::NoteId};
use zk_vault::{resolve_vault_links, scan_vault};

const SOURCE_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S01";
const TITLE_TARGET_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S02";
const ALIAS_TARGET_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S03";
const BASENAME_TARGET_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S04";
const PATH_TARGET_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S05";
const AMBIGUOUS_ONE_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S06";
const AMBIGUOUS_TWO_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S07";
const PRIORITY_TITLE_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S08";
const PRIORITY_ALIAS_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S09";
const PRIORITY_BASENAME_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G6S0A";

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("fixture file should be written");
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

fn note_content(id: &str, title: &str, aliases: &[&str], body: &str) -> String {
    let mut content = format!("---\nid: {id}\ntitle: {title}\ntags:\n  - rust\n");

    if !aliases.is_empty() {
        content.push_str("aliases:\n");
        for alias in aliases {
            let _ = writeln!(content, "  - {alias}");
        }
    }

    content.push_str("---\n");
    content.push_str(body);
    if !body.ends_with('\n') {
        content.push('\n');
    }

    content
}

fn resolve_fixture_vault(vault_root: &Path, config: &EffectiveConfig) -> Vec<ParsedNoteIndex> {
    let scan_result = scan_vault(vault_root, config).expect("fixture scan should succeed");
    resolve_vault_links(&scan_result, config)
}

fn find_index_by_title<'a>(indexes: &'a [ParsedNoteIndex], title: &str) -> &'a ParsedNoteIndex {
    indexes
        .iter()
        .find(|index| index.title == title)
        .expect("expected parsed note index should exist")
}

#[test]
fn wiki_link_resolves_to_target_note_id() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(SOURCE_ID, "Source", &[], "See [[Note B Title]].\n"),
    );
    write_text_file(
        &vault_root.join("10-target.md"),
        &note_content(TITLE_TARGET_ID, "Note B Title", &[], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(
        outbound_link.target_note_id,
        Some(NoteId::new(TITLE_TARGET_ID))
    );
    assert_eq!(outbound_link.raw_target, "Note B Title");
    assert!(!outbound_link.is_ambiguous);
}

#[test]
fn title_match_wins_over_alias_and_basename_matches() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(SOURCE_ID, "Source", &[], "See [[Priority Target]].\n"),
    );
    write_text_file(
        &vault_root.join("10-title.md"),
        &note_content(PRIORITY_TITLE_ID, "Priority Target", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("11-alias.md"),
        &note_content(
            PRIORITY_ALIAS_ID,
            "Alias Holder",
            &["Priority Target"],
            "Body\n",
        ),
    );
    write_text_file(
        &vault_root.join("Priority Target.md"),
        &note_content(PRIORITY_BASENAME_ID, "Basename Holder", &[], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(
        outbound_link.target_note_id,
        Some(NoteId::new(PRIORITY_TITLE_ID))
    );
    assert!(!outbound_link.is_ambiguous);
}

#[test]
fn alias_match_resolves_when_no_title_matches() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(SOURCE_ID, "Source", &[], "See [[Alias Match]].\n"),
    );
    write_text_file(
        &vault_root.join("10-target.md"),
        &note_content(ALIAS_TARGET_ID, "Alias Holder", &["Alias Match"], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(
        outbound_link.target_note_id,
        Some(NoteId::new(ALIAS_TARGET_ID))
    );
    assert!(!outbound_link.is_ambiguous);
}

#[test]
fn basename_match_resolves_when_title_and_alias_do_not_match() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(SOURCE_ID, "Source", &[], "See [[basename-match]].\n"),
    );
    write_text_file(
        &vault_root.join("refs/basename-match.md"),
        &note_content(BASENAME_TARGET_ID, "Basename Holder", &[], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(
        outbound_link.target_note_id,
        Some(NoteId::new(BASENAME_TARGET_ID))
    );
    assert_eq!(outbound_link.raw_target, "basename-match");
    assert!(!outbound_link.is_ambiguous);
}

#[test]
fn path_match_resolves_as_last_resort() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(SOURCE_ID, "Source", &[], "See [[refs/path-match]].\n"),
    );
    write_text_file(
        &vault_root.join("refs/path-match.md"),
        &note_content(PATH_TARGET_ID, "Path Holder", &[], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(
        outbound_link.target_note_id,
        Some(NoteId::new(PATH_TARGET_ID))
    );
    assert_eq!(outbound_link.raw_target, "refs/path-match");
    assert!(!outbound_link.is_ambiguous);
}

#[test]
fn ambiguous_wiki_link_sets_ambiguous_flag() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(SOURCE_ID, "Source", &[], "See [[Ambiguous Title]].\n"),
    );
    write_text_file(
        &vault_root.join("10-first.md"),
        &note_content(AMBIGUOUS_ONE_ID, "Ambiguous Title", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("11-second.md"),
        &note_content(AMBIGUOUS_TWO_ID, "Ambiguous Title", &[], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(outbound_link.target_note_id, None);
    assert!(outbound_link.is_ambiguous);
}

#[test]
fn unresolved_wiki_link_sets_none_without_ambiguity() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(SOURCE_ID, "Source", &[], "See [[Missing Target]].\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(outbound_link.target_note_id, None);
    assert!(!outbound_link.is_ambiguous);
}

#[test]
fn external_markdown_link_produces_external_link() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(
            SOURCE_ID,
            "Source",
            &[],
            "Read [Docs](https://example.com/docs).\n",
        ),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_external_links
        .first()
        .expect("source note should contain one outbound external link");

    assert_eq!(outbound_link.url, "https://example.com/docs");
    assert_eq!(outbound_link.link_text.as_deref(), Some("Docs"));
}

#[test]
fn image_asset_reference_produces_asset_reference() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(
            SOURCE_ID,
            "Source",
            &[],
            "![diagram](./assets/diagram.png)\n",
        ),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let asset_reference = source
        .asset_references
        .first()
        .expect("source note should contain one asset reference");

    assert_eq!(asset_reference.raw_path, "./assets/diagram.png");
    assert_eq!(asset_reference.target_asset_id, None);
}

#[test]
fn note_without_frontmatter_is_excluded_from_output_when_insertion_is_disabled() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.allow_insert_frontmatter = false;

    write_text_file(
        &vault_root.join("00-valid.md"),
        &note_content(TITLE_TARGET_ID, "Valid Note", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("10-no-frontmatter.md"),
        "# Missing Frontmatter\nBody\n",
    );

    let resolved = resolve_fixture_vault(vault_root, &config);

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].note_id, NoteId::new(TITLE_TARGET_ID));
}

#[test]
fn wiki_alias_preserves_display_text() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(
            SOURCE_ID,
            "Source",
            &[],
            "See [[Target Note|Display Text]].\n",
        ),
    );
    write_text_file(
        &vault_root.join("10-target.md"),
        &note_content(TITLE_TARGET_ID, "Target Note", &[], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");
    let outbound_link = source
        .outbound_note_links
        .first()
        .expect("source note should contain one outbound note link");

    assert_eq!(outbound_link.link_text.as_deref(), Some("Display Text"));
    assert_eq!(
        outbound_link.target_note_id,
        Some(NoteId::new(TITLE_TARGET_ID))
    );
}

#[test]
fn links_inside_fenced_code_blocks_are_ignored() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(
            SOURCE_ID,
            "Source",
            &[],
            "```md\n[[Target Note]]\n[Docs](https://example.com/docs)\n![diagram](./assets/diagram.png)\n```\n",
        ),
    );
    write_text_file(
        &vault_root.join("10-target.md"),
        &note_content(TITLE_TARGET_ID, "Target Note", &[], "Body\n"),
    );

    let resolved = resolve_fixture_vault(vault_root, &config);
    let source = find_index_by_title(&resolved, "Source");

    assert!(source.outbound_note_links.is_empty());
    assert!(source.outbound_external_links.is_empty());
    assert!(source.asset_references.is_empty());
}

#[test]
fn empty_vault_returns_empty_output() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    let resolved = resolve_fixture_vault(vault_root, &config);

    assert!(resolved.is_empty());
}

#[test]
fn mixed_vault_link_resolution_snapshot() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let mut config = base_config(vault_root);
    config.allow_insert_frontmatter = false;

    write_text_file(
        &vault_root.join("00-source.md"),
        &note_content(
            SOURCE_ID,
            "Source Note",
            &[],
            concat!(
                "See [[Title Match]].\n",
                "See [[Alias Match]].\n",
                "See [[basename-match]].\n",
                "See [[refs/path-match]].\n",
                "See [[Ambiguous Title]].\n",
                "See [[Missing Target]].\n",
                "See [[Alias Display Target|Display Text]].\n",
                "Read [Docs](https://example.com/docs).\n",
                "![diagram](./assets/diagram.png)\n",
                "```md\n",
                "[[Ignored Inside Fence]]\n",
                "[Ignored](https://ignored.example)\n",
                "![ignored](./assets/ignored.png)\n",
                "```\n",
            ),
        ),
    );
    write_text_file(
        &vault_root.join("10-title-match.md"),
        &note_content(TITLE_TARGET_ID, "Title Match", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("11-alias-holder.md"),
        &note_content(
            ALIAS_TARGET_ID,
            "Alias Holder",
            &["Alias Match", "Alias Display Target"],
            "Body\n",
        ),
    );
    write_text_file(
        &vault_root.join("12-ambiguous-one.md"),
        &note_content(AMBIGUOUS_ONE_ID, "Ambiguous Title", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("13-ambiguous-two.md"),
        &note_content(AMBIGUOUS_TWO_ID, "Ambiguous Title", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("refs/basename-match.md"),
        &note_content(BASENAME_TARGET_ID, "Basename Holder", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("refs/path-match.md"),
        &note_content(PATH_TARGET_ID, "Path Holder", &[], "Body\n"),
    );
    write_text_file(
        &vault_root.join("99-no-frontmatter.md"),
        "# Missing Frontmatter\nBody\n",
    );

    let resolved = resolve_fixture_vault(vault_root, &config);

    assert_debug_snapshot!("mixed_vault_link_resolution", resolved);
}
