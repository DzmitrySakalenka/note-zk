use std::fs;

mod common;

use common::{unique_test_dir, write_text};
use zk_app::{RebuildOptions, rebuild_index};
use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
};
use zk_index::{NoteRepo, open_memory};

#[test]
fn rebuild_index_orchestrates_scan_bridge_and_rebuild_idempotently() {
    let root = unique_test_dir("rebuild-index");
    let vault_root = root.join("vault");
    let source_id = NoteId::from("01ARZ3NDEKTSV4RRFFQ69G5FAV");
    let target_id = NoteId::from("01ARZ3NDEKTSV4RRFFQ69G5FAW");

    write_text(
        &vault_root.join("source.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAV
title: Source
tags:
  - tg8
---
# Source

Resolved wiki: [[Target Note]]
External link: [Example](https://example.com)
Asset link: ![Diagram](./assets/diagram.png)
",
    );
    write_text(
        &vault_root.join("target.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAW
title: Target Note
tags:
  - tg8
---
# Target Note

Body content
",
    );
    write_text(&vault_root.join("assets").join("diagram.png"), "png-bytes");

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };

    let mut conn = open_memory().expect("open in-memory SQLite");
    let first_outcome = rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("first rebuild should succeed");

    assert!(first_outcome.normalization.is_none());
    assert!(first_outcome.index_stats.notes_upserted >= 2);
    assert!(first_outcome.index_stats.assets_upserted >= 1);
    assert!(first_outcome.index_stats.note_links_inserted >= 1);

    let first_source_note = NoteRepo::get_note(&conn, &source_id)
        .expect("lookup source note after first rebuild")
        .expect("source note row should exist");
    let first_target_note = NoteRepo::get_note(&conn, &target_id)
        .expect("lookup target note after first rebuild")
        .expect("target note row should exist");
    let first_counts = read_table_counts(&conn);

    let second_outcome = rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("second rebuild should succeed");

    assert_eq!(second_outcome.index_stats, first_outcome.index_stats);
    assert_eq!(read_table_counts(&conn), first_counts);

    let second_source_note = NoteRepo::get_note(&conn, &source_id)
        .expect("lookup source note after second rebuild")
        .expect("source note row should exist after second rebuild");
    let second_target_note = NoteRepo::get_note(&conn, &target_id)
        .expect("lookup target note after second rebuild")
        .expect("target note row should exist after second rebuild");

    assert_eq!(second_source_note.note_id, first_source_note.note_id);
    assert_eq!(
        second_source_note.current_path,
        first_source_note.current_path
    );
    assert_eq!(
        second_source_note.content_hash,
        first_source_note.content_hash
    );
    assert_eq!(second_source_note.created_at, first_source_note.created_at);
    assert_eq!(second_source_note.updated_at, first_source_note.updated_at);

    assert_eq!(second_target_note.note_id, first_target_note.note_id);
    assert_eq!(
        second_target_note.current_path,
        first_target_note.current_path
    );
    assert_eq!(
        second_target_note.content_hash,
        first_target_note.content_hash
    );
    assert_eq!(second_target_note.created_at, first_target_note.created_at);
    assert_eq!(second_target_note.updated_at, first_target_note.updated_at);
}

#[test]
fn rebuild_index_write_normalized_writes_id_and_indexes_generated_note() {
    let root = unique_test_dir("rebuild-index-write-normalized");
    let vault_root = root.join("vault");
    let note_path = vault_root.join("notes").join("missing-id.md");

    write_text(
        &note_path,
        r"---
title: Missing Id
tags:
  - tg8
---
# Missing Id

Body content
",
    );

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };

    let mut conn = open_memory().expect("open in-memory SQLite");
    let outcome = rebuild_index(
        &vault_root,
        &config,
        &mut conn,
        RebuildOptions {
            write_normalized: true,
        },
    )
    .expect("rebuild with normalization write should succeed");

    let normalization = outcome
        .normalization
        .as_ref()
        .expect("normalization summary should be present");
    assert!(normalization.files_written >= 1);
    assert!(outcome.index_stats.notes_upserted >= 1);

    let normalized_contents =
        fs::read_to_string(&note_path).expect("read note after normalization write");
    let generated_id = extract_frontmatter_id(&normalized_contents)
        .expect("normalized frontmatter should contain generated id");
    assert_eq!(generated_id.len(), 26);
    assert!(generated_id.chars().all(|ch| ch.is_ascii_alphanumeric()));

    let indexed_note = NoteRepo::get_note(&conn, &NoteId::from(&generated_id))
        .expect("lookup note by generated id")
        .expect("note should be indexed under generated id");
    assert_eq!(
        indexed_note.current_path,
        RelativePath::new("notes/missing-id.md")
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableCounts {
    notes_index: usize,
    assets_index: usize,
    note_links: usize,
    external_links: usize,
    asset_references: usize,
}

fn read_table_counts(conn: &rusqlite::Connection) -> TableCounts {
    TableCounts {
        notes_index: count_rows(conn, "notes_index"),
        assets_index: count_rows(conn, "assets_index"),
        note_links: count_rows(conn, "note_links"),
        external_links: count_rows(conn, "external_links"),
        asset_references: count_rows(conn, "asset_references"),
    }
}

fn count_rows(conn: &rusqlite::Connection, table_name: &str) -> usize {
    let query = format!("SELECT COUNT(*) FROM {table_name}");
    let row_count: i64 = conn
        .query_row(&query, [], |row| row.get(0))
        .expect("count rows from table");
    usize::try_from(row_count).expect("row count should fit into usize")
}

fn extract_frontmatter_id(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()? != "---" {
        return None;
    }

    for line in lines {
        if line == "---" {
            break;
        }
        if let Some(id_value) = line.strip_prefix("id: ") {
            return Some(id_value.trim().to_string());
        }
    }

    None
}
