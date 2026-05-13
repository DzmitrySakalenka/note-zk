// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::BTreeMap;

mod common;

use common::{open_index_connection, unique_test_dir};
use zk_app::{EffectiveConfig, WriteContext, create_note, update_note};
use zk_domain::{
    id::{DeviceId, NoteId, RelativePath},
    note::NoteFrontmatter,
    write::{CreateNoteRequest, UpdateNoteRequest},
};
use zk_index::SourcesRepo;

fn page_id() -> NoteId {
    NoteId::new("01WRITESOURCES0000000000001")
}

fn first_source_id() -> NoteId {
    NoteId::new("01WRITESOURCE0000000000001")
}

fn second_source_id() -> NoteId {
    NoteId::new("01WRITESOURCE0000000000002")
}

fn device_id() -> DeviceId {
    DeviceId::new("device-write-sources-tests")
}

fn frontmatter(title: &str, sources: Vec<NoteId>) -> NoteFrontmatter {
    NoteFrontmatter {
        id: page_id(),
        title: title.to_string(),
        tags: vec!["wiki".to_string()],
        note_type: Some("wiki-entity".to_string()),
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources,
        extra: BTreeMap::new(),
    }
}

#[test]
fn create_note_persists_frontmatter_sources_to_index() {
    let root = unique_test_dir("write-sources-create");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();
    let sources = vec![first_source_id(), second_source_id()];

    create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id(),
        },
        CreateNoteRequest {
            path: RelativePath::new("wiki/page.md"),
            frontmatter: frontmatter("Wiki Page", sources.clone()),
            body_markdown: "Page body\n".to_string(),
        },
    )
    .expect("create note should succeed");

    let actual =
        SourcesRepo::list_sources_for_note(&connection, &page_id()).expect("query should succeed");
    assert_eq!(actual, sources);
}

#[test]
fn update_note_replaces_frontmatter_sources_in_index() {
    let root = unique_test_dir("write-sources-update");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let mut connection = open_index_connection();

    create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id(),
        },
        CreateNoteRequest {
            path: RelativePath::new("wiki/page.md"),
            frontmatter: frontmatter("Wiki Page", vec![first_source_id()]),
            body_markdown: "Page body\n".to_string(),
        },
    )
    .expect("create note should succeed");
    update_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut connection,
            device_id: &device_id(),
        },
        UpdateNoteRequest {
            note_id: page_id(),
            frontmatter: Some(frontmatter("Wiki Page Updated", vec![second_source_id()])),
            body_markdown: Some("Updated body\n".to_string()),
        },
        None,
    )
    .expect("update note should succeed");

    let actual =
        SourcesRepo::list_sources_for_note(&connection, &page_id()).expect("query should succeed");
    assert_eq!(actual, vec![second_source_id()]);
}
