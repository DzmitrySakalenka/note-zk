use std::{collections::BTreeMap, fs, io::ErrorKind, path::Path};

mod common;

use common::{unique_test_dir, write_text};
use zk_app::{AppError, build_asset_inputs, build_note_inputs};
use zk_domain::{
    derived::{AssetReference, ExternalLink, ParsedNoteIndex, ResolvedNoteLink},
    id::{ContentHash, NoteId, RelativePath},
    note::{DiagnosticLevel, NoteDocument, NoteFrontmatter, ScanDiagnostic},
};
use zk_vault::{
    AssetCandidate, NormalizationPlan, NormalizationResult, TitleSource, VaultScanResult,
};

#[test]
fn build_note_inputs_populates_links_and_roundtrips_frontmatter() {
    let note_id = NoteId::from("01TESTNOTEID00000000000000");
    let frontmatter = note_frontmatter(&note_id, "Bridge Note");
    let scan_result = VaultScanResult {
        notes: vec![normalization_result(
            "notes/bridge.md",
            frontmatter.clone(),
            "body",
            "hash-note-1",
        )],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    };
    let parsed_indexes = vec![ParsedNoteIndex {
        note_id: note_id.clone(),
        title: frontmatter.title.clone(),
        tags: frontmatter.tags.clone(),
        aliases: frontmatter.aliases.clone(),
        outbound_note_links: vec![
            ResolvedNoteLink {
                source_note_id: note_id.clone(),
                raw_target: "Existing Note".to_string(),
                target_note_id: Some(NoteId::from("01TARGETNOTEID000000000000")),
                link_text: Some("Existing".to_string()),
                is_ambiguous: false,
            },
            ResolvedNoteLink {
                source_note_id: note_id.clone(),
                raw_target: "Missing Note".to_string(),
                target_note_id: None,
                link_text: Some("Missing".to_string()),
                is_ambiguous: false,
            },
        ],
        outbound_external_links: vec![ExternalLink {
            source_note_id: note_id.clone(),
            url: "https://example.com".to_string(),
            link_text: Some("Example".to_string()),
        }],
        asset_references: vec![AssetReference {
            source_note_id: note_id.clone(),
            raw_path: "./assets/diagram.png".to_string(),
            target_asset_id: None,
        }],
        has_mermaid: false,
    }];

    let note_inputs = build_note_inputs(&scan_result, &parsed_indexes).expect("build note inputs");

    assert_eq!(note_inputs.len(), 1);
    let note_input = &note_inputs[0];
    assert_eq!(note_input.note_id, note_id);
    assert_eq!(note_input.path.as_str(), "notes/bridge.md");
    assert_eq!(note_input.title, "Bridge Note");
    assert_eq!(note_input.content_hash.as_str(), "hash-note-1");
    assert_eq!(note_input.created_at, "2026-04-16T10:00:00Z");
    assert_eq!(note_input.updated_at, "");
    assert_eq!(
        note_input.tags,
        vec!["zettel".to_string(), "bridge".to_string()]
    );
    assert_eq!(
        note_input.aliases,
        vec!["Bridge Alias".to_string(), "Bridge Alt".to_string()]
    );
    assert_eq!(note_input.note_links.len(), 2);
    assert_eq!(note_input.external_links.len(), 1);
    assert_eq!(note_input.asset_references.len(), 1);
    assert_eq!(note_input.note_links[0].raw_target, "Existing Note");
    assert_eq!(
        note_input.note_links[1].target_note_id, None,
        "unresolved link should remain unresolved"
    );

    let round_tripped: NoteFrontmatter =
        serde_json::from_str(&note_input.frontmatter_json).expect("frontmatter json roundtrip");
    assert_eq!(round_tripped, frontmatter);
}

#[test]
fn build_note_inputs_omits_notes_without_document() {
    let note_id = NoteId::from("01NODOCNOTEID00000000000000");
    let scan_result = VaultScanResult {
        notes: vec![NormalizationResult {
            relative_path: RelativePath::new("notes/no-document.md"),
            plan: empty_plan(),
            frontmatter: None,
            document: None,
            content_hash: ContentHash::from("hash-no-document"),
        }],
        assets: Vec::new(),
        diagnostics: vec![ScanDiagnostic {
            level: DiagnosticLevel::Error,
            path: RelativePath::new("notes/no-document.md"),
            message: "parse failed".to_string(),
        }],
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: vec![(note_id, vec![RelativePath::new("notes/no-document.md")])],
    };

    let note_inputs = build_note_inputs(&scan_result, &[]).expect("build note inputs");

    assert!(note_inputs.is_empty());
}

#[test]
fn build_note_inputs_omits_duplicate_note_ids() {
    let note_id = NoteId::from("01DUPLICATENOTEID0000000000");
    let frontmatter = note_frontmatter(&note_id, "Duplicate Note");
    let scan_result = VaultScanResult {
        notes: vec![normalization_result(
            "notes/duplicate.md",
            frontmatter.clone(),
            "body",
            "hash-duplicate",
        )],
        assets: Vec::new(),
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: vec![(
            note_id.clone(),
            vec![RelativePath::new("notes/duplicate.md")],
        )],
    };
    let parsed_indexes = vec![ParsedNoteIndex {
        note_id,
        title: frontmatter.title,
        tags: frontmatter.tags,
        aliases: frontmatter.aliases,
        outbound_note_links: Vec::new(),
        outbound_external_links: Vec::new(),
        asset_references: Vec::new(),
        has_mermaid: false,
    }];

    let note_inputs = build_note_inputs(&scan_result, &parsed_indexes).expect("build note inputs");

    assert!(note_inputs.is_empty());
}

#[test]
fn build_asset_inputs_populates_hash_size_and_mime_type() {
    let root = unique_test_dir("bridge-assets");
    let vault_root = root.join("vault");
    let png_path = vault_root.join("assets").join("diagram.png");
    let unknown_path = vault_root.join("assets").join("blob.unknown");
    fs::create_dir_all(png_path.parent().expect("png parent")).expect("create asset directory");
    let png_text = "png-bytes";
    let unknown_text = "opaque";
    write_text(&png_path, png_text);
    write_text(&unknown_path, unknown_text);

    let scan_result = VaultScanResult {
        notes: Vec::new(),
        assets: vec![
            asset_candidate("assets/diagram.png", &png_path),
            asset_candidate("assets/blob.unknown", &unknown_path),
        ],
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    };

    let asset_inputs = build_asset_inputs(&scan_result, &vault_root).expect("build asset inputs");

    assert_eq!(asset_inputs.len(), 2);
    assert_eq!(asset_inputs[0].path.as_str(), "assets/diagram.png");
    assert_eq!(
        asset_inputs[0].content_hash,
        ContentHash::from_bytes(png_text.as_bytes()),
    );
    assert_eq!(asset_inputs[0].size_bytes, png_text.len() as u64);
    assert_eq!(asset_inputs[0].mime_type.as_deref(), Some("image/png"));
    assert_eq!(asset_inputs[1].path.as_str(), "assets/blob.unknown");
    assert_eq!(asset_inputs[1].mime_type, None);
}

#[test]
fn build_asset_inputs_stream_hashes_large_assets() {
    let root = unique_test_dir("bridge-assets-large");
    let vault_root = root.join("vault");
    let asset_path = vault_root.join("assets").join("large.bin");
    fs::create_dir_all(asset_path.parent().expect("asset parent")).expect("create asset directory");
    let bytes: Vec<u8> = (0..(256 * 1024))
        .map(|index| u8::try_from(index % 251).expect("value should always fit into u8"))
        .collect();
    fs::write(&asset_path, &bytes).expect("write large asset");

    let scan_result = VaultScanResult {
        notes: Vec::new(),
        assets: vec![asset_candidate("assets/large.bin", &asset_path)],
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    };

    let asset_inputs = build_asset_inputs(&scan_result, &vault_root).expect("build asset inputs");

    assert_eq!(asset_inputs.len(), 1);
    assert_eq!(
        asset_inputs[0].content_hash,
        ContentHash::from_bytes(&bytes),
    );
    assert_eq!(asset_inputs[0].size_bytes, bytes.len() as u64);
    assert_eq!(asset_inputs[0].mime_type, None);
}

#[test]
fn build_asset_inputs_surfaces_io_failures() {
    let root = unique_test_dir("bridge-missing-asset");
    let vault_root = root.join("vault");
    fs::create_dir_all(&vault_root).expect("create vault root");
    let missing_path = vault_root.join("assets").join("missing.png");
    let scan_result = VaultScanResult {
        notes: Vec::new(),
        assets: vec![asset_candidate("assets/missing.png", &missing_path)],
        diagnostics: Vec::new(),
        frontmatter_parse_failures: Vec::new(),
        duplicate_ids: Vec::new(),
    };

    let err = build_asset_inputs(&scan_result, &vault_root).expect_err("missing asset should fail");

    match err {
        AppError::Io { path, source } => {
            assert_eq!(path, missing_path);
            assert_eq!(source.kind(), ErrorKind::NotFound);
        }
        other => panic!("expected AppError::Io, got {other:?}"),
    }
}

fn note_frontmatter(note_id: &NoteId, title: &str) -> NoteFrontmatter {
    NoteFrontmatter {
        id: note_id.clone(),
        title: title.to_string(),
        tags: vec!["zettel".to_string(), "bridge".to_string()],
        note_type: Some("concept".to_string()),
        aliases: vec!["Bridge Alias".to_string(), "Bridge Alt".to_string()],
        created: Some("2026-04-16T10:00:00Z".to_string()),
        updated: None,
        source: Some("https://example.com/source".to_string()),
        status: Some("draft".to_string()),
        sources: vec![NoteId::from("01SOURCENOTEID000000000000")],
        extra: BTreeMap::new(),
    }
}

fn normalization_result(
    relative_path: &str,
    frontmatter: NoteFrontmatter,
    body_markdown: &str,
    content_hash: &str,
) -> NormalizationResult {
    NormalizationResult {
        relative_path: RelativePath::new(relative_path),
        plan: empty_plan(),
        frontmatter: Some(frontmatter.clone()),
        document: Some(NoteDocument {
            frontmatter,
            body_markdown: body_markdown.to_string(),
        }),
        content_hash: ContentHash::from(content_hash),
    }
}

fn asset_candidate(relative_path: &str, absolute_path: &Path) -> AssetCandidate {
    AssetCandidate {
        relative_path: RelativePath::new(relative_path),
        absolute_path: absolute_path.to_path_buf(),
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
