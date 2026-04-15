//! Internal link-extraction types for the raw body-scanning pass.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawLink {
    WikiLink {
        raw_target: String,
        alias: Option<String>,
    },
    MarkdownLink {
        text: String,
        destination: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractedLinks {
    pub wiki_links: Vec<RawLink>,
    pub markdown_links: Vec<RawLink>,
}
