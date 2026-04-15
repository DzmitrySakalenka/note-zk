//! Internal link-classification and resolution types for the vault-aware pass.

use crate::link_extract::{ExtractedLinks, RawLink};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedLink {
    NoteLink {
        raw_target: String,
        link_text: Option<String>,
    },
    ExternalUrl {
        url: String,
        link_text: Option<String>,
    },
    AssetPath {
        raw_path: String,
        link_text: Option<String>,
    },
    LocalNotePathLink {
        raw_path: String,
        link_text: Option<String>,
    },
}

#[must_use]
pub fn classify_links(
    extracted: &ExtractedLinks,
    note_extensions: &[String],
) -> Vec<ClassifiedLink> {
    let mut classified =
        Vec::with_capacity(extracted.wiki_links.len() + extracted.markdown_links.len());

    for link in &extracted.wiki_links {
        let RawLink::WikiLink { raw_target, alias } = link else {
            continue;
        };
        classified.push(ClassifiedLink::NoteLink {
            raw_target: raw_target.clone(),
            link_text: alias.clone(),
        });
    }

    for link in &extracted.markdown_links {
        let RawLink::MarkdownLink { text, destination } = link else {
            continue;
        };
        classified.push(classify_markdown_link(destination, text, note_extensions));
    }

    classified
}

fn classify_markdown_link(
    destination: &str,
    text: &str,
    note_extensions: &[String],
) -> ClassifiedLink {
    let link_text = Some(text.to_string());

    if has_http_scheme(destination) {
        return ClassifiedLink::ExternalUrl {
            url: destination.to_string(),
            link_text,
        };
    }

    if let Some(extension) = path_extension(destination) {
        if is_note_extension(extension, note_extensions) {
            return ClassifiedLink::LocalNotePathLink {
                raw_path: destination.to_string(),
                link_text,
            };
        }

        return ClassifiedLink::AssetPath {
            raw_path: destination.to_string(),
            link_text,
        };
    }

    ClassifiedLink::NoteLink {
        raw_target: destination.to_string(),
        link_text,
    }
}

fn has_http_scheme(destination: &str) -> bool {
    let lowercased = destination.to_ascii_lowercase();
    lowercased.starts_with("http://") || lowercased.starts_with("https://")
}

fn path_extension(destination: &str) -> Option<&str> {
    let path_segment = destination.rsplit('/').next().unwrap_or(destination);
    let extension_start = path_segment.rfind('.')?;
    let extension = &path_segment[extension_start + 1..];

    if extension.is_empty() {
        return None;
    }

    Some(extension)
}

fn is_note_extension(extension: &str, note_extensions: &[String]) -> bool {
    note_extensions.iter().any(|allowed_extension| {
        let normalized = allowed_extension.trim_start_matches('.');
        normalized.eq_ignore_ascii_case(extension)
    })
}
