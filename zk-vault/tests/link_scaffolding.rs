// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_vault::{ClassifiedLink, ExtractedLinks, RawLink};

#[test]
fn raw_link_variants_construct_and_compare() {
    let wiki_link = RawLink::WikiLink {
        raw_target: "Note Title".to_string(),
        alias: Some("Alias Text".to_string()),
    };
    let markdown_link = RawLink::MarkdownLink {
        text: "Docs".to_string(),
        destination: "https://example.com".to_string(),
    };

    assert_eq!(
        wiki_link,
        RawLink::WikiLink {
            raw_target: "Note Title".to_string(),
            alias: Some("Alias Text".to_string()),
        }
    );
    assert_ne!(
        wiki_link,
        RawLink::WikiLink {
            raw_target: "Different Note".to_string(),
            alias: Some("Alias Text".to_string()),
        }
    );
    assert_eq!(
        markdown_link,
        RawLink::MarkdownLink {
            text: "Docs".to_string(),
            destination: "https://example.com".to_string(),
        }
    );
    assert_ne!(
        markdown_link,
        RawLink::MarkdownLink {
            text: "Docs".to_string(),
            destination: "https://other.example.com".to_string(),
        }
    );
}

#[test]
fn extracted_links_default_is_empty() {
    let extracted = ExtractedLinks::default();

    assert!(extracted.wiki_links.is_empty());
    assert!(extracted.markdown_links.is_empty());
}

#[test]
fn classified_link_variants_construct_and_compare() {
    let note_link = ClassifiedLink::NoteLink {
        raw_target: "Note Title".to_string(),
        link_text: Some("Display".to_string()),
    };
    let external_url = ClassifiedLink::ExternalUrl {
        url: "https://example.com".to_string(),
        link_text: Some("Docs".to_string()),
    };
    let asset_path = ClassifiedLink::AssetPath {
        raw_path: "./assets/diagram.png".to_string(),
        link_text: Some("Diagram".to_string()),
    };
    let local_note_path_link = ClassifiedLink::LocalNotePathLink {
        raw_path: "./notes/other.md".to_string(),
        link_text: Some("Other".to_string()),
    };

    assert_eq!(
        note_link,
        ClassifiedLink::NoteLink {
            raw_target: "Note Title".to_string(),
            link_text: Some("Display".to_string()),
        }
    );
    assert_ne!(
        note_link,
        ClassifiedLink::NoteLink {
            raw_target: "Different Title".to_string(),
            link_text: Some("Display".to_string()),
        }
    );

    assert_eq!(
        external_url,
        ClassifiedLink::ExternalUrl {
            url: "https://example.com".to_string(),
            link_text: Some("Docs".to_string()),
        }
    );
    assert_eq!(
        asset_path,
        ClassifiedLink::AssetPath {
            raw_path: "./assets/diagram.png".to_string(),
            link_text: Some("Diagram".to_string()),
        }
    );
    assert_eq!(
        local_note_path_link,
        ClassifiedLink::LocalNotePathLink {
            raw_path: "./notes/other.md".to_string(),
            link_text: Some("Other".to_string()),
        }
    );
}
