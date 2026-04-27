// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

mod common;

use common::{unique_test_dir, write_text};
use tracing_subscriber::{Layer, layer::Context, prelude::*};
use zk_app::{RebuildOptions, rebuild_index, status_summary};
use zk_domain::config::{EffectiveConfig, ValidationConfig};
use zk_index::open_memory;

#[derive(Clone, Default)]
struct ScanCountLayer {
    count: Arc<AtomicUsize>,
}

impl<S> Layer<S> for ScanCountLayer
where
    S: tracing::Subscriber,
{
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: Context<'_, S>,
    ) {
        if attributes.metadata().name() == "zk_app.scan_vault" {
            let _ = self.count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[test]
fn status_summary_reports_vault_and_index_snapshot_counts() {
    let root = unique_test_dir("status-summary");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("notes").join("clean-a.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAA
title: Clean A
tags:
  - tg8
---
# Clean A

Unresolved wiki: [[Missing Note]]
Asset: ![Diagram](../assets/diagram.png)
",
    );
    write_text(
        &vault_root.join("notes").join("clean-b.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAB
title: Clean B
tags:
  - tg8
---
# Clean B

Body content
",
    );
    write_text(
        &vault_root.join("notes").join("missing-title.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAC
tags:
  - tg8
---
Body without an H1 heading.
",
    );
    write_text(
        &vault_root.join("99_trash").join("old.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FAD
title: Old Note
tags:
  - tg8
---
# Old Note

Trashed content
",
    );
    write_text(&vault_root.join("assets").join("diagram.png"), "png-bytes");

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        validation: ValidationConfig {
            disabled_rules: Vec::new(),
            severity_overrides: BTreeMap::from([(
                "TitleDerivedFromFilename".to_string(),
                "error".to_string(),
            )]),
        },
        ..EffectiveConfig::default()
    };

    let mut conn = open_memory().expect("open in-memory SQLite");
    rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("initial rebuild should succeed");

    let summary =
        status_summary(&vault_root, &config, &conn).expect("status summary should succeed");

    assert_eq!(summary.total_notes, 4);
    assert_eq!(summary.total_assets, 1);
    assert!(summary.notes_with_errors >= 1);
    assert_eq!(summary.unresolved_link_count, 1);
    assert_eq!(summary.duplicate_id_count, 0);
    assert_eq!(summary.trashed_notes, 1);
}

#[test]
fn status_summary_scans_vault_once_per_call() {
    let root = unique_test_dir("status-summary-scan-count");
    let vault_root = root.join("vault");

    write_text(
        &vault_root.join("notes").join("kept.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FBA
title: Kept
tags:
  - tg9
---
# Kept

Body content
",
    );
    write_text(
        &vault_root.join("99_trash").join("old.md"),
        r"---
id: 01ARZ3NDEKTSV4RRFFQ69G5FBB
title: Old
tags:
  - tg9
---
# Old

Trashed content
",
    );

    let config = EffectiveConfig {
        vault_path: vault_root.clone(),
        allowed_note_extensions: vec!["md".to_string()],
        ..EffectiveConfig::default()
    };

    let mut conn = open_memory().expect("open in-memory SQLite");
    rebuild_index(&vault_root, &config, &mut conn, RebuildOptions::default())
        .expect("initial rebuild should succeed");

    let layer = ScanCountLayer::default();
    let count = Arc::clone(&layer.count);
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let summary =
        status_summary(&vault_root, &config, &conn).expect("status summary should succeed");

    assert_eq!(summary.trashed_notes, 1);
    assert_eq!(count.load(Ordering::Relaxed), 1);
}
