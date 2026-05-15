#![allow(unused_crate_dependencies)]

use std::collections::HashSet;

use zk_domain::id::{NoteId, RelativePath};
use zk_index::ListedNote;
use zk_tui::screens::notes::{TreeAnchor, TreeRow, build_tree_rows, resolve_tree_selection};

fn note(id: &str, title: &str, path: &str) -> ListedNote {
    ListedNote {
        note_id: NoteId::new(id),
        title: title.to_string(),
        current_path: RelativePath::new(path),
        is_trashed: false,
    }
}

fn expanded(paths: &[&str]) -> HashSet<String> {
    paths.iter().map(|p| (*p).to_string()).collect()
}

#[test]
fn build_tree_rows_empty() {
    assert!(build_tree_rows(&[], &HashSet::new()).is_empty());
}

#[test]
fn build_tree_rows_root_notes_sorted_by_title() {
    let notes = vec![
        note("01N1", "Bravo", "bravo.md"),
        note("01N2", "Alpha", "alpha.md"),
    ];
    let rows = build_tree_rows(&notes, &HashSet::new());
    assert_eq!(rows.len(), 2);
    match &rows[0] {
        TreeRow::Note { note_index, depth } => {
            assert_eq!(*depth, 0);
            assert_eq!(notes[*note_index].title, "Alpha");
        }
        TreeRow::Dir { .. } => panic!("expected note row first"),
    }
    match &rows[1] {
        TreeRow::Note { note_index, .. } => {
            assert_eq!(notes[*note_index].title, "Bravo");
        }
        TreeRow::Dir { .. } => panic!("expected second note row"),
    }
}

#[test]
fn build_tree_rows_collapsed_by_default() {
    let notes = vec![
        note("01N1", "Inner A", "projects/inner_a.md"),
        note("01N2", "Inner B", "projects/deep/inner_b.md"),
        note("01N3", "Other", "other/other.md"),
        note("01N4", "Root", "root.md"),
    ];
    let rows = build_tree_rows(&notes, &HashSet::new());
    // Two top-level dirs ("other", "projects" alphabetically) and one
    // root-level note. Nested files stay hidden until their dir is expanded.
    assert_eq!(rows.len(), 3);
    assert!(matches!(
        &rows[0],
        TreeRow::Dir { path, depth: 0, expanded: false } if path == "other"
    ));
    assert!(matches!(
        &rows[1],
        TreeRow::Dir { path, depth: 0, expanded: false } if path == "projects"
    ));
    assert!(matches!(&rows[2], TreeRow::Note { depth: 0, .. }));
}

#[test]
fn build_tree_rows_expand_one_level() {
    let notes = vec![
        note("01N1", "Inner A", "projects/inner_a.md"),
        note("01N2", "Inner B", "projects/inner_b.md"),
        note("01N3", "Deep", "projects/deep/deep.md"),
    ];
    let rows = build_tree_rows(&notes, &expanded(&["projects"]));
    // Expected order at depth 1: the deeper child dir "projects/deep"
    // (still collapsed) first, then notes "Inner A" and "Inner B".
    assert_eq!(rows.len(), 4);
    assert!(matches!(
        &rows[0],
        TreeRow::Dir { path, depth: 0, expanded: true } if path == "projects"
    ));
    assert!(matches!(
        &rows[1],
        TreeRow::Dir { path, depth: 1, expanded: false } if path == "projects/deep"
    ));
    assert!(matches!(&rows[2], TreeRow::Note { depth: 1, .. }));
    assert!(matches!(&rows[3], TreeRow::Note { depth: 1, .. }));
}

#[test]
fn build_tree_rows_expand_nested() {
    let notes = vec![note("01N1", "Deep", "a/b/deep.md")];
    let rows = build_tree_rows(&notes, &expanded(&["a", "a/b"]));
    assert_eq!(rows.len(), 3);
    assert!(matches!(
        &rows[0],
        TreeRow::Dir { path, depth: 0, expanded: true } if path == "a"
    ));
    assert!(matches!(
        &rows[1],
        TreeRow::Dir { path, depth: 1, expanded: true } if path == "a/b"
    ));
    assert!(matches!(&rows[2], TreeRow::Note { depth: 2, .. }));
}

#[test]
fn build_tree_rows_dirs_before_notes_at_same_level() {
    // A directory "m" and a root note titled "m" sort together by name;
    // dirs must precede notes regardless of how their names compare.
    let notes = vec![
        note("01N1", "m", "m.md"),
        note("01N2", "Inside", "m_dir/inside.md"),
    ];
    let rows = build_tree_rows(&notes, &HashSet::new());
    assert_eq!(rows.len(), 2);
    assert!(matches!(
        &rows[0],
        TreeRow::Dir { path, .. } if path == "m_dir"
    ));
    assert!(matches!(&rows[1], TreeRow::Note { .. }));
}

#[test]
fn resolve_tree_selection_prefers_note_anchor() {
    let notes = vec![
        note("01N1", "Inner A", "projects/inner_a.md"),
        note("01N2", "Inner B", "projects/inner_b.md"),
    ];
    let rows = build_tree_rows(&notes, &expanded(&["projects"]));
    let anchor = TreeAnchor::Note(notes[1].note_id.clone());
    let index = resolve_tree_selection(&rows, Some(&anchor), &notes);
    match &rows[index] {
        TreeRow::Note { note_index, .. } => {
            assert_eq!(notes[*note_index].note_id, notes[1].note_id);
        }
        TreeRow::Dir { .. } => panic!("anchor should land on a note row"),
    }
}

#[test]
fn resolve_tree_selection_prefers_dir_anchor() {
    let notes = vec![
        note("01N1", "Inner", "projects/inner.md"),
        note("01N2", "Root", "root.md"),
    ];
    let rows = build_tree_rows(&notes, &HashSet::new());
    let anchor = TreeAnchor::Dir("projects".to_string());
    let index = resolve_tree_selection(&rows, Some(&anchor), &notes);
    assert!(matches!(
        &rows[index],
        TreeRow::Dir { path, .. } if path == "projects"
    ));
}

#[test]
fn resolve_tree_selection_falls_back_to_zero_when_anchor_missing() {
    let notes = vec![note("01N1", "Root", "root.md")];
    let rows = build_tree_rows(&notes, &HashSet::new());
    let missing_anchor = TreeAnchor::Note(NoteId::new("01NOTFOUND"));
    assert_eq!(
        resolve_tree_selection(&rows, Some(&missing_anchor), &notes),
        0,
    );
}
