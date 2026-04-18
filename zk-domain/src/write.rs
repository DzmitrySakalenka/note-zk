use serde::{Deserialize, Serialize};

use crate::{
    id::{NoteId, RelativePath},
    note::NoteFrontmatter,
};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Request payload for creating a note at a specific vault-relative path.
pub struct CreateNoteRequest {
    /// Vault-relative target path for the new note file.
    pub path: RelativePath,
    /// Frontmatter to write into the created note.
    pub frontmatter: NoteFrontmatter,
    /// Markdown body content for the created note.
    pub body_markdown: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Request payload for updating an existing note by stable note ID.
pub struct UpdateNoteRequest {
    /// Stable identifier for the note to update.
    pub note_id: NoteId,
    /// Optional replacement frontmatter.
    pub frontmatter: Option<NoteFrontmatter>,
    /// Optional replacement markdown body.
    pub body_markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// Request payload for applying create and update operations in one batch.
pub struct BatchWriteRequest {
    /// Create operations to apply in order.
    pub creates: Vec<CreateNoteRequest>,
    /// Update operations to apply in order.
    pub updates: Vec<UpdateNoteRequest>,
}
