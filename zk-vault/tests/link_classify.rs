use zk_vault::{ClassifiedLink, ExtractedLinks, RawLink, classify_links};

fn note_extensions() -> Vec<String> {
    vec!["md".to_string(), "markdown".to_string()]
}

#[test]
fn classifies_wiki_link_as_note_link() {
    let extracted = ExtractedLinks {
        wiki_links: vec![RawLink::WikiLink {
            raw_target: "Note Title".to_string(),
            alias: Some("Shown Title".to_string()),
        }],
        markdown_links: Vec::new(),
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::NoteLink {
            raw_target: "Note Title".to_string(),
            link_text: Some("Shown Title".to_string()),
        }]
    );
}

#[test]
fn classifies_https_markdown_link_as_external_url() {
    let extracted = ExtractedLinks {
        wiki_links: Vec::new(),
        markdown_links: vec![RawLink::MarkdownLink {
            text: "Docs".to_string(),
            destination: "https://example.com/docs".to_string(),
        }],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::ExternalUrl {
            url: "https://example.com/docs".to_string(),
            link_text: Some("Docs".to_string()),
        }]
    );
}

#[test]
fn classifies_http_scheme_case_insensitively() {
    let extracted = ExtractedLinks {
        wiki_links: Vec::new(),
        markdown_links: vec![RawLink::MarkdownLink {
            text: "Docs".to_string(),
            destination: "HTTP://example.com/docs".to_string(),
        }],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::ExternalUrl {
            url: "HTTP://example.com/docs".to_string(),
            link_text: Some("Docs".to_string()),
        }]
    );
}

#[test]
fn classifies_note_path_with_configured_extension() {
    let extracted = ExtractedLinks {
        wiki_links: Vec::new(),
        markdown_links: vec![RawLink::MarkdownLink {
            text: "Other".to_string(),
            destination: "./relative/path.md".to_string(),
        }],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::LocalNotePathLink {
            raw_path: "./relative/path.md".to_string(),
            link_text: Some("Other".to_string()),
        }]
    );
}

#[test]
fn classifies_note_extension_case_insensitively() {
    let extracted = ExtractedLinks {
        wiki_links: Vec::new(),
        markdown_links: vec![RawLink::MarkdownLink {
            text: "Upper".to_string(),
            destination: "./relative/path.MD".to_string(),
        }],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::LocalNotePathLink {
            raw_path: "./relative/path.MD".to_string(),
            link_text: Some("Upper".to_string()),
        }]
    );
}

#[test]
fn classifies_non_note_relative_path_as_asset() {
    let extracted = ExtractedLinks {
        wiki_links: Vec::new(),
        markdown_links: vec![RawLink::MarkdownLink {
            text: "Image".to_string(),
            destination: "./assets/diagram.png".to_string(),
        }],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::AssetPath {
            raw_path: "./assets/diagram.png".to_string(),
            link_text: Some("Image".to_string()),
        }]
    );
}

#[test]
fn classifies_image_link_output_as_asset() {
    let extracted = ExtractedLinks {
        wiki_links: Vec::new(),
        markdown_links: vec![RawLink::MarkdownLink {
            text: "Diagram".to_string(),
            destination: "./image.png".to_string(),
        }],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::AssetPath {
            raw_path: "./image.png".to_string(),
            link_text: Some("Diagram".to_string()),
        }]
    );
}

#[test]
fn classifies_relative_path_without_extension_as_note_link() {
    let extracted = ExtractedLinks {
        wiki_links: Vec::new(),
        markdown_links: vec![RawLink::MarkdownLink {
            text: "Shorthand".to_string(),
            destination: "./notes/other-note".to_string(),
        }],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![ClassifiedLink::NoteLink {
            raw_target: "./notes/other-note".to_string(),
            link_text: Some("Shorthand".to_string()),
        }]
    );
}

#[test]
fn classifies_mixed_collection_in_deterministic_order() {
    let extracted = ExtractedLinks {
        wiki_links: vec![
            RawLink::WikiLink {
                raw_target: "Alpha".to_string(),
                alias: None,
            },
            RawLink::WikiLink {
                raw_target: "Beta".to_string(),
                alias: Some("Shown Beta".to_string()),
            },
        ],
        markdown_links: vec![
            RawLink::MarkdownLink {
                text: "Site".to_string(),
                destination: "https://example.com".to_string(),
            },
            RawLink::MarkdownLink {
                text: "Note".to_string(),
                destination: "./gamma.markdown".to_string(),
            },
            RawLink::MarkdownLink {
                text: "Asset".to_string(),
                destination: "./diagram.svg".to_string(),
            },
            RawLink::MarkdownLink {
                text: "Shorthand".to_string(),
                destination: "./delta".to_string(),
            },
        ],
    };

    let classified = classify_links(&extracted, &note_extensions());

    assert_eq!(
        classified,
        vec![
            ClassifiedLink::NoteLink {
                raw_target: "Alpha".to_string(),
                link_text: None,
            },
            ClassifiedLink::NoteLink {
                raw_target: "Beta".to_string(),
                link_text: Some("Shown Beta".to_string()),
            },
            ClassifiedLink::ExternalUrl {
                url: "https://example.com".to_string(),
                link_text: Some("Site".to_string()),
            },
            ClassifiedLink::LocalNotePathLink {
                raw_path: "./gamma.markdown".to_string(),
                link_text: Some("Note".to_string()),
            },
            ClassifiedLink::AssetPath {
                raw_path: "./diagram.svg".to_string(),
                link_text: Some("Asset".to_string()),
            },
            ClassifiedLink::NoteLink {
                raw_target: "./delta".to_string(),
                link_text: Some("Shorthand".to_string()),
            },
        ]
    );
}
