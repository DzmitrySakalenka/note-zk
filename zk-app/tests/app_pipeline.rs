// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::path::Path;

mod common;

use common::{open_index_connection, unique_test_dir, write_text};
use tempfile::TempDir;
use zk_app::{
    AppError, RebuildOptions, RebuildOutcome, WriteContext, bootstrap_effective_config,
    create_note, doctor_report, rebuild_index, scan_vault, status_summary, update_note,
};
use zk_domain::{
    config::CliOverrides,
    id::{DeviceId, NoteId, RelativePath},
    note::NoteFrontmatter,
    write::{CreateNoteRequest, UpdateNoteRequest},
};
use zk_index::NoteRepo;

fn sample_frontmatter(note_id: &str, title: &str) -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::new(note_id),
        title: title.to_string(),
        tags: vec!["tg8".to_string()],
        note_type: None,
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: vec![],
        extra: std::collections::BTreeMap::default(),
    }
}

fn env_for_home(home_root: &Path) -> impl Fn(&str) -> Option<String> {
    let home_text = home_root.to_string_lossy().to_string();
    move |name: &str| match name {
        "HOME" | "APPDATA" => Some(home_text.clone()),
        _ => None,
    }
}

fn device_id() -> DeviceId {
    DeviceId::new("device-app-pipeline-tests")
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the app pipeline acceptance test is clearer as one end-to-end scenario"
)]
fn full_read_pipeline() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let vault_root = temp_dir.path().join("vault");
    let home_root = temp_dir.path().join("home");

    write_text(
        &vault_root.join("a.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAA
title: Note A
tags:
  - tg8
---
# Note A

Body A
",
    );
    write_text(
        &vault_root.join("b.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAB
title: Note B
tags:
  - tg8
---
# Note B

Resolved wiki: [[Note C]]
Unresolved wiki: [[NonExistent]]
Asset: ![Diagram](./assets/diagram.png)
",
    );
    write_text(
        &vault_root.join("c.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAC
title: Note C
tags:
  - tg8
---
# Note C

Body C
",
    );
    write_text(
        &vault_root.join("sibling").join("dup-a.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAA
title: Duplicate A
tags:
  - tg8
---
# Duplicate A

Duplicate id content
",
    );
    write_text(
        &vault_root.join("invalid-frontmatter.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAD
title: Invalid Frontmatter
tags:
  - tg8
bad: [unterminated
---
Body with malformed frontmatter
",
    );
    write_text(
        &vault_root.join("99_trash").join("old.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAE
title: Old Note
tags:
  - tg8
---
# Old Note

Trashed content
",
    );
    write_text(&vault_root.join("assets").join("diagram.png"), "png-bytes");

    let env = env_for_home(&home_root);
    let config = bootstrap_effective_config(&vault_root, &CliOverrides::default(), &env)
        .expect("bootstrap effective config");

    let scan_outcome = scan_vault(&vault_root, &config).expect("scan vault should succeed");
    assert!(scan_outcome.scan_result.notes.len() >= 5);

    let mut conn = open_index_connection();
    let rebuild_outcome = rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("rebuild index should succeed");
    let rebuild_index_stats = match &rebuild_outcome {
        RebuildOutcome::ScanOnly { index_stats, .. } => index_stats,
        RebuildOutcome::Normalized { .. } => {
            panic!("default rebuild should not return normalization payload")
        }
    };
    assert_eq!(rebuild_index_stats.notes_upserted, 3);
    assert_eq!(rebuild_index_stats.assets_upserted, 1);
    assert!(rebuild_index_stats.note_links_inserted >= 1);
    assert!(rebuild_index_stats.asset_references_inserted >= 1);

    let summary =
        status_summary(&vault_root, &config, &conn).expect("status summary should succeed");
    assert_eq!(summary.total_notes, 3);
    assert_eq!(summary.total_assets, 1);
    assert!(summary.notes_with_errors >= 1);
    assert_eq!(summary.duplicate_id_count, 1);
    assert_eq!(summary.trashed_notes, 1);

    let doctor = doctor_report(&vault_root, &config, &conn).expect("doctor report should succeed");
    assert!(
        doctor
            .errors
            .iter()
            .any(|issue| issue.rule_id == "duplicate-id"),
        "expected duplicate-id in doctor error bucket"
    );
    assert!(
        doctor
            .errors
            .iter()
            .any(|issue| issue.rule_id == "invalid-frontmatter"),
        "expected invalid-frontmatter in doctor error bucket"
    );
    assert!(
        doctor
            .warnings
            .iter()
            .any(|issue| issue.rule_id == "unresolved-link"),
        "expected unresolved-link in doctor warning bucket"
    );
}

#[test]
fn write_path_roundtrip() {
    let root = unique_test_dir("app-pipeline-write-roundtrip");
    let vault_root = root.join("vault");
    let home_root = root.join("home");

    write_text(
        &vault_root.join(".zk").join("config.toml"),
        r#"
[folders."raw"]
immutable = true
"#,
    );

    let env = env_for_home(&home_root);
    let config = bootstrap_effective_config(&vault_root, &CliOverrides::default(), &env)
        .expect("bootstrap effective config");
    let mut conn = open_index_connection();

    let note_id = NoteId::new("01B6S01M2YQWZ6K2H8M7V1QY9A");
    create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut conn,
            device_id: &device_id(),
        },
        CreateNoteRequest {
            path: RelativePath::new("notes/roundtrip.md"),
            frontmatter: sample_frontmatter(note_id.as_str(), "Roundtrip"),
            body_markdown: "Before update\n".to_string(),
        },
    )
    .expect("create note should succeed");

    rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("initial rebuild should succeed");

    let before = NoteRepo::get_note(&conn, &note_id)
        .expect("lookup note after first rebuild")
        .expect("note should exist after first rebuild");

    update_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut conn,
            device_id: &device_id(),
        },
        UpdateNoteRequest {
            note_id: note_id.clone(),
            frontmatter: None,
            body_markdown: Some("After update\n".to_string()),
        },
        None,
    )
    .expect("update note should succeed");

    rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("second rebuild should succeed");
    let after = NoteRepo::get_note(&conn, &note_id)
        .expect("lookup note after second rebuild")
        .expect("note should exist after second rebuild");

    assert_ne!(
        before.content_hash, after.content_hash,
        "content hash should change after update"
    );

    let err = create_note(
        &vault_root,
        &config,
        &mut WriteContext {
            conn: &mut conn,
            device_id: &device_id(),
        },
        CreateNoteRequest {
            path: RelativePath::new("raw/new.md"),
            frontmatter: sample_frontmatter("01B6S01M2YQWZ6K2H8M7V1QY9B", "Raw"),
            body_markdown: "Blocked by immutable policy\n".to_string(),
        },
    )
    .expect_err("create note under immutable raw/ should fail");

    assert!(matches!(
        err,
        AppError::ImmutableFolder { path } if path == RelativePath::new("raw/new.md")
    ));
}
