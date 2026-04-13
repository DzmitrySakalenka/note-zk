use std::{collections::BTreeMap, path::PathBuf};

use zk_domain::{
    config::EffectiveConfig,
    id::{NoteId, RelativePath},
    note::{DiagnosticLevel, RawNoteFrontmatter, ScanDiagnostic},
};
use zk_vault::{
    BodyAnalysis, NormalizationContext, NoteCandidate, RawNoteDocument, TitleSource,
    normalize_frontmatter_with_id_generator, plan_normalization,
};

fn base_config() -> EffectiveConfig {
    EffectiveConfig::default()
}

fn raw_frontmatter() -> RawNoteFrontmatter {
    RawNoteFrontmatter {
        id: None,
        title: None,
        tags: vec!["untagged".to_string()],
        note_type: None,
        aliases: Vec::new(),
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: Vec::new(),
        extra: BTreeMap::new(),
    }
}

fn note_candidate(relative_path: &str, is_immutable: bool) -> NoteCandidate {
    NoteCandidate {
        relative_path: RelativePath::new(relative_path),
        absolute_path: PathBuf::from(relative_path),
        is_immutable,
    }
}

fn raw_doc(
    frontmatter: Option<RawNoteFrontmatter>,
    raw_frontmatter_yaml: Option<&str>,
    body_markdown: &str,
    has_frontmatter_delimiters: bool,
) -> RawNoteDocument {
    RawNoteDocument {
        frontmatter,
        raw_frontmatter_yaml: raw_frontmatter_yaml.map(ToString::to_string),
        body_markdown: body_markdown.to_string(),
        has_frontmatter_delimiters,
    }
}

fn has_diagnostic(
    diagnostics: &[ScanDiagnostic],
    level: DiagnosticLevel,
    message_snippet: &str,
) -> bool {
    diagnostics
        .iter()
        .any(|d| d.level == level && d.message.contains(message_snippet))
}

#[test]
fn normalize_injects_missing_id_with_deterministic_generator() {
    let mut raw = raw_frontmatter();
    raw.title = Some("Existing Title".to_string());
    raw.tags = vec!["rust".to_string()];

    let relative_path = RelativePath::new("notes/example.md");
    let body_analysis = BodyAnalysis {
        first_h1: Some("Ignored".to_string()),
        has_mermaid: false,
    };
    let config = base_config();
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let generated_id = NoteId::from("01HARD-CODED-ID-FOR-TEST");
    let (normalized, diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, || generated_id.clone());

    assert_eq!(normalized.id, generated_id);
    assert!(
        has_diagnostic(&diagnostics, DiagnosticLevel::Info, "id"),
        "expected info diagnostic for ID injection"
    );
}

#[test]
fn normalize_preserves_existing_id_without_id_injection_diagnostic() {
    let existing_id = NoteId::from("01EXISTINGIDVALUE1234567890");
    let mut raw = raw_frontmatter();
    raw.id = Some(existing_id.clone());
    raw.title = Some("Stable Title".to_string());
    raw.tags = vec!["rust".to_string()];

    let relative_path = RelativePath::new("notes/example.md");
    let body_analysis = BodyAnalysis {
        first_h1: Some("Body Heading".to_string()),
        has_mermaid: false,
    };
    let config = base_config();
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let (normalized, diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, NoteId::generate);

    assert_eq!(normalized.id, existing_id);
    assert!(
        !has_diagnostic(&diagnostics, DiagnosticLevel::Info, "id"),
        "did not expect info diagnostic for ID injection"
    );
}

#[test]
fn normalize_uses_frontmatter_title_over_h1_and_filename() {
    let mut raw = raw_frontmatter();
    raw.id = Some(NoteId::from("01A"));
    raw.title = Some("Frontmatter Title".to_string());
    raw.tags = vec!["rust".to_string()];

    let relative_path = RelativePath::new("notes/file-name.md");
    let body_analysis = BodyAnalysis {
        first_h1: Some("Body Heading".to_string()),
        has_mermaid: false,
    };
    let config = base_config();
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let (normalized, _diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, NoteId::generate);

    assert_eq!(normalized.title, "Frontmatter Title");
}

#[test]
fn normalize_derives_title_from_h1_when_frontmatter_title_is_missing() {
    let mut raw = raw_frontmatter();
    raw.id = Some(NoteId::from("01B"));
    raw.tags = vec!["rust".to_string()];

    let relative_path = RelativePath::new("notes/file-name.md");
    let body_analysis = BodyAnalysis {
        first_h1: Some("Heading Title".to_string()),
        has_mermaid: false,
    };
    let config = base_config();
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let (normalized, diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, NoteId::generate);

    assert_eq!(normalized.title, "Heading Title");
    assert!(
        has_diagnostic(&diagnostics, DiagnosticLevel::Info, "title"),
        "expected info diagnostic for title derivation"
    );
}

#[test]
fn normalize_derives_title_from_filename_when_h1_is_missing() {
    let mut raw = raw_frontmatter();
    raw.id = Some(NoteId::from("01C"));
    raw.tags = vec!["rust".to_string()];

    let relative_path = RelativePath::new("deep/my_file-name.md");
    let body_analysis = BodyAnalysis {
        first_h1: None,
        has_mermaid: false,
    };
    let config = base_config();
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let (normalized, _diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, NoteId::generate);

    assert_eq!(normalized.title, "My File Name");
}

#[test]
fn normalize_derives_title_from_filename_when_h1_derivation_disabled() {
    let mut raw = raw_frontmatter();
    raw.id = Some(NoteId::from("01D"));
    raw.tags = vec!["rust".to_string()];

    let relative_path = RelativePath::new("notes/h1_is_ignored.md");
    let body_analysis = BodyAnalysis {
        first_h1: Some("Should Be Ignored".to_string()),
        has_mermaid: false,
    };
    let mut config = base_config();
    config.derive_title_from_h1 = false;
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let (normalized, _diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, NoteId::generate);

    assert_eq!(normalized.title, "H1 Is Ignored");
}

#[test]
fn normalize_emits_info_diagnostic_for_untagged_default() {
    let mut raw = raw_frontmatter();
    raw.id = Some(NoteId::from("01E"));
    raw.title = Some("Tagged".to_string());
    raw.tags = vec!["untagged".to_string()];

    let relative_path = RelativePath::new("notes/tagged.md");
    let body_analysis = BodyAnalysis {
        first_h1: None,
        has_mermaid: false,
    };
    let config = base_config();
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let (_normalized, diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, NoteId::generate);

    assert!(
        has_diagnostic(&diagnostics, DiagnosticLevel::Info, "untagged"),
        "expected info diagnostic for untagged default"
    );
}

#[test]
fn plan_marks_immutable_notes_as_non_writable_while_reporting_needed_changes() {
    let mut raw = raw_frontmatter();
    raw.tags = vec!["rust".to_string()];

    let raw_doc = raw_doc(
        Some(raw),
        Some("tags:\n  - rust\n"),
        "# Immutable Heading\n",
        true,
    );
    let candidate = note_candidate("raw/imported-note.md", true);
    let config = base_config();

    let plan = plan_normalization(&raw_doc, &candidate, &config);

    assert!(plan.is_immutable);
    assert!(plan.needs_id_injection);
    assert!(plan.needs_title_derivation);
    assert_eq!(plan.title_source, TitleSource::H1);
}

#[test]
fn plan_reports_warning_and_skips_when_frontmatter_insertion_is_disabled() {
    let raw_doc = raw_doc(None, None, "# No Frontmatter\n", false);
    let candidate = note_candidate("notes/no-frontmatter.md", false);
    let mut config = base_config();
    config.allow_insert_frontmatter = false;

    let plan = plan_normalization(&raw_doc, &candidate, &config);

    assert!(
        has_diagnostic(&plan.diagnostics, DiagnosticLevel::Warning, "frontmatter"),
        "expected warning when insertion is disabled"
    );
    assert!(!plan.needs_id_injection);
    assert!(!plan.needs_title_derivation);
}

#[test]
fn plan_marks_frontmatter_insertion_when_missing_and_allowed() {
    let raw_doc = raw_doc(None, None, "# Missing Frontmatter\n", false);
    let candidate = note_candidate("notes/missing-frontmatter.md", false);
    let config = base_config();

    let plan = plan_normalization(&raw_doc, &candidate, &config);

    assert!(plan.needs_frontmatter_insertion);
    assert!(plan.needs_id_injection);
    assert!(plan.needs_title_derivation);
    assert_eq!(plan.title_source, TitleSource::H1);
}

#[test]
fn plan_detects_field_reorder_when_yaml_key_order_is_non_canonical() {
    let mut raw = raw_frontmatter();
    raw.id = Some(NoteId::from("01ORDER"));
    raw.title = Some("Order".to_string());
    raw.tags = vec!["rust".to_string()];

    let raw_doc = raw_doc(
        Some(raw),
        Some("title: Order\nid: 01ORDER\ntags:\n  - rust\n"),
        "Body\n",
        true,
    );
    let candidate = note_candidate("notes/order.md", false);
    let config = base_config();

    let plan = plan_normalization(&raw_doc, &candidate, &config);

    assert!(plan.needs_field_reorder);
}

#[test]
fn normalize_preserves_non_required_fields_and_extra_map() {
    let mut raw = raw_frontmatter();
    raw.tags = vec!["rust".to_string()];
    raw.note_type = Some("concept".to_string());
    raw.aliases = vec!["Alias".to_string()];
    raw.created = Some("2026-04-13T10:00:00Z".to_string());
    raw.updated = Some("2026-04-13T10:05:00Z".to_string());
    raw.source = Some("https://example.com".to_string());
    raw.status = Some("active".to_string());
    raw.sources = vec![NoteId::from("01SOURCE")];
    raw.extra
        .insert("custom_flag".to_string(), serde_yaml::Value::Bool(true));

    let relative_path = RelativePath::new("notes/derived-title.md");
    let body_analysis = BodyAnalysis {
        first_h1: Some("Derived Title".to_string()),
        has_mermaid: false,
    };
    let config = base_config();
    let context = NormalizationContext {
        relative_path: &relative_path,
        body_analysis: &body_analysis,
        config: &config,
        is_immutable: false,
    };

    let generated_id = NoteId::from("01GENERATED");
    let (normalized, _diagnostics) =
        normalize_frontmatter_with_id_generator(raw, &context, || generated_id.clone());

    assert_eq!(normalized.id, generated_id);
    assert_eq!(normalized.title, "Derived Title");
    assert_eq!(normalized.tags, vec!["rust".to_string()]);
    assert_eq!(normalized.note_type, Some("concept".to_string()));
    assert_eq!(normalized.aliases, vec!["Alias".to_string()]);
    assert_eq!(normalized.created, Some("2026-04-13T10:00:00Z".to_string()));
    assert_eq!(normalized.updated, Some("2026-04-13T10:05:00Z".to_string()));
    assert_eq!(normalized.source, Some("https://example.com".to_string()));
    assert_eq!(normalized.status, Some("active".to_string()));
    assert_eq!(normalized.sources, vec![NoteId::from("01SOURCE")]);
    assert_eq!(
        normalized.extra.get("custom_flag"),
        Some(&serde_yaml::Value::Bool(true))
    );
}
