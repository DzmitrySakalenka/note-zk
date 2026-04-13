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

fn split_frontmatter_sections(file_content: &str) -> Option<(&str, &str)> {
    const OPENING_DELIMITER: &str = "---\n";
    if !file_content.starts_with(OPENING_DELIMITER) {
        return None;
    }

    let frontmatter_and_body = &file_content[OPENING_DELIMITER.len()..];
    let mut line_start_index = 0usize;

    while line_start_index < frontmatter_and_body.len() {
        let search_space = &frontmatter_and_body[line_start_index..];
        if let Some(relative_newline_index) = search_space.find('\n') {
            let line_end_index = line_start_index + relative_newline_index;
            if frontmatter_and_body[line_start_index..line_end_index] == *"---" {
                let raw_frontmatter = &frontmatter_and_body[..line_start_index];
                let body_markdown = &frontmatter_and_body[line_end_index + 1..];
                return Some((raw_frontmatter, body_markdown));
            }
            line_start_index = line_end_index + 1;
            continue;
        }

        if frontmatter_and_body[line_start_index..] == *"---" {
            let raw_frontmatter = &frontmatter_and_body[..line_start_index];
            return Some((raw_frontmatter, ""));
        }
        break;
    }

    None
}
