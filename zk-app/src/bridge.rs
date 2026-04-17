use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use zk_domain::{
    derived::ParsedNoteIndex,
    id::{ContentHash, NoteId},
};
use zk_index::{AssetInput, NoteInput};
use zk_vault::VaultScanResult;

use crate::{AppError, Result};

const MIME_TYPES_BY_EXTENSION: [(&str, &str); 10] = [
    ("gif", "image/gif"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("txt", "text/plain"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
];

/// Builds vault-independent note inputs for index rebuild.
///
/// # Errors
///
/// Returns [`AppError::FrontmatterSerialization`] when frontmatter JSON
/// serialization fails.
pub fn build_note_inputs(
    scan_result: &VaultScanResult,
    parsed_indexes: &[ParsedNoteIndex],
) -> Result<Vec<NoteInput>> {
    let duplicate_note_ids: HashSet<NoteId> = scan_result
        .duplicate_ids
        .iter()
        .map(|(note_id, _)| note_id.clone())
        .collect();
    let parsed_indexes_by_note_id: HashMap<NoteId, &ParsedNoteIndex> = parsed_indexes
        .iter()
        .map(|parsed_index| (parsed_index.note_id.clone(), parsed_index))
        .collect();
    let mut note_inputs = Vec::with_capacity(scan_result.notes.len());

    for note in &scan_result.notes {
        let Some(document) = &note.document else {
            continue;
        };
        if duplicate_note_ids.contains(&document.frontmatter.id) {
            continue;
        }

        let parsed_index = parsed_indexes_by_note_id.get(&document.frontmatter.id);
        let frontmatter_json = serde_json::to_string(&document.frontmatter)
            .map_err(|source| AppError::FrontmatterSerialization { source })?;

        note_inputs.push(NoteInput {
            note_id: document.frontmatter.id.clone(),
            path: note.relative_path.clone(),
            title: document.frontmatter.title.clone(),
            content_hash: note.content_hash.clone(),
            frontmatter_json,
            created_at: document.frontmatter.created.clone().unwrap_or_default(),
            updated_at: document.frontmatter.updated.clone().unwrap_or_default(),
            tags: document.frontmatter.tags.clone(),
            aliases: document.frontmatter.aliases.clone(),
            note_links: parsed_index.map_or_else(Vec::new, |parsed_index| {
                parsed_index.outbound_note_links.clone()
            }),
            external_links: parsed_index.map_or_else(Vec::new, |parsed_index| {
                parsed_index.outbound_external_links.clone()
            }),
            asset_references: parsed_index.map_or_else(Vec::new, |parsed_index| {
                parsed_index.asset_references.clone()
            }),
        });
    }

    Ok(note_inputs)
}

/// Builds vault-independent asset inputs for index rebuild.
///
/// # Errors
///
/// Returns [`AppError::Io`] when asset reads or metadata lookups fail.
pub fn build_asset_inputs(
    scan_result: &VaultScanResult,
    vault_root: &Path,
) -> Result<Vec<AssetInput>> {
    let mut asset_inputs = Vec::with_capacity(scan_result.assets.len());

    for asset in &scan_result.assets {
        let asset_path = vault_root.join(asset.relative_path.as_str());
        let bytes = fs::read(&asset_path).map_err(|source| AppError::Io {
            path: asset_path.clone(),
            source,
        })?;
        let metadata = fs::metadata(&asset_path).map_err(|source| AppError::Io {
            path: asset_path.clone(),
            source,
        })?;

        asset_inputs.push(AssetInput {
            path: asset.relative_path.clone(),
            content_hash: ContentHash::from_bytes(&bytes),
            mime_type: mime_type_for_path(&asset_path),
            size_bytes: metadata.len(),
        });
    }

    Ok(asset_inputs)
}

fn mime_type_for_path(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?;
    let lowercase_extension = extension.to_ascii_lowercase();

    MIME_TYPES_BY_EXTENSION
        .iter()
        .find_map(|(candidate_extension, mime_type)| {
            (*candidate_extension == lowercase_extension).then(|| (*mime_type).to_string())
        })
}
