//! Internal link-extraction types for the raw body-scanning pass.

use crate::parse::parse_fence_line;

/// A single link extracted from markdown body text before classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawLink {
    /// `[[target]]` or `[[target|alias]]` wiki-style link.
    WikiLink {
        raw_target: String,
        alias: Option<String>,
    },
    /// Standard `[text](destination)` markdown link.
    MarkdownLink { text: String, destination: String },
}

/// Accumulator for links extracted from a note body, separated by syntax type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractedLinks {
    pub wiki_links: Vec<RawLink>,
    pub markdown_links: Vec<RawLink>,
}

#[must_use]
pub fn extract_links(body: &str) -> ExtractedLinks {
    let mut inside_fence = false;
    let mut opening_fence_len = 0usize;
    let mut extracted = ExtractedLinks::default();

    for line in body.lines() {
        if let Some((backtick_count, info)) = parse_fence_line(line) {
            if inside_fence {
                if backtick_count >= opening_fence_len && info.is_empty() {
                    inside_fence = false;
                    opening_fence_len = 0;
                }
                continue;
            }

            inside_fence = true;
            opening_fence_len = backtick_count;
            continue;
        }

        if inside_fence {
            continue;
        }

        extract_links_from_line(line, &mut extracted);
    }

    extracted
}

// ==============================================================================
// Inline Link Extraction State Machine
// ==============================================================================
//
// Scans a single line left-to-right, tracking inline code spans so that links
// inside backtick-delimited code are ignored. The state machine has two layers:
//
// 1. **Backtick tracking** -- counts consecutive backticks to handle both single
//    (`code`) and multi-backtick (``code``) spans. An opening run of N backticks
//    is only closed by exactly N closing backticks, matching CommonMark rules.
//
// 2. **Link dispatch** -- outside code spans, `[[` triggers wiki link parsing
//    (checked first since it is a superset of `[`), then bare `[` triggers
//    standard markdown link parsing. Both parsers return the index past the
//    consumed link on success, or the original index on failure (no match).

fn extract_links_from_line(line: &str, extracted: &mut ExtractedLinks) {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    let mut inline_code_delimiter = None;

    while index < bytes.len() {
        // Track inline code spans: match opening/closing backtick runs by count
        let backtick_count = count_repeated_byte(bytes, index, b'`');
        if backtick_count > 0 {
            match inline_code_delimiter {
                Some(opening_len) if backtick_count == opening_len => inline_code_delimiter = None,
                Some(_) => {}
                None => inline_code_delimiter = Some(backtick_count),
            }
            index += backtick_count;
            continue;
        }

        // Inside a code span -- skip all non-backtick bytes
        if inline_code_delimiter.is_some() {
            index += 1;
            continue;
        }

        // Try wiki link first: `[[` is a prefix of `[`, so it must be checked
        // before the standard markdown link branch
        if bytes[index] == b'[' && line[index..].starts_with("[[") {
            let next_index = parse_wiki_link(line, index, bytes, extracted);
            if next_index != index {
                index = next_index;
                continue;
            }
        }

        if bytes[index] == b'[' {
            let next_index = parse_markdown_link(line, index, extracted);
            if next_index != index {
                index = next_index;
                continue;
            }
        }

        index += 1;
    }
}

fn parse_wiki_link(
    line: &str,
    opening_index: usize,
    bytes: &[u8],
    extracted: &mut ExtractedLinks,
) -> usize {
    let Some(relative_closing_index) = line[opening_index + 2..].find("]]") else {
        return opening_index;
    };
    let closing_index = opening_index + 2 + relative_closing_index;

    if opening_index > 0 && bytes[opening_index - 1] == b'!' {
        return closing_index + 2;
    }

    let raw_content = &line[opening_index + 2..closing_index];
    let (raw_target, alias) = split_wiki_target_and_alias(raw_content);
    if raw_target.is_empty() || raw_target.contains('#') {
        return closing_index + 2;
    }

    extracted.wiki_links.push(RawLink::WikiLink {
        raw_target: raw_target.to_string(),
        alias,
    });
    closing_index + 2
}

fn parse_markdown_link(line: &str, opening_index: usize, extracted: &mut ExtractedLinks) -> usize {
    let Some(relative_label_end) = line[opening_index + 1..].find(']') else {
        return opening_index;
    };
    let label_end = opening_index + 1 + relative_label_end;
    let after_label = label_end + 1;

    if line
        .as_bytes()
        .get(after_label)
        .is_none_or(|byte| *byte != b'(')
    {
        return opening_index;
    }

    let Some(relative_destination_end) = line[after_label + 1..].find(')') else {
        return opening_index;
    };
    let destination_end = after_label + 1 + relative_destination_end;
    let destination = &line[after_label + 1..destination_end];
    if destination.is_empty() {
        return destination_end + 1;
    }

    extracted.markdown_links.push(RawLink::MarkdownLink {
        text: line[opening_index + 1..label_end].to_string(),
        destination: destination.to_string(),
    });
    destination_end + 1
}

fn split_wiki_target_and_alias(raw_content: &str) -> (&str, Option<String>) {
    let Some((raw_target, raw_alias)) = raw_content.split_once('|') else {
        let trimmed_target = raw_content.trim();
        return (trimmed_target, None);
    };

    let trimmed_target = raw_target.trim();
    let trimmed_alias = raw_alias.trim();
    let alias = if trimmed_alias.is_empty() {
        None
    } else {
        Some(trimmed_alias.to_string())
    };

    (trimmed_target, alias)
}

fn count_repeated_byte(bytes: &[u8], start: usize, needle: u8) -> usize {
    let mut count = 0usize;
    while bytes.get(start + count).is_some_and(|byte| *byte == needle) {
        count += 1;
    }
    count
}
