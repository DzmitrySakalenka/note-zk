pub(crate) mod ambiguous_link;
pub(crate) mod broken_asset_ref;
pub(crate) mod default_untagged_tag;
pub(crate) mod duplicate_id;
pub(crate) mod empty_body;
pub(crate) mod frontmatter_inserted;
pub(crate) mod id_format_suspicious;
pub(crate) mod id_injected;
pub(crate) mod invalid_frontmatter;
pub(crate) mod mermaid_block_detected;
pub(crate) mod missing_title;
pub(crate) mod no_h1_in_body;
pub(crate) mod suspicious_path_naming;
pub(crate) mod title_derived_from_filename;
pub(crate) mod title_derived_from_h1;
pub(crate) mod title_h1_mismatch;
pub(crate) mod trashed_unknown_identity;
pub(crate) mod unresolved_link;
pub(crate) mod wiki_index_outside_root;
pub(crate) mod wiki_source_missing;
pub(crate) mod wiki_unknown_type;

use super::ValidationRule;

static INVALID_FRONTMATTER: invalid_frontmatter::Rule = invalid_frontmatter::Rule;
static MISSING_TITLE: missing_title::Rule = missing_title::Rule;
static DUPLICATE_ID: duplicate_id::Rule = duplicate_id::Rule;
static UNRESOLVED_LINK: unresolved_link::Rule = unresolved_link::Rule;
static AMBIGUOUS_LINK: ambiguous_link::Rule = ambiguous_link::Rule;
static BROKEN_ASSET_REF: broken_asset_ref::Rule = broken_asset_ref::Rule;
static EMPTY_BODY: empty_body::Rule = empty_body::Rule;
static NO_H1_IN_BODY: no_h1_in_body::Rule = no_h1_in_body::Rule;
static TITLE_H1_MISMATCH: title_h1_mismatch::Rule = title_h1_mismatch::Rule;
static SUSPICIOUS_PATH_NAMING: suspicious_path_naming::Rule = suspicious_path_naming::Rule;
static ID_FORMAT_SUSPICIOUS: id_format_suspicious::Rule = id_format_suspicious::Rule;
static TITLE_DERIVED_FROM_H1: title_derived_from_h1::Rule = title_derived_from_h1::Rule;
static TITLE_DERIVED_FROM_FILENAME: title_derived_from_filename::Rule =
    title_derived_from_filename::Rule;
static ID_INJECTED: id_injected::Rule = id_injected::Rule;
static DEFAULT_UNTAGGED_TAG: default_untagged_tag::Rule = default_untagged_tag::Rule;
static FRONTMATTER_INSERTED: frontmatter_inserted::Rule = frontmatter_inserted::Rule;
static MERMAID_BLOCK_DETECTED: mermaid_block_detected::Rule = mermaid_block_detected::Rule;
static WIKI_UNKNOWN_TYPE: wiki_unknown_type::Rule = wiki_unknown_type::Rule;
static WIKI_SOURCE_MISSING: wiki_source_missing::Rule = wiki_source_missing::Rule;
static WIKI_INDEX_OUTSIDE_ROOT: wiki_index_outside_root::Rule = wiki_index_outside_root::Rule;

pub(crate) static ALL: &[&dyn ValidationRule] = &[
    &INVALID_FRONTMATTER,
    &MISSING_TITLE,
    &DUPLICATE_ID,
    &UNRESOLVED_LINK,
    &AMBIGUOUS_LINK,
    &BROKEN_ASSET_REF,
    &EMPTY_BODY,
    &NO_H1_IN_BODY,
    &TITLE_H1_MISMATCH,
    &SUSPICIOUS_PATH_NAMING,
    &ID_FORMAT_SUSPICIOUS,
    &TITLE_DERIVED_FROM_H1,
    &TITLE_DERIVED_FROM_FILENAME,
    &ID_INJECTED,
    &DEFAULT_UNTAGGED_TAG,
    &FRONTMATTER_INSERTED,
    &MERMAID_BLOCK_DETECTED,
    &WIKI_UNKNOWN_TYPE,
    &WIKI_SOURCE_MISSING,
    &WIKI_INDEX_OUTSIDE_ROOT,
];
