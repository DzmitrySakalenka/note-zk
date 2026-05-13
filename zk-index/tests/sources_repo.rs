// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::id::NoteId;
use zk_index::{SourcesRepo, ensure_schema, open_memory};

fn wiki_page_id() -> NoteId {
    NoteId::new("01WIKIPAGE0000000000000001")
}

fn second_wiki_page_id() -> NoteId {
    NoteId::new("01WIKIPAGE0000000000000002")
}

fn first_source_id() -> NoteId {
    NoteId::new("01SOURCE000000000000000001")
}

fn second_source_id() -> NoteId {
    NoteId::new("01SOURCE000000000000000002")
}

#[test]
fn replace_for_note_with_empty_sources_clears_existing_rows() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let page_id = wiki_page_id();

    SourcesRepo::replace_for_note(&connection, &page_id, &[first_source_id()])
        .expect("initial source replacement should succeed");
    SourcesRepo::replace_for_note(&connection, &page_id, &[])
        .expect("empty source replacement should succeed");

    let sources =
        SourcesRepo::list_sources_for_note(&connection, &page_id).expect("query should succeed");
    assert!(sources.is_empty());
}

#[test]
fn replace_for_note_round_trips_single_source() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let page_id = wiki_page_id();
    let source_id = first_source_id();

    SourcesRepo::replace_for_note(&connection, &page_id, std::slice::from_ref(&source_id))
        .expect("source replacement should succeed");

    let sources =
        SourcesRepo::list_sources_for_note(&connection, &page_id).expect("query should succeed");
    let pages = SourcesRepo::list_pages_using_source(&connection, &source_id)
        .expect("reverse query should succeed");
    assert_eq!(sources, vec![source_id]);
    assert_eq!(pages, vec![page_id]);
}

#[test]
fn replace_for_note_round_trips_multiple_sources_in_input_order() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let page_id = wiki_page_id();
    let sources = vec![first_source_id(), second_source_id()];

    SourcesRepo::replace_for_note(&connection, &page_id, &sources)
        .expect("source replacement should succeed");

    let actual =
        SourcesRepo::list_sources_for_note(&connection, &page_id).expect("query should succeed");
    assert_eq!(actual, sources);
}

#[test]
fn replace_for_note_is_idempotent_for_same_sources() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let page_id = wiki_page_id();
    let sources = vec![first_source_id(), second_source_id()];

    SourcesRepo::replace_for_note(&connection, &page_id, &sources)
        .expect("first source replacement should succeed");
    SourcesRepo::replace_for_note(&connection, &page_id, &sources)
        .expect("second source replacement should succeed");

    let actual =
        SourcesRepo::list_sources_for_note(&connection, &page_id).expect("query should succeed");
    assert_eq!(actual, sources);
}

#[test]
fn list_pages_using_source_returns_all_pages_in_insert_order() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source_id = first_source_id();
    let first_page_id = wiki_page_id();
    let second_page_id = second_wiki_page_id();

    SourcesRepo::replace_for_note(
        &connection,
        &first_page_id,
        std::slice::from_ref(&source_id),
    )
    .expect("first page source replacement should succeed");
    SourcesRepo::replace_for_note(
        &connection,
        &second_page_id,
        std::slice::from_ref(&source_id),
    )
    .expect("second page source replacement should succeed");

    let pages = SourcesRepo::list_pages_using_source(&connection, &source_id)
        .expect("reverse query should succeed");
    assert_eq!(pages, vec![first_page_id, second_page_id]);
}

#[test]
fn delete_for_note_removes_only_that_note_sources() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source_id = first_source_id();
    let deleted_page_id = wiki_page_id();
    let retained_page_id = second_wiki_page_id();

    SourcesRepo::replace_for_note(
        &connection,
        &deleted_page_id,
        std::slice::from_ref(&source_id),
    )
    .expect("deleted page source replacement should succeed");
    SourcesRepo::replace_for_note(
        &connection,
        &retained_page_id,
        std::slice::from_ref(&source_id),
    )
    .expect("retained page source replacement should succeed");

    SourcesRepo::delete_for_note(&connection, &deleted_page_id)
        .expect("delete for note should succeed");

    assert!(
        SourcesRepo::list_sources_for_note(&connection, &deleted_page_id)
            .expect("forward query should succeed")
            .is_empty()
    );
    let pages = SourcesRepo::list_pages_using_source(&connection, &source_id)
        .expect("reverse query should succeed");
    assert_eq!(pages, vec![retained_page_id]);
}
