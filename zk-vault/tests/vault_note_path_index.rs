// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{fs, path::Path};

use tempfile::TempDir;
use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::DiagnosticLevel,
};
use zk_vault::build_note_path_index;

fn write_text_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn base_config(vault_root: &Path) -> EffectiveConfig {
    EffectiveConfig {
        vault_path: vault_root.to_path_buf(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    }
}

#[test]
fn build_note_path_index_indexes_only_notes_with_ids() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("notes/with-id.md"),
        concat!(
            "---\n",
            "id: 01INDEXWITHID0000000000001\n",
            "title: With Id\n",
            "tags:\n",
            "  - tg8\n",
            "---\n",
            "Body\n",
        ),
    );
    write_text_file(
        &vault_root.join("notes/no-id.md"),
        concat!(
            "---\n",
            "title: No Id\n",
            "tags:\n",
            "  - tg8\n",
            "---\n",
            "Body\n",
        ),
    );

    let build = build_note_path_index(vault_root, &config).expect("index build should succeed");

    assert_eq!(
        build.index.get(&NoteId::new("01INDEXWITHID0000000000001")),
        Some(&RelativePath::new("notes/with-id.md"))
    );
    assert!(build.diagnostics.is_empty());
}

#[test]
fn build_note_path_index_skips_invalid_yaml_and_records_diagnostic() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("notes/valid.md"),
        concat!(
            "---\n",
            "id: 01INDEXVALID00000000000001\n",
            "title: Valid\n",
            "tags:\n",
            "  - tg8\n",
            "---\n",
            "Body\n",
        ),
    );
    write_text_file(
        &vault_root.join("notes/invalid.md"),
        concat!("---\n", "id: [broken\n", "---\n", "Body\n",),
    );

    let build = build_note_path_index(vault_root, &config).expect("index build should succeed");

    assert_eq!(
        build.index.get(&NoteId::new("01INDEXVALID00000000000001")),
        Some(&RelativePath::new("notes/valid.md"))
    );
    assert!(
        build.diagnostics.iter().any(|diagnostic| {
            diagnostic.level == DiagnosticLevel::Error
                && diagnostic.path == RelativePath::new("notes/invalid.md")
                && diagnostic
                    .message
                    .contains("failed to parse note while building note path index")
        }),
        "invalid yaml should be surfaced as a build diagnostic"
    );
}

#[test]
fn build_note_path_index_excludes_duplicate_ids_from_resolution() {
    let tempdir = TempDir::new().expect("tempdir should be created");
    let vault_root = tempdir.path();
    let config = base_config(vault_root);

    write_text_file(
        &vault_root.join("notes/duplicate-a.md"),
        concat!(
            "---\n",
            "id: 01INDEXDUPLICATE0000000001\n",
            "title: Duplicate A\n",
            "tags:\n",
            "  - tg8\n",
            "---\n",
            "Body\n",
        ),
    );
    write_text_file(
        &vault_root.join("notes/duplicate-b.md"),
        concat!(
            "---\n",
            "id: 01INDEXDUPLICATE0000000001\n",
            "title: Duplicate B\n",
            "tags:\n",
            "  - tg8\n",
            "---\n",
            "Body\n",
        ),
    );
    write_text_file(
        &vault_root.join("notes/unique.md"),
        concat!(
            "---\n",
            "id: 01INDEXUNIQUE000000000001\n",
            "title: Unique\n",
            "tags:\n",
            "  - tg8\n",
            "---\n",
            "Body\n",
        ),
    );

    let build = build_note_path_index(vault_root, &config).expect("index build should succeed");

    assert_eq!(
        build.index.get(&NoteId::new("01INDEXDUPLICATE0000000001")),
        None,
        "duplicate ids must not resolve to an arbitrary path"
    );
    assert_eq!(
        build.index.get(&NoteId::new("01INDEXUNIQUE000000000001")),
        Some(&RelativePath::new("notes/unique.md"))
    );

    let duplicate_diagnostics: Vec<_> = build
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("duplicate note id detected"))
        .collect();
    assert_eq!(duplicate_diagnostics.len(), 2);
    assert!(
        duplicate_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path == RelativePath::new("notes/duplicate-a.md"))
    );
    assert!(
        duplicate_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path == RelativePath::new("notes/duplicate-b.md"))
    );
}
