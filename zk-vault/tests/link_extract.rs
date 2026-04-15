use zk_vault::{ExtractedLinks, RawLink, extract_links};

#[test]
fn extracts_plain_wiki_link() {
    let extracted = extract_links("See [[Note Title]] for details.");

    assert_eq!(
        extracted,
        ExtractedLinks {
            wiki_links: vec![RawLink::WikiLink {
                raw_target: "Note Title".to_string(),
                alias: None,
            }],
            markdown_links: Vec::new(),
        }
    );
}

#[test]
fn extracts_wiki_link_with_alias_and_trims_target_whitespace() {
    let extracted = extract_links("See [[  path/to/note  |Alias Text]] for details.");

    assert_eq!(
        extracted,
        ExtractedLinks {
            wiki_links: vec![RawLink::WikiLink {
                raw_target: "path/to/note".to_string(),
                alias: Some("Alias Text".to_string()),
            }],
            markdown_links: Vec::new(),
        }
    );
}

#[test]
fn extracts_wiki_path_without_alias() {
    let extracted = extract_links("[[projects/example-note]]");

    assert_eq!(
        extracted.wiki_links,
        vec![RawLink::WikiLink {
            raw_target: "projects/example-note".to_string(),
            alias: None,
        }]
    );
    assert!(extracted.markdown_links.is_empty());
}

#[test]
fn extracts_wiki_path_with_alias() {
    let extracted = extract_links("[[projects/example-note|Project Note]]");

    assert_eq!(
        extracted.wiki_links,
        vec![RawLink::WikiLink {
            raw_target: "projects/example-note".to_string(),
            alias: Some("Project Note".to_string()),
        }]
    );
    assert!(extracted.markdown_links.is_empty());
}

#[test]
fn extracts_external_markdown_link() {
    let extracted = extract_links("Read [Docs](https://example.com).");

    assert_eq!(
        extracted.markdown_links,
        vec![RawLink::MarkdownLink {
            text: "Docs".to_string(),
            destination: "https://example.com".to_string(),
        }]
    );
    assert!(extracted.wiki_links.is_empty());
}

#[test]
fn extracts_relative_markdown_link() {
    let extracted = extract_links("Read [Other](./relative/path.md).");

    assert_eq!(
        extracted.markdown_links,
        vec![RawLink::MarkdownLink {
            text: "Other".to_string(),
            destination: "./relative/path.md".to_string(),
        }]
    );
}

#[test]
fn extracts_markdown_image_link() {
    let extracted = extract_links("![Diagram](./image.png)");

    assert_eq!(
        extracted.markdown_links,
        vec![RawLink::MarkdownLink {
            text: "Diagram".to_string(),
            destination: "./image.png".to_string(),
        }]
    );
}

#[test]
fn skips_links_inside_fenced_code_blocks() {
    let body = "\
Before [[Visible]]
```md
[[Hidden]]
[Hidden](https://example.com)
```
After [Visible Link](./visible.md)
";
    let extracted = extract_links(body);

    assert_eq!(
        extracted,
        ExtractedLinks {
            wiki_links: vec![RawLink::WikiLink {
                raw_target: "Visible".to_string(),
                alias: None,
            }],
            markdown_links: vec![RawLink::MarkdownLink {
                text: "Visible Link".to_string(),
                destination: "./visible.md".to_string(),
            }],
        }
    );
}

#[test]
fn skips_links_inside_inline_code_spans() {
    let body = "\
Outside [[Visible]] and `[[Hidden]] [Skip](https://example.com)` and [Shown](./shown.md)
";
    let extracted = extract_links(body);

    assert_eq!(
        extracted,
        ExtractedLinks {
            wiki_links: vec![RawLink::WikiLink {
                raw_target: "Visible".to_string(),
                alias: None,
            }],
            markdown_links: vec![RawLink::MarkdownLink {
                text: "Shown".to_string(),
                destination: "./shown.md".to_string(),
            }],
        }
    );
}

#[test]
fn extracts_multiple_links_on_the_same_line() {
    let extracted = extract_links(
        "Links: [[One]] [[Two|Second]] [Three](https://example.com) ![Four](./four.png)",
    );

    assert_eq!(
        extracted,
        ExtractedLinks {
            wiki_links: vec![
                RawLink::WikiLink {
                    raw_target: "One".to_string(),
                    alias: None,
                },
                RawLink::WikiLink {
                    raw_target: "Two".to_string(),
                    alias: Some("Second".to_string()),
                },
            ],
            markdown_links: vec![
                RawLink::MarkdownLink {
                    text: "Three".to_string(),
                    destination: "https://example.com".to_string(),
                },
                RawLink::MarkdownLink {
                    text: "Four".to_string(),
                    destination: "./four.png".to_string(),
                },
            ],
        }
    );
}

#[test]
fn extracts_mixed_wiki_and_markdown_links() {
    let body = "\
[[Alpha]]
[Beta](./beta.md)
[[Gamma|Shown Gamma]]
![Delta](./delta.png)
";
    let extracted = extract_links(body);

    assert_eq!(
        extracted,
        ExtractedLinks {
            wiki_links: vec![
                RawLink::WikiLink {
                    raw_target: "Alpha".to_string(),
                    alias: None,
                },
                RawLink::WikiLink {
                    raw_target: "Gamma".to_string(),
                    alias: Some("Shown Gamma".to_string()),
                },
            ],
            markdown_links: vec![
                RawLink::MarkdownLink {
                    text: "Beta".to_string(),
                    destination: "./beta.md".to_string(),
                },
                RawLink::MarkdownLink {
                    text: "Delta".to_string(),
                    destination: "./delta.png".to_string(),
                },
            ],
        }
    );
}

#[test]
fn ignores_empty_targets_and_destinations() {
    let extracted = extract_links("[[]] [Text]() [[  |Alias]]");

    assert_eq!(extracted, ExtractedLinks::default());
}

#[test]
fn empty_body_returns_no_links() {
    assert_eq!(extract_links(""), ExtractedLinks::default());
}

#[test]
fn ignores_unclosed_brackets_without_panicking() {
    let extracted = extract_links("[[Unclosed [Text](https://example.com");

    assert_eq!(extracted, ExtractedLinks::default());
}

#[test]
fn ignores_unsupported_wiki_forms() {
    let extracted = extract_links("![[embed-note]] [[note#heading]] [[regular-note]]");

    assert_eq!(
        extracted.wiki_links,
        vec![RawLink::WikiLink {
            raw_target: "regular-note".to_string(),
            alias: None,
        }]
    );
    assert!(extracted.markdown_links.is_empty());
}
