use std::{collections::BTreeMap, fs};

mod common;

use common::{unique_test_dir, write_text};
use zk_app::{AppError, create_note};
use zk_domain::{
    config::{EffectiveConfig, FolderPolicy},
    id::{NoteId, RelativePath},
    note::NoteFrontmatter,
    write::CreateNoteRequest,
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
fn fresh_create_writes_file_returns_note_id_and_roundtrips() {
    let root = unique_test_dir("create-note-fresh");
    let vault_root = root.join("vault");
    let request = CreateNoteRequest {
        path: RelativePath::new("notes/new.md"),
        frontmatter: sample_frontmatter("01CREATE000000000000000001", "Fresh"),
        body_markdown: "# Fresh\n\nBody\n".to_string(),
    };
    let expected_path = vault_root.join("notes").join("new.md");
    let config = EffectiveConfig::default();

    let created_id =
        create_note(&vault_root, &config, request.clone()).expect("create_note should succeed");

    assert!(!created_id.as_str().is_empty());
    assert_eq!(created_id, request.frontmatter.id);
    assert!(expected_path.exists());

    let parsed = parse_note_file(&expected_path).expect("parse created note");
    let parsed_frontmatter = parsed
        .frontmatter
        .expect("created note should include frontmatter");
    assert_eq!(parsed_frontmatter.id, Some(created_id));
    assert_eq!(parsed_frontmatter.title.as_deref(), Some("Fresh"));
    assert_eq!(parsed.body_markdown, "# Fresh\n\nBody\n");
}

#[test]
fn empty_frontmatter_id_triggers_ulid_generation() {
    let root = unique_test_dir("create-note-generate-id");
    let vault_root = root.join("vault");
    let request = CreateNoteRequest {
        path: RelativePath::new("notes/generated.md"),
        frontmatter: sample_frontmatter("", "Generated Id"),
        body_markdown: "body".to_string(),
    };
    let config = EffectiveConfig::default();

    let created_id =
        create_note(&vault_root, &config, request).expect("create_note should succeed");

    assert!(!created_id.as_str().is_empty());
    assert_eq!(created_id.as_str().len(), 26);
}

#[test]
fn caller_supplied_id_is_preserved_verbatim() {
    let root = unique_test_dir("create-note-preserve-id");
    let vault_root = root.join("vault");
    let supplied_id = NoteId::from("01SUPPLIED0000000000000001");
    let request = CreateNoteRequest {
        path: RelativePath::new("notes/preserved.md"),
        frontmatter: sample_frontmatter(supplied_id.as_str(), "Preserved Id"),
        body_markdown: "body".to_string(),
    };
    let config = EffectiveConfig::default();

    let created_id =
        create_note(&vault_root, &config, request).expect("create_note should preserve id");

    assert_eq!(created_id, supplied_id);
}

#[test]
fn duplicate_path_yields_note_path_exists_error() {
    let root = unique_test_dir("create-note-duplicate-path");
    let vault_root = root.join("vault");
    let target_path = vault_root.join("notes").join("existing.md");
    write_text(
        &target_path,
        r#"---
id: 01EXISTING0000000000000001
title: Existing
tags:
  - tg8
---
Existing body
"#,
    );
    let request = CreateNoteRequest {
        path: RelativePath::new("notes/existing.md"),
        frontmatter: sample_frontmatter("01NEW0000000000000000000001", "New"),
        body_markdown: "body".to_string(),
    };
    let config = EffectiveConfig::default();

    let err = create_note(&vault_root, &config, request).expect_err("duplicate should fail");
    assert!(matches!(
        err,
        AppError::NotePathExists { path } if path == RelativePath::new("notes/existing.md")
    ));
}

#[test]
fn immutable_folder_target_yields_immutable_folder_error() {
    let root = unique_test_dir("create-note-immutable");
    let vault_root = root.join("vault");
    let request = CreateNoteRequest {
        path: RelativePath::new("raw/new.md"),
        frontmatter: sample_frontmatter("01IMMUTABLE000000000000001", "Immutable"),
        body_markdown: "body".to_string(),
    };
    let mut config = EffectiveConfig::default();
    config.folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
        },
    )]);

    let err = create_note(&vault_root, &config, request).expect_err("raw path should be blocked");
    assert!(matches!(
        err,
        AppError::ImmutableFolder { path } if path == RelativePath::new("raw/new.md")
    ));
}

#[test]
fn nested_target_path_creates_intermediate_directories() {
    let root = unique_test_dir("create-note-nested-dirs");
    let vault_root = root.join("vault");
    let nested_path = RelativePath::new("notes/daily/2026-04-17.md");
    let request = CreateNoteRequest {
        path: nested_path.clone(),
        frontmatter: sample_frontmatter("01NESTED000000000000000001", "Nested"),
        body_markdown: "body".to_string(),
    };
    let config = EffectiveConfig::default();

    let _created_id =
        create_note(&vault_root, &config, request).expect("create_note should succeed");

    let expected_file = vault_root.join(nested_path.as_str());
    let expected_parent = expected_file
        .parent()
        .expect("nested path should have parent directory");
    assert!(expected_parent.exists());
    assert!(expected_file.exists());

    let content = fs::read_to_string(expected_file).expect("read created nested note");
    assert!(content.contains("title: Nested"));
}
