// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use common::{unique_test_dir, write_text};
use zk_app::{
    EffectiveConfig, RebuildOptions, list_pages_using_source, list_sources_for_note, rebuild_index,
    scan_vault,
};
use zk_domain::id::NoteId;
use zk_index::open_memory;

fn first_source_id() -> NoteId {
    NoteId::new("01WIKIWORKFLOWSOURCE0000001")
}

fn second_source_id() -> NoteId {
    NoteId::new("01WIKIWORKFLOWSOURCE0000002")
}

fn target_page_id() -> NoteId {
    NoteId::new("01WIKIWORKFLOWPAGE00000001")
}

#[test]
fn provenance_round_trip_survives_scan_and_rebuild() {
    let root = unique_test_dir("wiki-workflows-provenance");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("raw").join("first.md"),
        r"---
id: 01WIKIWORKFLOWSOURCE0000001
title: First Source
tags:
  - raw
---
# First Source

Raw material one.
",
    );
    write_text(
        &vault_root.join("raw").join("second.md"),
        r"---
id: 01WIKIWORKFLOWSOURCE0000002
title: Second Source
tags:
  - raw
---
# Second Source

Raw material two.
",
    );
    write_text(
        &vault_root.join("wiki").join("combined.md"),
        r"---
id: 01WIKIWORKFLOWPAGE00000001
title: Combined Wiki Page
tags:
  - wiki
type: wiki-synthesis
sources:
  - 01WIKIWORKFLOWSOURCE0000001
  - 01WIKIWORKFLOWSOURCE0000002
---
# Combined Wiki Page

Synthesized from both sources.
",
    );

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };

    let scan = scan_vault(&vault_root, &config).expect("scan_vault should succeed");
    assert_eq!(scan.scan_result.notes.len(), 3);
    let target_scan = scan
        .scan_result
        .notes
        .iter()
        .find(|note| note.relative_path.as_str() == "wiki/combined.md")
        .expect("target wiki page should be scanned");
    assert_eq!(
        target_scan
            .document
            .as_ref()
            .expect("target should parse")
            .frontmatter
            .sources,
        vec![first_source_id(), second_source_id()]
    );

    let mut connection = open_memory().expect("open in-memory SQLite");
    rebuild_index(
        &vault_root,
        &config,
        &mut connection,
        RebuildOptions::default(),
    )
    .expect("rebuild should succeed");

    assert_eq!(
        list_sources_for_note(&connection, &target_page_id()).expect("sources should query"),
        vec![first_source_id(), second_source_id()]
    );
    assert_eq!(
        list_pages_using_source(&connection, &first_source_id()).expect("pages should query"),
        vec![target_page_id()]
    );
}
