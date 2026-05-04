//! Paragraph and heading-aware note chunker.

use chrono::Utc;
use zk_domain::{
    ChunkId, ChunkRecord,
    id::{ContentHash, NoteId},
};

use crate::{
    AiError, AiResult,
    chunker::{Chunker, ChunkerOptions},
};

/// Paragraph and heading-aware chunker for V1 embedding recomputation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphChunker {
    /// Chunking options.
    pub opts: ChunkerOptions,
}

impl ParagraphChunker {
    /// Creates a paragraph chunker with explicit options.
    #[must_use]
    pub fn new(opts: ChunkerOptions) -> Self {
        Self { opts }
    }
}

impl Default for ParagraphChunker {
    fn default() -> Self {
        Self::new(ChunkerOptions::default())
    }
}

impl Chunker for ParagraphChunker {
    fn chunk(&self, note_id: &NoteId, body: &str) -> AiResult<Vec<ChunkRecord>> {
        if self.opts.max_tokens == 0 {
            return Err(AiError::InvalidConfig(
                "max_tokens must be greater than zero".to_string(),
            ));
        }

        if body.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut chunk_texts = Vec::new();

        for section in split_sections(body, self.opts.respect_headings) {
            let units = split_section_units(&section, self.opts.max_tokens);
            chunk_texts.extend(pack_units(&units, &self.opts));
        }

        Ok(chunk_texts
            .into_iter()
            .enumerate()
            .map(|(ordinal, text)| build_chunk(note_id, ordinal, text))
            .collect())
    }
}

fn build_chunk(note_id: &NoteId, ordinal: usize, text: String) -> ChunkRecord {
    ChunkRecord {
        chunk_id: ChunkId::generate(),
        note_id: note_id.clone(),
        ordinal: u32::try_from(ordinal).unwrap_or(u32::MAX),
        content_hash: ContentHash::from_bytes(text.as_bytes()),
        token_count: Some(estimate_tokens(&text)),
        text,
        created_at: Utc::now(),
    }
}

fn split_sections(body: &str, respect_headings: bool) -> Vec<String> {
    if !respect_headings {
        return vec![body.trim().to_string()];
    }

    let mut sections = Vec::new();
    let mut current = Vec::new();

    for line in body.lines() {
        if line.starts_with('#') && !current.is_empty() {
            push_joined_lines(&mut sections, &mut current);
        }

        current.push(line);
    }

    push_joined_lines(&mut sections, &mut current);
    sections
}

fn push_joined_lines(sections: &mut Vec<String>, lines: &mut Vec<&str>) {
    let section = lines.join("\n").trim().to_string();

    if !section.is_empty() {
        sections.push(section);
    }

    lines.clear();
}

fn split_section_units(section: &str, max_tokens: u32) -> Vec<String> {
    split_paragraphs(section)
        .into_iter()
        .flat_map(|paragraph| {
            if estimate_tokens(&paragraph) > max_tokens {
                split_oversized_paragraph(&paragraph, max_tokens)
            } else {
                vec![paragraph]
            }
        })
        .collect()
}

fn split_paragraphs(section: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();

    for line in section.lines() {
        if line.trim().is_empty() {
            push_joined_lines(&mut paragraphs, &mut current);
            continue;
        }

        current.push(line);
    }

    push_joined_lines(&mut paragraphs, &mut current);
    paragraphs
}

fn split_oversized_paragraph(paragraph: &str, max_tokens: u32) -> Vec<String> {
    split_sentence_units(paragraph)
        .into_iter()
        .flat_map(|unit| {
            if estimate_tokens(&unit) > max_tokens {
                split_by_word_budget(&unit, max_tokens)
            } else {
                vec![unit]
            }
        })
        .collect()
}

fn split_sentence_units(text: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();

    while let Some((index, character)) = chars.next() {
        let end = index + character.len_utf8();
        let is_sentence_boundary = matches!(character, '.' | '!' | '?')
            && chars
                .peek()
                .is_some_and(|(_, next_character)| next_character.is_whitespace());
        let is_newline_boundary = character == '\n';

        if is_sentence_boundary || is_newline_boundary {
            push_trimmed_slice(&mut units, &text[start..end]);
            start = skip_following_whitespace(text, &mut chars, end);
        }
    }

    push_trimmed_slice(&mut units, &text[start..]);
    units
}

fn skip_following_whitespace(
    text: &str,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    current_start: usize,
) -> usize {
    let mut next_start = current_start;

    while let Some((index, character)) = chars.peek().copied() {
        if !character.is_whitespace() {
            break;
        }

        chars.next();
        next_start = index + character.len_utf8();
    }

    next_start.min(text.len())
}

fn push_trimmed_slice(units: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();

    if !trimmed.is_empty() {
        units.push(trimmed.to_string());
    }
}

fn split_by_word_budget(text: &str, max_tokens: u32) -> Vec<String> {
    let word_budget = word_budget_for_tokens(max_tokens);
    let words = text.split_whitespace().collect::<Vec<_>>();

    words
        .chunks(word_budget)
        .map(|chunk| chunk.join(" "))
        .collect()
}

fn pack_units(units: &[String], opts: &ChunkerOptions) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let overlap_tokens = opts.overlap_tokens.min(opts.max_tokens.saturating_sub(1));

    for unit in units {
        if current.is_empty() {
            current.push(unit.clone());
            continue;
        }

        let candidate = join_units_with_paragraph_breaks(&current, Some(unit.as_str()));

        if estimate_tokens(&candidate) <= opts.max_tokens {
            current.push(unit.clone());
            continue;
        }

        let emitted = join_units_with_paragraph_breaks(&current, None);
        let overlap = tail_text_by_token_budget(&emitted, overlap_tokens);
        chunks.push(emitted);

        current.clear();

        if let Some(overlap_text) = overlap {
            let overlapped = join_units_with_paragraph_breaks(
                std::slice::from_ref(&overlap_text),
                Some(unit.as_str()),
            );

            if estimate_tokens(&overlapped) <= opts.max_tokens {
                current.push(overlap_text);
            }
        }

        current.push(unit.clone());
    }

    if !current.is_empty() {
        chunks.push(join_units_with_paragraph_breaks(&current, None));
    }

    chunks
}

fn join_units_with_paragraph_breaks(units: &[String], extra: Option<&str>) -> String {
    let mut text = units.join("\n\n");

    if let Some(extra_unit) = extra {
        if !text.is_empty() {
            text.push_str("\n\n");
        }

        text.push_str(extra_unit);
    }

    text
}

fn tail_text_by_token_budget(text: &str, token_budget: u32) -> Option<String> {
    if token_budget == 0 {
        return None;
    }

    let word_budget = word_budget_for_tokens(token_budget);
    let words = text.split_whitespace().collect::<Vec<_>>();

    if words.is_empty() {
        return None;
    }

    let start = words.len().saturating_sub(word_budget);
    Some(words[start..].join(" "))
}

fn estimate_tokens(text: &str) -> u32 {
    let word_count = text.split_whitespace().count();
    let estimated = word_count.saturating_mul(4) / 3;

    u32::try_from(estimated).unwrap_or(u32::MAX)
}

fn word_budget_for_tokens(max_tokens: u32) -> usize {
    let word_budget = (u64::from(max_tokens).saturating_mul(3) / 4).max(1);

    usize::try_from(word_budget).unwrap_or(usize::MAX)
}
