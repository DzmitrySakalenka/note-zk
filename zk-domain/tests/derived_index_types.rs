use zk_domain::derived::{AssetReference, ExternalLink, ParsedNoteIndex, ResolvedNoteLink};
use zk_domain::id::{AssetId, NoteId};

// ==========================================================================
// ResolvedNoteLink -- construction and field access
// ==========================================================================

fn sample_resolved_link() -> ResolvedNoteLink {
    ResolvedNoteLink {
        source_note_id: NoteId::from("01HZ3NOTE000000000000000A"),
        raw_target: "Some Other Note".to_string(),
        target_note_id: Some(NoteId::from("01HZ3NOTE000000000000000B")),
        link_text: Some("alias text".to_string()),
        is_ambiguous: false,
    }
}

#[test]
fn resolved_note_link_construction() {
    let link = sample_resolved_link();
    assert_eq!(link.source_note_id.to_string(), "01HZ3NOTE000000000000000A");
    assert_eq!(link.raw_target, "Some Other Note");
    assert_eq!(
        link.target_note_id
            .as_ref()
            .map(std::string::ToString::to_string),
        Some("01HZ3NOTE000000000000000B".to_string())
    );
    assert_eq!(link.link_text.as_deref(), Some("alias text"));
    assert!(!link.is_ambiguous);
}

#[test]
fn resolved_note_link_unresolved_not_found() {
    // Target not found: target_note_id is None, is_ambiguous is false.
    let link = ResolvedNoteLink {
        target_note_id: None,
        link_text: None,
        is_ambiguous: false,
        ..sample_resolved_link()
    };
    assert!(link.target_note_id.is_none());
    assert!(!link.is_ambiguous);
}

#[test]
fn resolved_note_link_unresolved_ambiguous() {
    // Multiple matches: target_note_id is None, is_ambiguous is true.
    let link = ResolvedNoteLink {
        target_note_id: None,
        is_ambiguous: true,
        ..sample_resolved_link()
    };
    assert!(link.target_note_id.is_none());
    assert!(link.is_ambiguous);
}

#[test]
fn resolved_note_link_wiki_link_without_alias() {
    // [[Note Title]] form -- no pipe alias.
    let link = ResolvedNoteLink {
        raw_target: "Note Title".to_string(),
        link_text: None,
        ..sample_resolved_link()
    };
    assert_eq!(link.raw_target, "Note Title");
    assert!(link.link_text.is_none());
}

#[test]
fn resolved_note_link_wiki_link_with_path() {
    // [[path/to/note]] form.
    let link = ResolvedNoteLink {
        raw_target: "path/to/note".to_string(),
        ..sample_resolved_link()
    };
    assert_eq!(link.raw_target, "path/to/note");
}

#[test]
fn resolved_note_link_clone_and_eq() {
    let a = sample_resolved_link();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn resolved_note_link_different_target_not_equal() {
    let a = sample_resolved_link();
    let b = ResolvedNoteLink {
        raw_target: "Different Note".to_string(),
        ..sample_resolved_link()
    };
    assert_ne!(a, b);
}

// ==========================================================================
// ExternalLink -- construction and field access
// ==========================================================================

fn sample_external_link() -> ExternalLink {
    ExternalLink {
        source_note_id: NoteId::from("01HZ3NOTE000000000000000A"),
        url: "https://example.com/article".to_string(),
        link_text: Some("Example Article".to_string()),
    }
}

#[test]
fn external_link_construction() {
    let link = sample_external_link();
    assert_eq!(link.source_note_id.to_string(), "01HZ3NOTE000000000000000A");
    assert_eq!(link.url, "https://example.com/article");
    assert_eq!(link.link_text.as_deref(), Some("Example Article"));
}

#[test]
fn external_link_without_text() {
    let link = ExternalLink {
        link_text: None,
        ..sample_external_link()
    };
    assert!(link.link_text.is_none());
}

#[test]
fn external_link_clone_and_eq() {
    let a = sample_external_link();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn external_link_different_url_not_equal() {
    let a = sample_external_link();
    let b = ExternalLink {
        url: "https://other.com".to_string(),
        ..sample_external_link()
    };
    assert_ne!(a, b);
}

// ==========================================================================
// AssetReference -- construction and field access
// ==========================================================================

fn sample_asset_reference() -> AssetReference {
    AssetReference {
        source_note_id: NoteId::from("01HZ3NOTE000000000000000A"),
        raw_path: "./assets/diagram.png".to_string(),
        target_asset_id: Some(AssetId::from("01HZ3ASSET00000000000000A")),
    }
}

#[test]
fn asset_reference_construction() {
    let r = sample_asset_reference();
    assert_eq!(r.source_note_id.to_string(), "01HZ3NOTE000000000000000A");
    assert_eq!(r.raw_path, "./assets/diagram.png");
    assert_eq!(
        r.target_asset_id
            .as_ref()
            .map(std::string::ToString::to_string),
        Some("01HZ3ASSET00000000000000A".to_string())
    );
}

#[test]
fn asset_reference_unresolved() {
    let r = AssetReference {
        target_asset_id: None,
        ..sample_asset_reference()
    };
    assert!(r.target_asset_id.is_none());
}

#[test]
fn asset_reference_clone_and_eq() {
    let a = sample_asset_reference();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn asset_reference_different_path_not_equal() {
    let a = sample_asset_reference();
    let b = AssetReference {
        raw_path: "./assets/photo.jpg".to_string(),
        ..sample_asset_reference()
    };
    assert_ne!(a, b);
}

// ==========================================================================
// ParsedNoteIndex -- construction and field access
// ==========================================================================

fn sample_parsed_note_index() -> ParsedNoteIndex {
    ParsedNoteIndex {
        note_id: NoteId::from("01HZ3NOTE000000000000000A"),
        title: "Example Note".to_string(),
        tags: vec!["project/foo".to_string(), "type/concept".to_string()],
        aliases: vec!["Example Alias".to_string()],
        outbound_note_links: vec![sample_resolved_link()],
        outbound_external_links: vec![sample_external_link()],
        asset_references: vec![sample_asset_reference()],
        has_mermaid: false,
    }
}

#[test]
fn parsed_note_index_construction() {
    let idx = sample_parsed_note_index();
    assert_eq!(idx.note_id.to_string(), "01HZ3NOTE000000000000000A");
    assert_eq!(idx.title, "Example Note");
    assert_eq!(idx.tags, vec!["project/foo", "type/concept"]);
    assert_eq!(idx.aliases, vec!["Example Alias"]);
    assert_eq!(idx.outbound_note_links.len(), 1);
    assert_eq!(idx.outbound_external_links.len(), 1);
    assert_eq!(idx.asset_references.len(), 1);
    assert!(!idx.has_mermaid);
}

#[test]
fn parsed_note_index_with_mermaid() {
    let idx = ParsedNoteIndex {
        has_mermaid: true,
        ..sample_parsed_note_index()
    };
    assert!(idx.has_mermaid);
}

#[test]
fn parsed_note_index_empty_collections() {
    let idx = ParsedNoteIndex {
        tags: vec![],
        aliases: vec![],
        outbound_note_links: vec![],
        outbound_external_links: vec![],
        asset_references: vec![],
        ..sample_parsed_note_index()
    };
    assert!(idx.tags.is_empty());
    assert!(idx.aliases.is_empty());
    assert!(idx.outbound_note_links.is_empty());
    assert!(idx.outbound_external_links.is_empty());
    assert!(idx.asset_references.is_empty());
}

#[test]
fn parsed_note_index_multiple_links() {
    let link_a = sample_resolved_link();
    let link_b = ResolvedNoteLink {
        raw_target: "Another Note".to_string(),
        target_note_id: Some(NoteId::from("01HZ3NOTE000000000000000C")),
        link_text: None,
        is_ambiguous: false,
        ..sample_resolved_link()
    };
    let idx = ParsedNoteIndex {
        outbound_note_links: vec![link_a, link_b],
        ..sample_parsed_note_index()
    };
    assert_eq!(idx.outbound_note_links.len(), 2);
    assert_eq!(idx.outbound_note_links[0].raw_target, "Some Other Note");
    assert_eq!(idx.outbound_note_links[1].raw_target, "Another Note");
}

#[test]
fn parsed_note_index_clone_and_eq() {
    let a = sample_parsed_note_index();
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn parsed_note_index_different_title_not_equal() {
    let a = sample_parsed_note_index();
    let b = ParsedNoteIndex {
        title: "Different Title".to_string(),
        ..sample_parsed_note_index()
    };
    assert_ne!(a, b);
}

// ==========================================================================
// Debug formatting
// ==========================================================================

#[test]
fn all_types_implement_debug() {
    let _ = format!("{:?}", sample_resolved_link());
    let _ = format!("{:?}", sample_external_link());
    let _ = format!("{:?}", sample_asset_reference());
    let _ = format!("{:?}", sample_parsed_note_index());
}
