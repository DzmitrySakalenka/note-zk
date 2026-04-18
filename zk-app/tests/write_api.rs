use std::{collections::BTreeMap, fs};

mod common;

use common::{unique_test_dir, write_text};
use zk_app::{AppError, BatchWriteKind, batch_write, create_note, update_note};
use zk_domain::{
    config::{EffectiveConfig, FolderPolicy},
    id::{NoteId, RelativePath},
    note::NoteFrontmatter,
    write::{BatchWriteRequest, CreateNoteRequest, UpdateNoteRequest},
};
use zk_vault::parse_note_file;

fn sample_frontmatter(id: &str, title: &str) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::from(id),
        title: title.to_string(),
        tags: vec!["tg8".to_string()],
        note_type: None,
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: vec![],
        extra: Default::default(),
    }
}

#[test]
fn roundtrip_create_then_update_changes_body() {
    let root = unique_test_dir("write-api-roundtrip");
    let vault_root = root.join("vault");
    let note_id = NoteId::from("01WRITEAPI0000000000000001");
    let create_request = CreateNoteRequest {
        path: RelativePath::new("notes/roundtrip.md"),
        frontmatter: sample_frontmatter(note_id.as_str(), "Roundtrip"),
        body_markdown: "Original body\n".to_string(),
    };
    let config = EffectiveConfig::default();

    create_note(&vault_root, &config, create_request).expect("create should succeed");
    update_note(
        &vault_root,
        &config,
        UpdateNoteRequest {
            note_id: note_id.clone(),
            frontmatter: None,
            body_markdown: Some("Updated body\n".to_string()),
        },
        None,
    )
    .expect("update should succeed");

    let parsed = parse_note_file(&vault_root.join("notes").join("roundtrip.md"))
        .expect("parse updated roundtrip note");
    assert_eq!(parsed.body_markdown, "Updated body\n");
}

#[test]
fn update_note_missing_id_returns_note_not_found() {
    let root = unique_test_dir("write-api-update-missing");
    let vault_root = root.join("vault");
    write_text(
        &vault_root.join("notes").join("existing.md"),
        r#"---
id: 01EXISTING0000000000000001
title: Existing
tags:
  - tg8
---
Existing body
"#,
    );
    let config = EffectiveConfig::default();

    let err = update_note(
        &vault_root,
        &config,
        UpdateNoteRequest {
            note_id: NoteId::from("01MISSING00000000000000001"),
            frontmatter: None,
            body_markdown: Some("body\n".to_string()),
        },
        None,
    )
    .expect_err("missing note update should fail");

    assert!(matches!(
        err,
        AppError::NoteNotFound { note_id } if note_id.as_str() == "01MISSING00000000000000001"
    ));
}

#[test]
fn batch_write_two_creates_and_one_update_reports_expected_counts() {
    let root = unique_test_dir("write-api-batch-counts");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let existing_id = NoteId::from("01EXISTINGWRITEAPI0000000001");
    create_note(
        &vault_root,
        &config,
        CreateNoteRequest {
            path: RelativePath::new("notes/existing.md"),
            frontmatter: sample_frontmatter(existing_id.as_str(), "Existing"),
            body_markdown: "Before update\n".to_string(),
        },
    )
    .expect("seed existing note");

    let create_a_id = NoteId::from("01BATCHCREATEA0000000000001");
    let create_b_id = NoteId::from("01BATCHCREATEB0000000000001");
    let outcome = batch_write(
        &vault_root,
        &config,
        BatchWriteRequest {
            creates: vec![
                CreateNoteRequest {
                    path: RelativePath::new("notes/a.md"),
                    frontmatter: sample_frontmatter(create_a_id.as_str(), "A"),
                    body_markdown: "A body\n".to_string(),
                },
                CreateNoteRequest {
                    path: RelativePath::new("notes/b.md"),
                    frontmatter: sample_frontmatter(create_b_id.as_str(), "B"),
                    body_markdown: "B body\n".to_string(),
                },
            ],
            updates: vec![UpdateNoteRequest {
                note_id: existing_id.clone(),
                frontmatter: None,
                body_markdown: Some("After update\n".to_string()),
            }],
        },
    )
    .expect("batch write should succeed");

    assert_eq!(outcome.created, vec![create_a_id, create_b_id]);
    assert_eq!(outcome.updated, vec![existing_id]);
    assert!(outcome.failures.is_empty());

    let updated = parse_note_file(&vault_root.join("notes").join("existing.md"))
        .expect("parse updated existing note");
    assert_eq!(updated.body_markdown, "After update\n");
}

#[test]
fn batch_write_immutable_middle_create_records_failure_and_keeps_earlier_success() {
    let root = unique_test_dir("write-api-batch-immutable");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("create vault root directory");
    let mut config = EffectiveConfig::default();
    config.folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);

    let first_id = NoteId::from("01BATCHOK000000000000000001");
    let blocked_id = NoteId::from("01BATCHBLOCKED000000000001");
    let outcome = batch_write(
        &vault_root,
        &config,
        BatchWriteRequest {
            creates: vec![
                CreateNoteRequest {
                    path: RelativePath::new("notes/ok.md"),
                    frontmatter: sample_frontmatter(first_id.as_str(), "OK"),
                    body_markdown: "ok body\n".to_string(),
                },
                CreateNoteRequest {
                    path: RelativePath::new("raw/blocked.md"),
                    frontmatter: sample_frontmatter(blocked_id.as_str(), "Blocked"),
                    body_markdown: "blocked body\n".to_string(),
                },
            ],
            updates: vec![],
        },
    )
    .expect("batch write should return outcome");

    assert_eq!(outcome.created, vec![first_id]);
    assert!(outcome.updated.is_empty());
    assert_eq!(outcome.failures.len(), 1);
    let failure = &outcome.failures[0];
    assert_eq!(failure.request_index, 1);
    assert_eq!(failure.kind, BatchWriteKind::Create);
    assert_eq!(failure.note_id, Some(blocked_id));
    assert!(failure.error_message.contains("folder is immutable"));
    assert!(vault_root.join("notes").join("ok.md").exists());
    assert!(!vault_root.join("raw").join("blocked.md").exists());
}

#[test]
fn update_note_mismatched_frontmatter_id_returns_note_id_mismatch() {
    let root = unique_test_dir("write-api-note-id-mismatch");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let original_id = NoteId::from("01WRITEAPIMISMATCH000000001");

    create_note(
        &vault_root,
        &config,
        CreateNoteRequest {
            path: RelativePath::new("notes/mismatch.md"),
            frontmatter: sample_frontmatter(original_id.as_str(), "Mismatch"),
            body_markdown: "before\n".to_string(),
        },
    )
    .expect("create note for mismatch test");

    let conflicting_id = NoteId::from("01WRITEAPIMISMATCH000000002");
    let err = update_note(
        &vault_root,
        &config,
        UpdateNoteRequest {
            note_id: original_id.clone(),
            frontmatter: Some(sample_frontmatter(conflicting_id.as_str(), "Conflict")),
            body_markdown: None,
        },
        None,
    )
    .expect_err("mismatched ids should fail");

    assert!(matches!(
        err,
        AppError::NoteIdMismatch {
            request_note_id,
            frontmatter_note_id
        } if request_note_id == original_id && frontmatter_note_id == conflicting_id
    ));

    let persisted = fs::read_to_string(vault_root.join("notes").join("mismatch.md"))
        .expect("read note after mismatch rejection");
    assert!(persisted.contains("title: Mismatch"));
}

#[test]
fn batch_write_builds_note_path_index_once_for_many_updates() {
    let root = unique_test_dir("write-api-batch-single-index");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();

    for note_index in 0..50 {
        let note_id_text = format!("batch-single-index-{note_index:02}");
        create_note(
            &vault_root,
            &config,
            CreateNoteRequest {
                path: RelativePath::new(&format!("notes/note-{note_index:02}.md")),
                frontmatter: sample_frontmatter(&note_id_text, &format!("Note {note_index:02}")),
                body_markdown: format!("Before {note_index:02}\n"),
            },
        )
        .expect("seed note for batch update test");
    }

    let updates: Vec<_> = (0..20)
        .map(|note_index| UpdateNoteRequest {
            note_id: NoteId::from(&format!("batch-single-index-{note_index:02}")),
            frontmatter: None,
            body_markdown: Some(format!("After {note_index:02}\n")),
        })
        .collect();

    let outcome = batch_write(
        &vault_root,
        &config,
        BatchWriteRequest {
            creates: vec![],
            updates,
        },
    )
    .expect("batch write should succeed");

    assert!(outcome.created.is_empty());
    assert_eq!(outcome.updated.len(), 20);
    assert!(outcome.failures.is_empty());
    #[cfg(debug_assertions)]
    assert_eq!(outcome.note_path_index_builds, 1);

    for note_index in 0..20 {
        let parsed = parse_note_file(
            &vault_root
                .join("notes")
                .join(format!("note-{note_index:02}.md")),
        )
        .expect("parse updated note after batch");
        assert_eq!(parsed.body_markdown, format!("After {note_index:02}\n"));
    }
}

#[test]
fn batch_write_can_update_note_created_earlier_in_same_batch() {
    let root = unique_test_dir("write-api-batch-create-then-update");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let note_id = NoteId::from("01BATCHCREATEUPDATE00000001");
    fs::create_dir_all(&vault_root).expect("create vault root for batch test");

    let outcome = batch_write(
        &vault_root,
        &config,
        BatchWriteRequest {
            creates: vec![CreateNoteRequest {
                path: RelativePath::new("notes/new.md"),
                frontmatter: sample_frontmatter(note_id.as_str(), "Created Then Updated"),
                body_markdown: "Created body\n".to_string(),
            }],
            updates: vec![UpdateNoteRequest {
                note_id: note_id.clone(),
                frontmatter: None,
                body_markdown: Some("Updated in batch\n".to_string()),
            }],
        },
    )
    .expect("batch write should succeed");

    assert_eq!(outcome.created, vec![note_id.clone()]);
    assert_eq!(outcome.updated, vec![note_id.clone()]);
    assert!(outcome.failures.is_empty());

    let parsed =
        parse_note_file(&vault_root.join("notes").join("new.md")).expect("parse created note");
    assert_eq!(parsed.body_markdown, "Updated in batch\n");
}

#[test]
fn update_note_duplicate_id_returns_note_not_found() {
    let root = unique_test_dir("write-api-duplicate-id");
    let vault_root = root.join("vault");
    let config = EffectiveConfig::default();
    let duplicate_id = NoteId::from("01WRITEAPIDUPLICATE0000001");

    create_note(
        &vault_root,
        &config,
        CreateNoteRequest {
            path: RelativePath::new("notes/first.md"),
            frontmatter: sample_frontmatter(duplicate_id.as_str(), "First"),
            body_markdown: "first\n".to_string(),
        },
    )
    .expect("create first duplicate note");
    create_note(
        &vault_root,
        &config,
        CreateNoteRequest {
            path: RelativePath::new("notes/second.md"),
            frontmatter: sample_frontmatter(duplicate_id.as_str(), "Second"),
            body_markdown: "second\n".to_string(),
        },
    )
    .expect("create second duplicate note");

    let err = update_note(
        &vault_root,
        &config,
        UpdateNoteRequest {
            note_id: duplicate_id.clone(),
            frontmatter: None,
            body_markdown: Some("should not write\n".to_string()),
        },
        None,
    )
    .expect_err("duplicate id should not resolve to an arbitrary note");

    assert!(matches!(
        err,
        AppError::NoteNotFound { note_id } if note_id == duplicate_id
    ));
}
