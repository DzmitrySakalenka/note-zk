use zk_domain::{
    derived::{ExternalLink, ResolvedNoteLink},
    id::NoteId,
};
use zk_index::{LinkRepo, ensure_schema, open_memory};

fn source_note_id() -> NoteId {
    NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMP")
}

fn second_source_note_id() -> NoteId {
    NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMQ")
}

fn target_note_id() -> NoteId {
    NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMR")
}

fn secondary_target_note_id() -> NoteId {
    NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMS")
}

fn sample_note_links(source_note_id: &NoteId) -> Vec<ResolvedNoteLink> {
    vec![
        ResolvedNoteLink {
            source_note_id: source_note_id.clone(),
            raw_target: "Knowledge Graph Basics".to_string(),
            target_note_id: Some(target_note_id()),
            link_text: Some("KG Basics".to_string()),
            is_ambiguous: true,
        },
        ResolvedNoteLink {
            source_note_id: source_note_id.clone(),
            raw_target: "SQLite Indexing".to_string(),
            target_note_id: Some(secondary_target_note_id()),
            link_text: None,
            is_ambiguous: false,
        },
    ]
}

fn sample_external_links(source_note_id: &NoteId) -> Vec<ExternalLink> {
    vec![
        ExternalLink {
            source_note_id: source_note_id.clone(),
            url: "https://example.com".to_string(),
            link_text: Some("Example".to_string()),
        },
        ExternalLink {
            source_note_id: source_note_id.clone(),
            url: "https://rust-lang.org".to_string(),
            link_text: None,
        },
    ]
}

#[test]
fn replace_note_links_and_get_note_links_round_trip() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source = source_note_id();
    let links = sample_note_links(&source);

    LinkRepo::replace_note_links(&connection, &source, &links)
        .expect("note links should be replaced");

    let actual = LinkRepo::get_note_links(&connection, &source).expect("query should succeed");
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0].source_note_id, source);
    assert_eq!(actual[0].raw_target, "Knowledge Graph Basics");
    assert_eq!(actual[0].target_note_id, Some(target_note_id()));
    assert_eq!(actual[0].link_text.as_deref(), Some("KG Basics"));
    assert!(!actual[0].is_ambiguous);
    assert_eq!(actual[1].raw_target, "SQLite Indexing");
    assert_eq!(actual[1].target_note_id, Some(secondary_target_note_id()));
}

#[test]
fn replace_external_links_and_get_external_links_round_trip() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source = source_note_id();
    let links = sample_external_links(&source);

    LinkRepo::replace_external_links(&connection, &source, &links)
        .expect("external links should be replaced");

    let actual = LinkRepo::get_external_links(&connection, &source).expect("query should succeed");
    assert_eq!(actual, links);
}

#[test]
fn replace_note_links_removes_old_rows() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source = source_note_id();
    LinkRepo::replace_note_links(&connection, &source, &sample_note_links(&source))
        .expect("first note-link replacement should succeed");

    let replacement = vec![ResolvedNoteLink {
        source_note_id: source.clone(),
        raw_target: "Fresh Target".to_string(),
        target_note_id: Some(secondary_target_note_id()),
        link_text: Some("Fresh".to_string()),
        is_ambiguous: false,
    }];
    LinkRepo::replace_note_links(&connection, &source, &replacement)
        .expect("second note-link replacement should succeed");

    let actual = LinkRepo::get_note_links(&connection, &source).expect("query should succeed");
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].raw_target, "Fresh Target");
    assert_eq!(actual[0].target_note_id, Some(secondary_target_note_id()));
}

#[test]
fn backlinks_query_returns_links_from_multiple_sources_to_shared_target() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");

    let first_source = source_note_id();
    let second_source = second_source_note_id();
    LinkRepo::replace_note_links(
        &connection,
        &first_source,
        &[ResolvedNoteLink {
            source_note_id: first_source.clone(),
            raw_target: "Shared Target".to_string(),
            target_note_id: Some(target_note_id()),
            link_text: None,
            is_ambiguous: false,
        }],
    )
    .expect("first source links should be replaced");
    LinkRepo::replace_note_links(
        &connection,
        &second_source,
        &[ResolvedNoteLink {
            source_note_id: second_source.clone(),
            raw_target: "Shared Target".to_string(),
            target_note_id: Some(target_note_id()),
            link_text: Some("Display".to_string()),
            is_ambiguous: false,
        }],
    )
    .expect("second source links should be replaced");

    let backlinks =
        LinkRepo::get_backlinks(&connection, &target_note_id()).expect("query should succeed");
    assert_eq!(backlinks.len(), 2);
    assert_eq!(backlinks[0].source_note_id, first_source);
    assert_eq!(backlinks[1].source_note_id, second_source);
}

#[test]
fn unresolved_note_link_round_trips_with_none_target_note_id() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let source = source_note_id();

    LinkRepo::replace_note_links(
        &connection,
        &source,
        &[ResolvedNoteLink {
            source_note_id: source.clone(),
            raw_target: "Missing Note".to_string(),
            target_note_id: None,
            link_text: Some("Missing".to_string()),
            is_ambiguous: true,
        }],
    )
    .expect("unresolved note link should be stored");

    let links = LinkRepo::get_note_links(&connection, &source).expect("query should succeed");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target_note_id, None);
    assert_eq!(links[0].link_text.as_deref(), Some("Missing"));
    assert!(!links[0].is_ambiguous);
}

#[test]
fn replace_note_links_works_inside_outer_transaction() {
    let connection = open_memory().expect("in-memory database should open");
    ensure_schema(&connection).expect("schema should be ready");
    let transaction = connection
        .unchecked_transaction()
        .expect("outer transaction should begin");
    let source = source_note_id();

    LinkRepo::replace_note_links(&transaction, &source, &sample_note_links(&source))
        .expect("note-link replacement inside outer transaction should succeed");
    transaction
        .commit()
        .expect("outer transaction should commit");

    let actual = LinkRepo::get_note_links(&connection, &source).expect("query should succeed");
    assert_eq!(actual.len(), 2);
}
