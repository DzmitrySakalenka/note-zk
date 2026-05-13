// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use zk_domain::{
    JobId,
    id::{ContentHash, NoteId},
};
use zk_server::{
    ai::lint_wiki::{WikiLintFinding, WikiLintFindingKind, WikiLintSeverity},
    migration::ensure_schema,
    repo::{AiJobKind, AiJobsRepo, NoteRow, NotesRepo, WikiLintFindingsRepo, WikiLintScope},
};

fn setup_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database should open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys pragma should be enabled");
    ensure_schema(&connection).expect("schema migration should succeed");
    connection
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn note_id(value: &str) -> NoteId {
    NoteId::new(value)
}

fn note_row(note_id: &NoteId, path: &str) -> NoteRow {
    let now = parse_timestamp("2026-05-13T10:00:00Z");
    let body = format!("# {path}");

    NoteRow {
        note_id: note_id.to_string(),
        current_path: path.to_string(),
        title: path.to_string(),
        content_hash: ContentHash::from_bytes(body.as_bytes()).to_string(),
        body_markdown: body,
        frontmatter_json: format!(r#"{{"id":"{note_id}","title":"{path}"}}"#),
        created_at: now,
        updated_at: now,
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: 1,
    }
}

fn insert_note(connection: &Connection, note_id: &NoteId, path: &str) {
    NotesRepo::create(connection, &note_row(note_id, path)).expect("note should insert");
}

fn enqueue_lint_job(connection: &Connection, request_id: &str) -> JobId {
    AiJobsRepo::new(connection)
        .enqueue(
            AiJobKind::LintWiki {
                scope: WikiLintScope::Vault,
                request_id: request_id.to_string(),
            },
            parse_timestamp("2026-05-13T10:05:00Z"),
        )
        .expect("lint job should enqueue")
        .job_id
}

fn finding(
    kind: WikiLintFindingKind,
    page_id: Option<NoteId>,
    source_id: Option<NoteId>,
    message: &str,
) -> WikiLintFinding {
    WikiLintFinding {
        kind,
        page_id,
        source_id,
        message: message.to_string(),
        severity: WikiLintSeverity::Warning,
    }
}

#[test]
fn insert_many_lists_and_counts_findings_for_one_job() {
    let connection = setup_connection();
    let page_id = note_id("01LINTREPOPAGE000000001");
    let source_id = note_id("01LINTREPOSOURCE0000001");
    insert_note(&connection, &page_id, "wiki/page.md");
    insert_note(&connection, &source_id, "raw/source.md");
    let job_id = enqueue_lint_job(&connection, "req-lint-repo");
    let other_job_id = enqueue_lint_job(&connection, "req-lint-other");
    let repo = WikiLintFindingsRepo::new(&connection);

    let inserted = repo
        .insert_many(
            &job_id,
            &[
                finding(
                    WikiLintFindingKind::OrphanPage,
                    Some(page_id.clone()),
                    None,
                    "wiki page is orphaned",
                ),
                finding(
                    WikiLintFindingKind::StaleClaim,
                    Some(page_id.clone()),
                    Some(source_id.clone()),
                    "source is newer than wiki page",
                ),
            ],
            parse_timestamp("2026-05-13T10:06:00Z"),
        )
        .expect("findings should insert");
    repo.insert_many(
        &other_job_id,
        &[finding(
            WikiLintFindingKind::ImportantConceptWithoutPage,
            None,
            None,
            "concept missing",
        )],
        parse_timestamp("2026-05-13T10:07:00Z"),
    )
    .expect("other job findings should insert");

    assert_eq!(inserted, 2);
    assert_eq!(
        repo.count_for_job(&job_id)
            .expect("finding count should load"),
        2
    );
    let rows = repo
        .list_for_job(&job_id)
        .expect("findings should list for job");
    let mut messages = rows
        .iter()
        .map(|row| row.message.clone())
        .collect::<Vec<_>>();
    messages.sort();

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.job_id == job_id));
    assert!(
        rows.iter()
            .all(|row| row.severity == WikiLintSeverity::Warning)
    );
    assert!(
        rows.iter()
            .all(|row| ulid::Ulid::from_string(&row.finding_id).is_ok())
    );
    assert_eq!(
        messages,
        vec![
            "source is newer than wiki page".to_string(),
            "wiki page is orphaned".to_string(),
        ]
    );
    assert_eq!(
        repo.count_for_job(&other_job_id)
            .expect("other finding count should load"),
        1
    );
}

#[test]
fn insert_many_replaces_existing_rows_for_same_job() {
    let connection = setup_connection();
    let page_id = note_id("01LINTREPOREPLACE000001");
    insert_note(&connection, &page_id, "wiki/page.md");
    let job_id = enqueue_lint_job(&connection, "req-lint-replace");
    let repo = WikiLintFindingsRepo::new(&connection);

    repo.insert_many(
        &job_id,
        &[
            finding(
                WikiLintFindingKind::OrphanPage,
                Some(page_id.clone()),
                None,
                "old finding one",
            ),
            finding(
                WikiLintFindingKind::MissingCrossReference,
                Some(page_id.clone()),
                None,
                "old finding two",
            ),
        ],
        parse_timestamp("2026-05-13T10:06:00Z"),
    )
    .expect("old findings should insert");

    repo.insert_many(
        &job_id,
        &[finding(
            WikiLintFindingKind::OrphanPage,
            Some(page_id.clone()),
            None,
            "replacement finding",
        )],
        parse_timestamp("2026-05-13T10:07:00Z"),
    )
    .expect("replacement findings should insert");

    let rows = repo
        .list_for_job(&job_id)
        .expect("findings should list after replacement");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].message, "replacement finding");
    assert_eq!(rows[0].page_id, Some(page_id));
}

#[test]
fn list_for_job_limited_applies_deterministic_limit() {
    let connection = setup_connection();
    let job_id = enqueue_lint_job(&connection, "req-lint-limited");
    let repo = WikiLintFindingsRepo::new(&connection);

    repo.insert_many(
        &job_id,
        &[
            finding(WikiLintFindingKind::OrphanPage, None, None, "first finding"),
            finding(
                WikiLintFindingKind::MissingCrossReference,
                None,
                None,
                "second finding",
            ),
            finding(
                WikiLintFindingKind::ImportantConceptWithoutPage,
                None,
                None,
                "third finding",
            ),
        ],
        parse_timestamp("2026-05-13T10:06:00Z"),
    )
    .expect("findings should insert");

    let rows = repo
        .list_for_job_limited(&job_id, 2)
        .expect("limited findings should list");
    let expected = repo
        .list_for_job(&job_id)
        .expect("all findings should list")
        .into_iter()
        .take(2)
        .map(|row| row.finding_id)
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|row| row.finding_id.clone())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn encoded_kind_and_severity_are_lowercase_text_enums() {
    let connection = setup_connection();
    let job_id = enqueue_lint_job(&connection, "req-lint-text-enums");
    let repo = WikiLintFindingsRepo::new(&connection);

    repo.insert_many(
        &job_id,
        &[WikiLintFinding {
            kind: WikiLintFindingKind::ImportantConceptWithoutPage,
            page_id: None,
            source_id: None,
            message: "concept missing".to_string(),
            severity: WikiLintSeverity::Error,
        }],
        parse_timestamp("2026-05-13T10:06:00Z"),
    )
    .expect("finding should insert");

    let raw: (String, String) = connection
        .query_row(
            "SELECT kind, severity FROM wiki_lint_findings WHERE job_id = ?1",
            [job_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("raw finding should load");
    let rows = repo
        .list_for_job(&job_id)
        .expect("decoded finding should load");

    assert_eq!(
        raw,
        (
            "important_concept_without_page".to_string(),
            "error".to_string(),
        )
    );
    assert_eq!(
        rows[0].kind,
        WikiLintFindingKind::ImportantConceptWithoutPage
    );
    assert_eq!(rows[0].severity, WikiLintSeverity::Error);
}

#[test]
fn page_foreign_key_is_set_null_and_job_foreign_key_cascades() {
    let connection = setup_connection();
    let page_id = note_id("01LINTREPOFKPAGE000001");
    insert_note(&connection, &page_id, "wiki/page.md");
    let job_id = enqueue_lint_job(&connection, "req-lint-fk");
    let repo = WikiLintFindingsRepo::new(&connection);
    repo.insert_many(
        &job_id,
        &[finding(
            WikiLintFindingKind::OrphanPage,
            Some(page_id.clone()),
            None,
            "wiki page is orphaned",
        )],
        parse_timestamp("2026-05-13T10:06:00Z"),
    )
    .expect("finding should insert");

    NotesRepo::delete_hard(&connection, page_id.as_str()).expect("note should delete");
    let rows = repo
        .list_for_job(&job_id)
        .expect("finding should survive page delete");
    assert_eq!(rows[0].page_id, None);

    connection
        .execute("DELETE FROM ai_jobs WHERE job_id = ?1", [job_id.as_str()])
        .expect("job should delete");
    assert_eq!(
        repo.count_for_job(&job_id)
            .expect("finding count should load after cascade"),
        0
    );
}
