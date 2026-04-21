use std::{
    collections::BTreeMap,
    fmt, fs,
    path::Path,
    sync::{Arc, Mutex},
};

use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};
use zk_app::{build_asset_inputs, build_note_inputs};
use zk_domain::{
    derived::ParsedNoteIndex,
    id::{ContentHash, NoteId, RelativePath},
    note::{NoteDocument, NoteFrontmatter},
};
use zk_vault::{
    AssetCandidate, NormalizationPlan, NormalizationResult, TitleSource, TrashState,
    VaultScanResult,
};

mod common;

use common::{unique_test_dir, write_text};

#[derive(Debug, Clone, PartialEq, Eq)]
struct BridgeWarning {
    target: String,
    path: String,
    message: String,
}

#[derive(Clone, Default)]
struct BridgeWarningLayer {
    warnings: Arc<Mutex<Vec<BridgeWarning>>>,
}

impl<S> Layer<S> for BridgeWarningLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN || event.metadata().target() != "zk_app::bridge"
        {
            return;
        }

        let mut visitor = BridgeWarningVisitor::default();
        event.record(&mut visitor);

        self.warnings
            .lock()
            .expect("warning collection mutex should not be poisoned")
            .push(BridgeWarning {
                target: event.metadata().target().to_string(),
                path: visitor.path.unwrap_or_default(),
                message: visitor.message.unwrap_or_default(),
            });
    }
}

#[derive(Default)]
struct BridgeWarningVisitor {
    path: Option<String>,
    message: Option<String>,
}

impl Visit for BridgeWarningVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let value = format!("{value:?}");
        self.record_value(field.name(), &value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field.name(), value);
    }
}

impl BridgeWarningVisitor {
    fn record_value(&mut self, field_name: &str, value: &str) {
        let normalized = value.trim_matches('"').to_string();

        match field_name {
            "path" => self.path = Some(normalized),
            "message" => self.message = Some(normalized),
            _ => {}
        }
    }
}

#[test]
fn build_note_inputs_propagates_trash_fields_for_live_and_trashed_notes() {
    let live_id = NoteId::new("01LIVEBRIDGETRASHNOTE0000001");
    let trashed_id = NoteId::new("01TRASHBRIDGETRASHNOTE000001");
    let live_frontmatter = note_frontmatter(&live_id, "Live Note");
    let trashed_frontmatter = note_frontmatter(&trashed_id, "Trashed Note");
    let scan_result = VaultScanResult {
        notes: vec![
            normalization_result(
                "notes/live.md",
                live_frontmatter,
                "live body",
                "hash-live",
                None,
            ),
            normalization_result(
                "99_trash/trashed.md",
                trashed_frontmatter,
                "trashed body",
                "hash-trashed",
                Some("2026-04-20T08:00:00Z"),
            ),
        ],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
        trashed_note_count: 1,
    };
    let parsed_indexes = vec![
        parsed_index(&live_id, "Live Note"),
        parsed_index(&trashed_id, "Trashed Note"),
    ];

    let note_inputs = build_note_inputs(&scan_result, &parsed_indexes).expect("build note inputs");

    assert_eq!(note_inputs.len(), 2);
    let live_input = note_inputs
        .iter()
        .find(|input| input.path == RelativePath::new("notes/live.md"))
        .expect("live note input should exist");
    let trashed_input = note_inputs
        .iter()
        .find(|input| input.path == RelativePath::new("99_trash/trashed.md"))
        .expect("trashed note input should exist");

    assert!(!live_input.is_trashed);
    assert_eq!(live_input.trashed_at, None);
    assert!(trashed_input.is_trashed);
    assert_eq!(
        trashed_input.trashed_at.as_deref(),
        Some("2026-04-20T08:00:00Z")
    );
}

#[test]
fn build_asset_inputs_propagates_trash_fields_for_trashed_assets() {
    let root = unique_test_dir("bridge-trash-assets");
    let vault_root = root.join("vault");
    let trashed_path = vault_root.join("99_trash").join("diagram.png");
    fs::create_dir_all(trashed_path.parent().expect("asset parent should exist"))
        .expect("create asset directory");
    write_text(&trashed_path, "png-bytes");

    let scan_result = VaultScanResult {
        notes: Vec::new(),
        assets: vec![asset_candidate(
            "99_trash/diagram.png",
            &trashed_path,
            TrashState::Trashed,
            Some("2026-04-20T09:00:00Z"),
        )],
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
        trashed_note_count: 0,
    };

    let asset_inputs = build_asset_inputs(&scan_result, &vault_root).expect("build asset inputs");

    assert_eq!(asset_inputs.len(), 1);
    assert!(asset_inputs[0].is_trashed);
    assert_eq!(
        asset_inputs[0].trashed_at.as_deref(),
        Some("2026-04-20T09:00:00Z")
    );
}

#[test]
fn build_note_inputs_warns_and_drops_trashed_notes_missing_stable_id() {
    let live_id = NoteId::new("01LIVEBRIDGEDROPTRASHNOTE0001");
    let scan_result = VaultScanResult {
        notes: vec![
            normalization_result(
                "notes/live.md",
                note_frontmatter(&live_id, "Live Note"),
                "live body",
                "hash-live",
                None,
            ),
            missing_id_trashed_result("99_trash/lost.md", "hash-lost", "2026-04-20T10:00:00Z"),
        ],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
        trashed_note_count: 1,
    };
    let parsed_indexes = vec![parsed_index(&live_id, "Live Note")];
    let layer = BridgeWarningLayer::default();
    let warnings = Arc::clone(&layer.warnings);
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let note_inputs = build_note_inputs(&scan_result, &parsed_indexes).expect("build note inputs");

    assert_eq!(note_inputs.len(), 1);
    assert_eq!(note_inputs[0].path.as_str(), "notes/live.md");
    let warnings = warnings
        .lock()
        .expect("warning collection mutex should not be poisoned")
        .clone();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].target, "zk_app::bridge");
    assert_eq!(warnings[0].path, "99_trash/lost.md");
    assert!(
        warnings[0].message.contains("missing stable id"),
        "warning should describe the missing trashed note id"
    );
}

fn parsed_index(note_id: &NoteId, title: &str) -> ParsedNoteIndex {
    ParsedNoteIndex {
        note_id: note_id.clone(),
        title: title.to_string(),
        tags: vec!["bridge".to_string()],
        aliases: Vec::new(),
        outbound_note_links: Vec::new(),
        outbound_external_links: Vec::new(),
        asset_references: Vec::new(),
        has_mermaid: false,
    }
}

fn note_frontmatter(note_id: &NoteId, title: &str) -> NoteFrontmatter {
    NoteFrontmatter {
        id: note_id.clone(),
        title: title.to_string(),
        tags: vec!["bridge".to_string()],
        note_type: None,
        aliases: Vec::new(),
        created: Some("2026-04-16T10:00:00Z".to_string()),
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: BTreeMap::new(),
    }
}

fn normalization_result(
    relative_path: &str,
    frontmatter: NoteFrontmatter,
    body_markdown: &str,
    content_hash: &str,
    trashed_at: Option<&str>,
) -> NormalizationResult {
    let trash_state = if trashed_at.is_some() {
        TrashState::Trashed
    } else {
        TrashState::Live
    };

    NormalizationResult {
        relative_path: RelativePath::new(relative_path),
        note_id: Some(frontmatter.id.clone()),
        trash_state,
        trashed_at: trashed_at.map(ToString::to_string),
        plan: empty_plan(),
        frontmatter: Some(frontmatter.clone()),
        document: Some(NoteDocument {
            frontmatter,
            body_markdown: body_markdown.to_string(),
        }),
        content_hash: ContentHash::new(content_hash),
    }
}

fn missing_id_trashed_result(
    relative_path: &str,
    content_hash: &str,
    trashed_at: &str,
) -> NormalizationResult {
    NormalizationResult {
        relative_path: RelativePath::new(relative_path),
        note_id: None,
        trash_state: TrashState::Trashed,
        trashed_at: Some(trashed_at.to_string()),
        plan: empty_plan(),
        frontmatter: None,
        document: None,
        content_hash: ContentHash::new(content_hash),
    }
}

fn asset_candidate(
    relative_path: &str,
    absolute_path: &Path,
    trash_state: TrashState,
    trashed_at: Option<&str>,
) -> AssetCandidate {
    AssetCandidate {
        relative_path: RelativePath::new(relative_path),
        absolute_path: absolute_path.to_path_buf(),
        trash_state,
        trashed_at: trashed_at.map(ToString::to_string),
    }
}

fn empty_plan() -> NormalizationPlan {
    NormalizationPlan {
        needs_id_injection: false,
        needs_title_derivation: false,
        title_source: TitleSource::Frontmatter,
        needs_frontmatter_insertion: false,
        needs_field_reorder: false,
        is_immutable: false,
        diagnostics: Vec::new(),
    }
}
