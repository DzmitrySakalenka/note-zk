// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use common::{unique_test_dir, write_text};
use zk_app::{
    EffectiveConfig, RebuildOptions, list_pages_using_source, list_sources_for_note, rebuild_index,
};
use zk_domain::id::NoteId;
use zk_index::open_memory;

fn source_id() -> NoteId {
    NoteId::new("01WIKIPROVSOURCE0000000001")
}

fn first_wiki_page_id() -> NoteId {
    NoteId::new("01WIKIPROVPAGE00000000001")
}

fn second_wiki_page_id() -> NoteId {
    NoteId::new("01WIKIPROVPAGE00000000002")
}

#[test]
fn wiki_provenance_helpers_read_sources_after_rebuild() {
    let root = unique_test_dir("wiki-provenance");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("sources").join("shared.md"),
        r"---
id: 01WIKIPROVSOURCE0000000001
title: Shared Source
---
# Shared Source

Raw material
",
    );
    write_text(
        &vault_root.join("wiki").join("alpha.md"),
        r"---
id: 01WIKIPROVPAGE00000000001
title: Alpha Wiki Page
type: wiki-entity
sources:
  - 01WIKIPROVSOURCE0000000001
---
# Alpha Wiki Page

Synthesized from shared source
",
    );
    write_text(
        &vault_root.join("wiki").join("beta.md"),
        r"---
id: 01WIKIPROVPAGE00000000002
title: Beta Wiki Page
type: wiki-synthesis
sources:
  - 01WIKIPROVSOURCE0000000001
---
# Beta Wiki Page

Also synthesized from shared source
",
    );

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };
    let mut connection = open_memory().expect("open in-memory SQLite");

    rebuild_index(
        &vault_root,
        &config,
        &mut connection,
        RebuildOptions::default(),
    )
    .expect("rebuild should succeed");

    assert_eq!(
        list_sources_for_note(&connection, &first_wiki_page_id()).expect("query should succeed"),
        vec![source_id()]
    );
    assert_eq!(
        list_sources_for_note(&connection, &second_wiki_page_id()).expect("query should succeed"),
        vec![source_id()]
    );

    let pages =
        list_pages_using_source(&connection, &source_id()).expect("reverse query should succeed");
    assert_eq!(pages.len(), 2);
    assert!(pages.contains(&first_wiki_page_id()));
    assert!(pages.contains(&second_wiki_page_id()));
}
