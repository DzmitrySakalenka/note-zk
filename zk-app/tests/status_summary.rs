use std::collections::BTreeMap;

mod common;

use common::{unique_test_dir, write_text};
use zk_app::{RebuildOptions, rebuild_index, status_summary};
use zk_domain::config::{EffectiveConfig, ValidationConfig};
use zk_index::open_memory;

#[test]
fn status_summary_reports_vault_and_index_snapshot_counts() {
    let root = unique_test_dir("status-summary");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("notes").join("clean-a.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAA
title: Clean A
tags:
  - tg8
---
# Clean A

Unresolved wiki: [[Missing Note]]
Asset: ![Diagram](../assets/diagram.png)
"#,
    );
    write_text(
        &vault_root.join("notes").join("clean-b.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAB
title: Clean B
tags:
  - tg8
---
# Clean B

Body content
"#,
    );
    write_text(
        &vault_root.join("notes").join("missing-title.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAC
tags:
  - tg8
---
Body without an H1 heading.
"#,
    );
    write_text(
        &vault_root.join("99_trash").join("old.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAD
title: Old Note
tags:
  - tg8
---
# Old Note

Trashed content
"#,
    );
    write_text(&vault_root.join("assets").join("diagram.png"), "png-bytes");

    let mut config = EffectiveConfig::default();
    config.vault_path = vault_root.clone();
    config.allowed_note_extensions = vec!["md".to_string()];
    config.validation = ValidationConfig {
        disabled_rules: Vec::new(),
        severity_overrides: BTreeMap::from([(
            "TitleDerivedFromFilename".to_string(),
            "error".to_string(),
        )]),
    };

    let mut conn = open_memory().expect("open in-memory SQLite");
    rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("initial rebuild should succeed");

    let summary =
        status_summary(&vault_root, &config, &conn).expect("status summary should succeed");

    assert_eq!(summary.total_notes, 3);
    assert_eq!(summary.total_assets, 1);
    assert!(summary.notes_with_errors >= 1);
    assert_eq!(summary.unresolved_link_count, 1);
    assert_eq!(summary.duplicate_id_count, 0);
    assert_eq!(summary.trash_count, 1);
}
