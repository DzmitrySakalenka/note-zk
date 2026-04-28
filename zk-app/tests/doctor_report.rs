// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use common::{unique_test_dir, write_text};
use zk_app::doctor_report;
use zk_domain::config::EffectiveConfig;
use zk_index::{ensure_schema, open_memory};

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the report grouping acceptance test is clearer as one end-to-end scenario"
)]
fn doctor_report_groups_expected_errors_and_warnings() {
    let root = unique_test_dir("doctor-report");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("dup-a.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEE
title: Duplicate A
tags:
  - tg8
---
# Duplicate A

Body content
",
    );
    write_text(
        &vault_root.join("dup-b.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEE
title: Duplicate B
tags:
  - tg8
---
# Duplicate B

Body content
",
    );
    write_text(
        &vault_root.join("issues.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEF
title: Issues
tags:
  - tg8
---
# Issues

Unresolved wiki: [[Missing Note]]
Missing asset: ![Missing](./assets/missing.png)
",
    );
    write_text(
        &vault_root.join("empty.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FEG
title: Empty
tags:
  - tg8
---
",
    );
    write_text(
        &vault_root.join("invalid.md"),
        "---\ninvalid: [yaml\n---\nBody\n",
    );

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };

    let conn = open_memory().expect("open in-memory SQLite");
    ensure_schema(&conn).expect("schema should exist");
    let report = doctor_report(&vault_root, &config, &conn).expect("doctor report should succeed");

    assert!(
        report
            .errors
            .iter()
            .any(|issue| issue.rule_id == "duplicate-id"),
        "duplicate-id should be present in error bucket"
    );
    assert_eq!(
        report
            .errors
            .iter()
            .filter(|issue| issue.rule_id == "duplicate-id")
            .count(),
        2,
        "duplicate-id should be reported once per duplicate path"
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
            .any(|issue| issue.rule_id == "broken-asset-ref"),
        "broken-asset-ref should be present in warning bucket"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|issue| issue.rule_id == "empty-body"),
        "empty-body should be present in warning bucket"
    );
    assert!(
        report
            .errors
            .iter()
            .any(|issue| issue.rule_id == "invalid-frontmatter"),
        "invalid-frontmatter should be present in error bucket"
    );
    assert!(
        report
            .errors
            .iter()
            .chain(report.warnings.iter())
            .chain(report.infos.iter())
            .all(|issue| issue.rule_id != "scan-diagnostic"),
        "doctor report should only emit enum-backed kebab-case rule ids"
    );
}
