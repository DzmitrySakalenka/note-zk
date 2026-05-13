use rusqlite::Connection;
use zk_domain::id::NoteId;
use zk_index::SourcesRepo;

use crate::Result as AppResult;

/// Lists source notes that contributed to one wiki page.
///
/// # Errors
///
/// Returns [`crate::AppError`] if the local index query fails.
pub fn list_sources_for_note(conn: &Connection, note_id: &NoteId) -> AppResult<Vec<NoteId>> {
    Ok(SourcesRepo::list_sources_for_note(conn, note_id)?)
}

/// Lists wiki pages that cite one source note.
///
/// # Errors
///
/// Returns [`crate::AppError`] if the local index query fails.
pub fn list_pages_using_source(
    conn: &Connection,
    source_note_id: &NoteId,
) -> AppResult<Vec<NoteId>> {
    Ok(SourcesRepo::list_pages_using_source(conn, source_note_id)?)
}
