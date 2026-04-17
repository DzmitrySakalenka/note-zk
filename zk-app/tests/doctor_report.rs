mod common;

use common::{unique_test_dir, write_text};
use zk_app::doctor_report;
use zk_domain::config::EffectiveConfig;
use zk_index::open_memory;

#[test]
fn doctor_report_groups_expected_errors_and_warnings() {
    let root = unique_test_dir("doctor-report");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("dup-a.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEE
title: Duplicate A
tags:
  - tg8
---
# Duplicate A

Body content
"#,
    );
    write_text(
        &vault_root.join("dup-b.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEE
title: Duplicate B
tags:
  - tg8
---
# Duplicate B

Body content
"#,
    );
    write_text(
        &vault_root.join("issues.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEF
title: Issues
tags:
  - tg8
---
# Issues

Unresolved wiki: [[Missing Note]]
Missing asset: ![Missing](./assets/missing.png)
"#,
    );
    write_text(
        &vault_root.join("empty.md"),
        r#"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEG
title: Empty
tags:
  - tg8
---
"#,
    );

    let mut config = EffectiveConfig::default();
    config.vault_path = vault_root.clone();
    config.allowed_note_extensions = vec!["md".to_string()];

    let conn = open_memory().expect("open in-memory SQLite");
    let report = doctor_report(&vault_root, &config, &conn).expect("doctor report should succeed");

    assert!(
        report
            .errors
            .iter()
            .any(|issue| issue.rule_id == "duplicate-id"),
        "duplicate-id should be present in error bucket"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|issue| issue.rule_id == "unresolved-link"),
        "unresolved-link should be present in warning bucket"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|issue| issue.rule_id == "unresolved-asset"),
        "unresolved-asset should be present in warning bucket"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|issue| issue.rule_id == "empty-body"),
        "empty-body should be present in warning bucket"
    );
}
