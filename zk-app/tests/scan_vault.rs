use std::collections::BTreeMap;

mod common;

use common::{unique_test_dir, write_text};
use zk_app::scan_vault;
use zk_domain::config::EffectiveConfig;

#[test]
fn scan_vault_facade_returns_scan_links_and_validation() {
    let root = unique_test_dir("scan-vault-facade");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("source.md"),
        r#"---
id: 01SOURCE000000000000000001
title: Source
tags:
  - scan
---
# Source

Resolved wiki: [[Target Note]]
Unresolved wiki: [[Missing Note]]
External: [Example](https://example.com)
Asset: ![Diagram](./assets/diagram.png)
"#,
    );
    write_text(
        &vault_root.join("target.md"),
        r#"---
id: 01TARGET000000000000000001
title: Target Note
tags:
  - scan
---
# Target

Body content
"#,
    );
    write_text(
        &vault_root.join("no-h1.md"),
        r#"---
id: 01NOH10000000000000000001
title: No H1
tags:
  - scan
---
This body has no H1 heading.
"#,
    );
    write_text(&vault_root.join("assets").join("diagram.png"), "png-bytes");

    let mut config = EffectiveConfig::default();
    config.vault_path = vault_root.clone();
    config.allowed_note_extensions = vec!["md".to_string()];
    config.validation = zk_domain::config::ValidationConfig {
        disabled_rules: Vec::new(),
        severity_overrides: BTreeMap::from([("NoH1InBody".to_string(), "error".to_string())]),
    };

    let outcome = scan_vault(&vault_root, &config).expect("scan_vault should succeed");

    assert_eq!(outcome.scan_result.notes.len(), 3);
    assert_eq!(outcome.scan_result.assets.len(), 1);
    assert_eq!(outcome.parsed_indexes.len(), 3);
    assert!(outcome.validation_report.summary.error_count >= 1);

    let target_id = outcome
        .parsed_indexes
        .iter()
        .find(|parsed_index| parsed_index.title == "Target Note")
        .expect("target index should exist")
        .note_id
        .clone();
    let source = outcome
        .parsed_indexes
        .iter()
        .find(|parsed_index| parsed_index.title == "Source")
        .expect("source index should exist");

    let resolved_link = source
        .outbound_note_links
        .iter()
        .find(|link| link.raw_target == "Target Note")
        .expect("resolved link should exist");
    assert_eq!(resolved_link.target_note_id, Some(target_id));

    let unresolved_link = source
        .outbound_note_links
        .iter()
        .find(|link| link.raw_target == "Missing Note")
        .expect("unresolved link should exist");
    assert_eq!(unresolved_link.target_note_id, None);

    assert_eq!(source.outbound_external_links.len(), 1);
    assert_eq!(source.asset_references.len(), 1);
}
