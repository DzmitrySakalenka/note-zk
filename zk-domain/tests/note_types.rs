use std::collections::BTreeMap;

use zk_domain::id::{ContentHash, DeviceId, NoteId, RelativePath, ServerRevision};
use zk_domain::note::{NoteDocument, NoteFrontmatter, NoteRecord};

// ==========================================================================
// NoteFrontmatter -- construction and field access
// ==========================================================================

fn sample_frontmatter() -> NoteFrontmatter {
    NoteFrontmatter {
        id: NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMP"),
        title: "Knowledge Graph Basics".to_string(),
        tags: vec!["graph".to_string(), "rust".to_string()],
        note_type: Some("concept".to_string()),
        aliases: vec!["KG basics".to_string()],
        created: Some("2026-04-02T10:00:00Z".to_string()),
        updated: Some("2026-04-02T10:00:00Z".to_string()),
        source: Some("https://example.com".to_string()),
        status: Some("active".to_string()),
        sources: vec![],
        extra: BTreeMap::default(),
    }
}

#[test]
fn frontmatter_required_fields_accessible() {
    let fm = sample_frontmatter();
    assert_eq!(fm.id.to_string(), "01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    assert_eq!(fm.title, "Knowledge Graph Basics");
    assert_eq!(fm.tags, vec!["graph", "rust"]);
}

#[test]
fn frontmatter_optional_fields() {
    let fm = sample_frontmatter();
    assert_eq!(fm.note_type.as_deref(), Some("concept"));
    assert_eq!(fm.aliases, vec!["KG basics"]);
    assert_eq!(fm.created.as_deref(), Some("2026-04-02T10:00:00Z"));
    assert_eq!(fm.updated.as_deref(), Some("2026-04-02T10:00:00Z"));
    assert_eq!(fm.source.as_deref(), Some("https://example.com"));
    assert_eq!(fm.status.as_deref(), Some("active"));
}

#[test]
fn frontmatter_sources_for_wiki_provenance() {
    let fm = NoteFrontmatter {
        sources: vec![NoteId::new("01AAA"), NoteId::new("01BBB")],
        ..sample_frontmatter()
    };
    assert_eq!(fm.sources.len(), 2);
    assert_eq!(fm.sources[0].to_string(), "01AAA");
}

#[test]
fn frontmatter_minimal_with_defaults() {
    let fm = NoteFrontmatter {
        id: NoteId::new("01MINIMAL"),
        title: "Minimal Note".to_string(),
        tags: vec!["untagged".to_string()],
        note_type: None,
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: vec![],
        extra: BTreeMap::default(),
    };
    assert_eq!(fm.tags, vec!["untagged"]);
    assert!(fm.note_type.is_none());
    assert!(fm.aliases.is_empty());
    assert!(fm.sources.is_empty());
}

#[test]
fn frontmatter_clone_and_eq() {
    let a = sample_frontmatter();
    let b = a.clone();
    assert_eq!(a, b);
}

// ==========================================================================
// NoteFrontmatter -- YAML serde
// ==========================================================================

#[test]
fn frontmatter_yaml_roundtrip_full() {
    let fm = sample_frontmatter();
    let yaml = serde_yaml::to_string(&fm).expect("serialize");
    let back: NoteFrontmatter = serde_yaml::from_str(&yaml).expect("deserialize");
    assert_eq!(fm, back);
}

#[test]
fn frontmatter_yaml_roundtrip_minimal() {
    let fm = NoteFrontmatter {
        id: NoteId::new("01MINIMAL"),
        title: "Minimal".to_string(),
        tags: vec!["untagged".to_string()],
        note_type: None,
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: vec![],
        extra: BTreeMap::default(),
    };
    let yaml = serde_yaml::to_string(&fm).expect("serialize");
    let back: NoteFrontmatter = serde_yaml::from_str(&yaml).expect("deserialize");
    assert_eq!(fm, back);
}

#[test]
fn frontmatter_deserialize_tags_as_single_string() {
    let yaml = r#"
id: "01TEST"
title: "Single Tag Test"
tags: rust
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(fm.tags, vec!["rust"]);
}

#[test]
fn frontmatter_deserialize_tags_as_list() {
    let yaml = r#"
id: "01TEST"
title: "List Tag Test"
tags:
  - rust
  - project/foo
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(fm.tags, vec!["rust", "project/foo"]);
}

#[test]
fn frontmatter_missing_tags_defaults_to_untagged() {
    let yaml = r#"
id: "01TEST"
title: "No Tags"
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(fm.tags, vec!["untagged"]);
}

#[test]
fn frontmatter_serialize_tags_always_as_list() {
    let fm = NoteFrontmatter {
        id: NoteId::new("01TEST"),
        title: "Test".to_string(),
        tags: vec!["single".to_string()],
        note_type: None,
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: vec![],
        extra: BTreeMap::default(),
    };
    let yaml = serde_yaml::to_string(&fm).expect("serialize");
    assert!(
        yaml.contains("- single"),
        "tags should serialize as YAML list, got:\n{yaml}"
    );
}

#[test]
fn frontmatter_deserialize_aliases_as_single_string() {
    let yaml = r#"
id: "01TEST"
title: "Alias Test"
tags:
  - test
aliases: "My Alias"
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(fm.aliases, vec!["My Alias"]);
}

#[test]
fn frontmatter_deserialize_aliases_as_list() {
    let yaml = r#"
id: "01TEST"
title: "Alias Test"
tags:
  - test
aliases:
  - Alias One
  - Alias Two
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(fm.aliases, vec!["Alias One", "Alias Two"]);
}

#[test]
fn frontmatter_missing_aliases_defaults_to_empty() {
    let yaml = r#"
id: "01TEST"
title: "No Aliases"
tags:
  - test
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert!(fm.aliases.is_empty());
}

#[test]
fn frontmatter_missing_sources_defaults_to_empty() {
    let yaml = r#"
id: "01TEST"
title: "No Sources"
tags:
  - test
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert!(fm.sources.is_empty());
}

// ==========================================================================
// NoteFrontmatter -- unknown keys preservation via `extra`
// ==========================================================================

#[test]
fn frontmatter_preserves_unknown_keys() {
    let yaml = r#"
id: "01TEST"
title: "Extra Fields"
tags:
  - test
custom_field: "hello"
priority: 5
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(
        fm.extra.get("custom_field").and_then(|v| v.as_str()),
        Some("hello"),
    );
    assert_eq!(
        fm.extra.get("priority").and_then(serde_yaml::Value::as_u64),
        Some(5),
    );
}

#[test]
fn frontmatter_unknown_keys_survive_roundtrip() {
    let yaml = r#"
id: "01TEST"
title: "Roundtrip Extra"
tags:
  - test
my_custom_key: "preserved"
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    let reserialized = serde_yaml::to_string(&fm).expect("serialize");
    let back: NoteFrontmatter = serde_yaml::from_str(&reserialized).expect("re-deserialize");
    assert_eq!(
        back.extra.get("my_custom_key").and_then(|v| v.as_str()),
        Some("preserved"),
    );
}

#[test]
fn frontmatter_unknown_keys_do_not_clobber_known_fields() {
    let yaml = r#"
id: "01TEST"
title: "No Clobber"
tags:
  - test
weird_key: true
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(fm.title, "No Clobber");
    assert_eq!(fm.tags, vec!["test"]);
    assert!(fm.extra.contains_key("weird_key"));
    assert!(!fm.extra.contains_key("title"));
    assert!(!fm.extra.contains_key("tags"));
}

// ==========================================================================
// NoteFrontmatter -- serde field naming
// ==========================================================================

#[test]
fn frontmatter_type_field_uses_yaml_key_type() {
    let yaml = r#"
id: "01TEST"
title: "Type Field"
tags:
  - test
type: concept
"#;
    let fm: NoteFrontmatter = serde_yaml::from_str(yaml).expect("deserialize");
    assert_eq!(fm.note_type.as_deref(), Some("concept"));
}

#[test]
fn frontmatter_type_field_serializes_as_type() {
    let fm = NoteFrontmatter {
        id: NoteId::new("01TEST"),
        title: "Test".to_string(),
        tags: vec!["test".to_string()],
        note_type: Some("concept".to_string()),
        aliases: vec![],
        created: None,
        updated: None,
        source: None,
        status: None,
        sources: vec![],
        extra: BTreeMap::default(),
    };
    let yaml = serde_yaml::to_string(&fm).expect("serialize");
    assert!(
        yaml.contains("type:"),
        "note_type field should serialize as 'type' in YAML, got:\n{yaml}"
    );
}

// ==========================================================================
// NoteDocument
// ==========================================================================

#[test]
fn note_document_construction() {
    let doc = NoteDocument {
        frontmatter: sample_frontmatter(),
        body_markdown: "# Knowledge Graph Basics\n\nSome content here.".to_string(),
    };
    assert_eq!(doc.frontmatter.title, "Knowledge Graph Basics");
    assert_eq!(
        doc.body_markdown,
        "# Knowledge Graph Basics\n\nSome content here."
    );
}

#[test]
fn note_document_clone_and_eq() {
    let a = NoteDocument {
        frontmatter: sample_frontmatter(),
        body_markdown: "body".to_string(),
    };
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn note_document_body_is_not_serialized_to_yaml() {
    let doc = NoteDocument {
        frontmatter: sample_frontmatter(),
        body_markdown: "# Title\n\nBody content".to_string(),
    };
    let fm_yaml = serde_yaml::to_string(&doc.frontmatter).expect("serialize frontmatter");
    assert!(
        !fm_yaml.contains("Body content"),
        "body should not appear in frontmatter YAML"
    );
}

// ==========================================================================
// NoteRecord
// ==========================================================================

fn sample_record() -> NoteRecord {
    NoteRecord {
        note_id: NoteId::new("01HZ3KQBFG9T2R5XJ4N8VW6YMP"),
        current_path: RelativePath::new("notes/knowledge-graph.md"),
        content_hash: ContentHash::from_bytes(b"sample content"),
        created_at: "2026-04-02T10:00:00Z".to_string(),
        updated_at: "2026-04-02T12:00:00Z".to_string(),
        trashed_at: None,
        deleted_at: None,
        is_trashed: false,
        is_deleted: false,
        last_editor_device_id: None,
        server_revision: ServerRevision::new(1),
    }
}

#[test]
fn note_record_construction() {
    let rec = sample_record();
    assert_eq!(rec.note_id.to_string(), "01HZ3KQBFG9T2R5XJ4N8VW6YMP");
    assert_eq!(rec.current_path.as_str(), "notes/knowledge-graph.md");
    assert!(!rec.is_trashed);
    assert!(!rec.is_deleted);
    assert!(rec.trashed_at.is_none());
    assert!(rec.deleted_at.is_none());
    assert!(rec.last_editor_device_id.is_none());
    assert_eq!(rec.server_revision.value(), 1);
}

#[test]
fn note_record_has_no_body_or_title() {
    // NoteRecord must NOT contain body (ADR 0013) or title fields.
    // Title lives in NoteFrontmatter (canonical) and ParsedNoteIndex (derived).
    // Body lives only on the filesystem.
    // This test is structural: if someone adds those fields, sample_record()
    // will fail to compile because it won't initialize them.
    let rec = sample_record();
    assert_eq!(rec.note_id.to_string(), "01HZ3KQBFG9T2R5XJ4N8VW6YMP");
}

#[test]
fn note_record_with_trash_state() {
    let rec = NoteRecord {
        trashed_at: Some("2026-04-05T08:00:00Z".to_string()),
        is_trashed: true,
        ..sample_record()
    };
    assert!(rec.is_trashed);
    assert!(!rec.is_deleted);
    assert_eq!(rec.trashed_at.as_deref(), Some("2026-04-05T08:00:00Z"));
    assert!(rec.deleted_at.is_none());
}

#[test]
fn note_record_with_deleted_state() {
    let rec = NoteRecord {
        trashed_at: Some("2026-04-05T08:00:00Z".to_string()),
        deleted_at: Some("2026-04-12T08:00:00Z".to_string()),
        is_trashed: true,
        is_deleted: true,
        ..sample_record()
    };
    assert!(rec.is_trashed);
    assert!(rec.is_deleted);
    assert_eq!(rec.deleted_at.as_deref(), Some("2026-04-12T08:00:00Z"));
}

#[test]
fn note_record_with_editor_device() {
    let rec = NoteRecord {
        last_editor_device_id: Some(DeviceId::new("device-macbook")),
        ..sample_record()
    };
    assert_eq!(
        rec.last_editor_device_id
            .as_ref()
            .map(std::string::ToString::to_string),
        Some("device-macbook".to_string())
    );
}

#[test]
fn note_record_clone_and_eq() {
    let a = sample_record();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn note_record_timestamps_are_sync_engine_not_user() {
    // created_at/updated_at on NoteRecord are sync-engine timestamps,
    // distinct from the optional created/updated in NoteFrontmatter.
    // They may coincidentally match, but are semantically different:
    //   rec.created_at = when the sync engine first saw the note
    //   fm.created     = user-authored metadata in the file
    let rec = sample_record();
    let fm = sample_frontmatter();
    assert_eq!(rec.created_at, "2026-04-02T10:00:00Z");
    assert_eq!(fm.created.as_deref(), Some("2026-04-02T10:00:00Z"));
}
