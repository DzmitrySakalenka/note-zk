// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, path::Path};

mod common;

use common::{unique_test_dir, write_text};
use rusqlite::types::ValueRef;
use tempfile::{Builder, TempDir};
use zk_app::{AppError, RebuildOptions, rebuild_index};
use zk_domain::config::EffectiveConfig;
use zk_index::open_or_create;

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexSnapshot {
    notes_index: Vec<Vec<String>>,
    assets_index: Vec<Vec<String>>,
    note_links: Vec<Vec<String>>,
    external_links: Vec<Vec<String>>,
    asset_references: Vec<Vec<String>>,
    delete_records: Vec<Vec<String>>,
    note_tags: Vec<Vec<String>>,
    note_aliases: Vec<Vec<String>>,
    schema_version: Vec<Vec<String>>,
}

#[test]
fn rebuild_index_rolls_back_index_changes_when_reconciliation_fails() {
    let test_root = temp_test_dir();
    let vault_root = test_root.path().join("vault");
    let db_path = test_root.path().join("index.sqlite");

    write_text(
        &vault_root.join("notes").join("source.md"),
        r"---
id: 01REBUILDROLLBACKSOURCE0001
title: Source
tags:
  - rollback
---
# Source

Resolved wiki: [[Target]]
External link: [Example](https://example.com)
Asset link: ![Diagram](../assets/diagram.png)
",
    );
    write_text(
        &vault_root.join("notes").join("target.md"),
        r"---
id: 01REBUILDROLLBACKTARGET0001
title: Target
---
# Target
",
    );
    write_text(&vault_root.join("assets").join("diagram.png"), "png-bytes");
    write_text(
        &vault_root.join("99_trash").join("archived.md"),
        r"---
id: 01REBUILDROLLBACKTRASH00001
title: Archived
---
# Archived
",
    );

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };

    let mut conn = open_or_create(&db_path).expect("open file-backed SQLite");
    rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("initial rebuild should succeed");
    let before_failure = snapshot_index(&db_path);

    write_text(
        &vault_root.join("notes").join("source.md"),
        r"---
id: 01REBUILDROLLBACKSOURCE0001
title: Source Updated
tags:
  - rollback-updated
---
# Source Updated

Resolved wiki: [[Target]]
",
    );
    write_text(
        &vault_root.join("notes").join("added.md"),
        r"---
id: 01REBUILDROLLBACKADDED00001
title: Added
tags:
  - added
---
# Added

Body
",
    );

    // Deny `delete_records` reads so the rebuild fails inside reconciliation,
    // after the transactional index rebuild has already started writing.
    conn.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| {
        match ctx.action {
            rusqlite::hooks::AuthAction::Read {
                table_name: "delete_records",
                ..
            } => rusqlite::hooks::Authorization::Deny,
            _ => rusqlite::hooks::Authorization::Allow,
        }
    }))
    .expect("install authorizer");

    let err = rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect_err("reconciliation failure should abort rebuild");
    assert!(matches!(
        &err,
        AppError::IndexContext { context, .. }
            if context.contains("during rebuild reconciliation")
    ));
    assert!(err.to_string().contains("get delete record"));

    drop(conn);

    let after_failure = snapshot_index(&db_path);
    assert_eq!(after_failure, before_failure);
}

fn temp_test_dir() -> TempDir {
    let parent = unique_test_dir("rebuild-rollback-parent");
    fs::create_dir_all(&parent).expect("create rebuild rollback parent");
    Builder::new()
        .prefix("rebuild-rollback-")
        .tempdir_in(parent)
        .expect("create tempdir in rebuild rollback parent")
}

fn snapshot_index(db_path: &Path) -> IndexSnapshot {
    let conn = open_or_create(db_path).expect("open snapshot database");

    IndexSnapshot {
        notes_index: query_rows(
            &conn,
            "SELECT note_id, current_path, title, content_hash, frontmatter_json, created_at, updated_at, trashed_at, deleted_at, is_trashed, is_deleted, last_editor_device_id, server_revision FROM notes_index ORDER BY note_id",
        ),
        assets_index: query_rows(
            &conn,
            "SELECT asset_id, current_path, content_hash, mime_type, size_bytes, created_at, updated_at, trashed_at, deleted_at, is_trashed, is_deleted, server_revision FROM assets_index ORDER BY asset_id",
        ),
        note_links: query_rows(
            &conn,
            "SELECT source_note_id, raw_target, target_note_id, link_text FROM note_links ORDER BY source_note_id, raw_target, target_note_id, link_text",
        ),
        external_links: query_rows(
            &conn,
            "SELECT source_note_id, url, link_text FROM external_links ORDER BY source_note_id, url, link_text",
        ),
        asset_references: query_rows(
            &conn,
            "SELECT source_note_id, raw_path, target_asset_id FROM asset_references ORDER BY source_note_id, raw_path, target_asset_id",
        ),
        delete_records: query_rows(
            &conn,
            "SELECT item_id, item_type, original_path, trashed_at, retention_until, deleted_at, delete_revision FROM delete_records ORDER BY item_type, item_id",
        ),
        note_tags: query_rows(
            &conn,
            "SELECT note_id, tag FROM note_tags ORDER BY note_id, tag",
        ),
        note_aliases: query_rows(
            &conn,
            "SELECT note_id, alias FROM note_aliases ORDER BY note_id, alias",
        ),
        schema_version: query_rows(&conn, "SELECT version FROM schema_version ORDER BY version"),
    }
}

fn query_rows(conn: &rusqlite::Connection, sql: &str) -> Vec<Vec<String>> {
    let mut statement = conn.prepare(sql).expect("prepare snapshot query");
    let column_count = statement.column_count();
    let rows = statement
        .query_map([], |row| {
            let mut values = Vec::with_capacity(column_count);
            for index in 0..column_count {
                let value = row.get_ref(index)?;
                values.push(render_value(value));
            }
            Ok(values)
        })
        .expect("execute snapshot query");

    rows.map(|row| row.expect("snapshot row should decode"))
        .collect()
}

fn render_value(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "<NULL>".to_string(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        ValueRef::Blob(value) => format!("{value:?}"),
    }
}
