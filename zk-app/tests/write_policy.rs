// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::collections::BTreeMap;

use zk_app::{AppError, ensure_writable, is_path_derived, is_path_immutable};
use zk_domain::{config::FolderPolicy, id::RelativePath};

#[test]
fn empty_folder_map_is_writable() {
    let folders = BTreeMap::new();
    let path = RelativePath::new("notes/entry.md");

    assert!(!is_path_immutable(&path, &folders));
    assert!(!is_path_derived(&path, &folders));
    assert!(ensure_writable(&path, &folders).is_ok());
}

#[test]
fn immutable_folder_blocks_write_to_child_path() {
    let folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
            derived: None,
        },
    )]);
    let path = RelativePath::new("raw/foo.md");

    assert!(is_path_immutable(&path, &folders));
    let err = ensure_writable(&path, &folders).expect_err("raw/* should be blocked");
    assert!(matches!(
        err,
        AppError::ImmutableFolder { path: blocked } if blocked == path
    ));
}

#[test]
fn nested_folder_policy_overrides_less_specific_parent() {
    let folders = BTreeMap::from([
        (
            "raw".to_string(),
            FolderPolicy {
                immutable: Some(true),
                derived: None,
            },
        ),
        (
            "raw/source".to_string(),
            FolderPolicy {
                immutable: Some(false),
                derived: None,
            },
        ),
    ]);
    let path = RelativePath::new("raw/source/bar.md");

    assert!(!is_path_immutable(&path, &folders));
    assert!(ensure_writable(&path, &folders).is_ok());
}

#[test]
fn sibling_path_outside_immutable_tree_is_writable() {
    let folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
            derived: None,
        },
    )]);
    let path = RelativePath::new("notes/foo.md");

    assert!(!is_path_immutable(&path, &folders));
    assert!(ensure_writable(&path, &folders).is_ok());
}

#[test]
fn immutable_none_is_treated_as_writable() {
    let folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: None,
            derived: None,
        },
    )]);
    let path = RelativePath::new("raw/foo.md");

    assert!(!is_path_immutable(&path, &folders));
    assert!(ensure_writable(&path, &folders).is_ok());
}

#[test]
fn normalized_windows_and_dot_slash_paths_match_immutable_prefix() {
    let folders = BTreeMap::from([(
        "raw".to_string(),
        FolderPolicy {
            immutable: Some(true),
            derived: None,
        },
    )]);
    let windows_path = RelativePath::new("raw\\foo.md");
    let dot_slash_path = RelativePath::new("./raw/foo.md");

    assert!(is_path_immutable(&windows_path, &folders));
    assert!(is_path_immutable(&dot_slash_path, &folders));
    assert!(ensure_writable(&windows_path, &folders).is_err());
    assert!(ensure_writable(&dot_slash_path, &folders).is_err());
}

#[test]
fn derived_folder_matches_child_path() {
    let folders = BTreeMap::from([(
        "wiki".to_string(),
        FolderPolicy {
            immutable: None,
            derived: Some(true),
        },
    )]);
    let path = RelativePath::new("wiki/index.md");

    assert!(is_path_derived(&path, &folders));
    assert!(!is_path_immutable(&path, &folders));
    assert!(ensure_writable(&path, &folders).is_ok());
}

#[test]
fn nested_derived_folder_policy_overrides_less_specific_parent() {
    let folders = BTreeMap::from([
        (
            "wiki".to_string(),
            FolderPolicy {
                immutable: None,
                derived: Some(true),
            },
        ),
        (
            "wiki/manual".to_string(),
            FolderPolicy {
                immutable: None,
                derived: Some(false),
            },
        ),
    ]);
    let path = RelativePath::new("wiki/manual/page.md");

    assert!(!is_path_derived(&path, &folders));
}
