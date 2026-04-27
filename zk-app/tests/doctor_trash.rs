// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::fs;

mod common;

use common::{unique_test_dir, write_text};
use zk_app::doctor_report;
use zk_domain::{
    config::EffectiveConfig,
    id::{ContentHash, NoteId, RelativePath},
    note::NoteFrontmatter,
    sync::ItemType,
    sync_state::DeleteRecord,
};
use zk_index::{DeleteRecordRepo, NoteRepo, ensure_schema, open_memory};

fn sample_frontmatter(note_id: &str, title: &str) -> String {
    serde_json::to_string(&NoteFrontmatter {
        id: NoteId::new(note_id),
        title: title.to_string(),
        tags: vec!["tg10".to_string()],
        note_type: None,
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: std::collections::BTreeMap::default(),
    })
    .expect("frontmatter JSON should serialize")
}

fn config_for(vault_root: &std::path::Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

#[test]
fn doctor_report_warns_when_trashed_file_has_no_delete_record() {
    let root = unique_test_dir("doctor-trash-without-record");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("vault root should exist");
    write_text(
        &vault_root.join("99_trash").join("orphan.md"),
        r"---
id: 01TG10DOCTORTRASH0000000001
title: Orphan Trash
tags:
  - tg10
---
# Orphan Trash

Body
",
    );

    let config = config_for(&vault_root);
    let conn = open_memory().expect("open in-memory SQLite");
    ensure_schema(&conn).expect("schema should exist");

    let report = doctor_report(&vault_root, &config, &conn).expect("doctor report should succeed");
    let issue = report
        .warnings
        .iter()
        .find(|issue| issue.rule_id == "trash-file-without-record")
        .expect("trash-file-without-record warning should be present");

    assert_eq!(issue.path, Some(RelativePath::new("99_trash/orphan.md")));
    assert_eq!(
        issue.item_id.as_deref(),
        Some("01TG10DOCTORTRASH0000000001")
    );
    assert_eq!(issue.item_type, Some(ItemType::Note));
    assert!(
        issue.message.contains("delete record"),
        "warning should mention the missing delete record"
    );
}

#[test]
fn doctor_report_warns_when_delete_record_has_no_matching_index_row() {
    let root = unique_test_dir("doctor-orphan-delete-record");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("vault root should exist");

    let config = config_for(&vault_root);
    let conn = open_memory().expect("open in-memory SQLite");
    ensure_schema(&conn).expect("schema should exist");
    DeleteRecordRepo::insert_delete_record(
        &conn,
        &DeleteRecord {
            target: zk_domain::DeleteRecordTarget::Note(NoteId::new(
                "01TG10ORPHANDELETE0000000001",
            )),
            original_path: RelativePath::new("notes/missing.md"),
            trashed_at: "2026-04-20T10:00:00Z".to_string(),
            retention_until: "2099-01-01T00:00:00Z".to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("orphan delete record should insert");

    let report = doctor_report(&vault_root, &config, &conn).expect("doctor report should succeed");
    let issue = report
        .warnings
        .iter()
        .find(|issue| issue.rule_id == "orphan-delete-record")
        .expect("orphan-delete-record warning should be present");

    assert_eq!(issue.path, Some(RelativePath::new("notes/missing.md")));
    assert_eq!(
        issue.item_id.as_deref(),
        Some("01TG10ORPHANDELETE0000000001")
    );
    assert_eq!(issue.item_type, Some(ItemType::Note));
}

#[test]
fn doctor_report_reports_expired_retention_with_purge_hint() {
    let root = unique_test_dir("doctor-retention-expired");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("vault root should exist");

    let config = config_for(&vault_root);
    let conn = open_memory().expect("open in-memory SQLite");
    ensure_schema(&conn).expect("schema should exist");
    NoteRepo::upsert_note(
        &conn,
        &NoteId::new("01TG10RETENTIONDUE0000000001"),
        &RelativePath::new("99_trash/2026-04-01/expired.md"),
        "Expired Trash",
        &ContentHash::new("hash-expired-trash"),
        &sample_frontmatter("01TG10RETENTIONDUE0000000001", "Expired Trash"),
        "2026-04-01T10:00:00Z",
        "2026-04-01T10:00:00Z",
        Some("2026-04-01T10:00:00Z"),
        true,
    )
    .expect("trashed note should upsert");
    DeleteRecordRepo::insert_delete_record(
        &conn,
        &DeleteRecord {
            target: zk_domain::DeleteRecordTarget::Note(NoteId::new(
                "01TG10RETENTIONDUE0000000001",
            )),
            original_path: RelativePath::new("notes/expired.md"),
            trashed_at: "2026-04-01T10:00:00Z".to_string(),
            retention_until: "2026-04-02T00:00:00Z".to_string(),
            deleted_at: None,
            delete_revision: None,
        },
    )
    .expect("expired delete record should insert");

    let report = doctor_report(&vault_root, &config, &conn).expect("doctor report should succeed");
    let issue = report
        .infos
        .iter()
        .find(|issue| issue.rule_id == "retention-expired-not-purged")
        .expect("retention-expired-not-purged info should be present");

    assert_eq!(
        issue.path,
        Some(RelativePath::new("99_trash/2026-04-01/expired.md"))
    );
    assert_eq!(
        issue.item_id.as_deref(),
        Some("01TG10RETENTIONDUE0000000001")
    );
    assert_eq!(issue.item_type, Some(ItemType::Note));
    assert!(
        issue.message.contains("zk-cli purge --expired --yes"),
        "info should include the purge hint"
    );
}
