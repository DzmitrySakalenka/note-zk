use std::{fs, path::Path};

use zk_domain::note::RawNoteFrontmatter;

use crate::error::VaultError;

#[derive(Debug, Clone, PartialEq)]
pub struct RawNoteDocument {
    pub frontmatter: Option<RawNoteFrontmatter>,
    pub raw_frontmatter_yaml: Option<String>,
    pub body_markdown: String,
    pub has_frontmatter_delimiters: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyAnalysis {
    pub first_h1: Option<String>,
    pub has_mermaid: bool,
}

/// Parses a markdown file into raw frontmatter and body sections.
///
/// # Errors
///
/// Returns [`VaultError::Io`] when file reading fails, [`VaultError::Utf8Decode`]
/// when bytes are not valid UTF-8 text, and [`VaultError::FrontmatterParse`]
/// when a detected frontmatter block contains invalid YAML.
pub fn parse_note_file(path: &Path) -> Result<RawNoteDocument, VaultError> {
    let file_bytes = fs::read(path).map_err(|source| VaultError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_content = String::from_utf8(file_bytes).map_err(|source| VaultError::Utf8Decode {
        path: path.to_path_buf(),
        source,
    })?;

    let Some((raw_yaml, body_markdown)) = split_frontmatter_sections(&file_content) else {
        return Ok(RawNoteDocument {
            frontmatter: None,
            raw_frontmatter_yaml: None,
            body_markdown: file_content,
            has_frontmatter_delimiters: false,
        });
    };

    let parsed_frontmatter =
        serde_yaml::from_str::<RawNoteFrontmatter>(raw_yaml).map_err(|source| {
            VaultError::FrontmatterParse {
                path: path.to_path_buf(),
                source,
            }
        })?;

    Ok(RawNoteDocument {
        frontmatter: Some(parsed_frontmatter),
        raw_frontmatter_yaml: Some(raw_yaml.to_string()),
        body_markdown: body_markdown.to_string(),
        has_frontmatter_delimiters: true,
    })
}

#[must_use]
pub fn extract_first_h1(body: &str) -> Option<String> {
    analyze_body(body).first_h1
}

#[must_use]
pub fn detect_mermaid(body: &str) -> bool {
    analyze_body(body).has_mermaid
}

#[must_use]
pub fn analyze_body(body: &str) -> BodyAnalysis {
    analyze_body_internal(body)
}

fn split_frontmatter_sections(file_content: &str) -> Option<(&str, &str)> {
    let (opening_line, frontmatter_start_index) = split_next_line(file_content)?;
    if !is_frontmatter_delimiter(opening_line) {
        return None;
    }

    let frontmatter_and_body = &file_content[frontmatter_start_index..];
    let mut line_start_index = 0usize;

    while line_start_index < frontmatter_and_body.len() {
        let search_space = &frontmatter_and_body[line_start_index..];
        if let Some((line, next_line_start_index)) = split_next_line(search_space) {
            if is_frontmatter_delimiter(line) {
                let raw_frontmatter = &frontmatter_and_body[..line_start_index];
                let body_markdown = &search_space[next_line_start_index..];
                return Some((raw_frontmatter, body_markdown));
            }
            line_start_index += next_line_start_index;
            continue;
        }

        if is_frontmatter_delimiter(search_space) {
            let raw_frontmatter = &frontmatter_and_body[..line_start_index];
            return Some((raw_frontmatter, ""));
        }
        break;
    }

    None
}

fn split_next_line(content: &str) -> Option<(&str, usize)> {
    let newline_index = content.find('\n')?;
    Some((&content[..newline_index], newline_index + 1))
}

fn is_frontmatter_delimiter(line: &str) -> bool {
    line.strip_suffix('\r').unwrap_or(line) == "---"
}

fn analyze_body_internal(body: &str) -> BodyAnalysis {
    let mut inside_fence = false;
    let mut opening_fence_len = 0usize;
    let mut first_h1 = None;
    let mut has_mermaid = false;

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
            if info == "mermaid" {
                has_mermaid = true;
            }
            continue;
        }

        if inside_fence || first_h1.is_some() {
            continue;
        }

        first_h1 = extract_h1_from_line(line);
    }

    BodyAnalysis {
        first_h1,
        has_mermaid,
    }
}

pub(crate) fn parse_fence_line(line: &str) -> Option<(usize, &str)> {
    let indent_width = line.chars().take_while(|ch| *ch == ' ').count();
    if indent_width > 3 {
        return None;
    }

    let trimmed = &line[indent_width..];
    let backtick_count = trimmed.chars().take_while(|ch| *ch == '`').count();
    if backtick_count < 3 {
        return None;
    }

    let remainder = trimmed[backtick_count..].trim();
    Some((backtick_count, remainder))
}

fn extract_h1_from_line(line: &str) -> Option<String> {
    let indent_width = line.chars().take_while(|ch| *ch == ' ').count();
    if indent_width > 3 {
        return None;
    }

    let candidate = &line[indent_width..];
    if !candidate.starts_with('#') || candidate.starts_with("##") {
        return None;
    }

    let mut heading_chars = candidate[1..].chars();
    let first_after_hash = heading_chars.next()?;
    if !first_after_hash.is_whitespace() {
        return None;
    }
    let heading_tail = heading_chars.as_str();
    Some(heading_tail.trim_end().to_string())
}
