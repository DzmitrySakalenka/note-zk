//! DTO bridge helpers for sync HTTP payloads.

pub(super) fn note_row_from_push_item(
    item: &zk_domain::PushNoteItem,
    existing: Option<&crate::repo::NoteRow>,
    server_revision: i64,
    now: chrono::DateTime<chrono::Utc>,
    last_editor_device_id: Option<&str>,
) -> crate::ServerResult<crate::repo::NoteRow> {
    let _ = (item, existing, server_revision, now, last_editor_device_id);
    todo!("filled in by TG13-004")
}

pub(super) fn asset_row_from_push_item(
    item: &zk_domain::PushAssetItem,
    existing: Option<&crate::repo::AssetRow>,
    server_revision: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> crate::ServerResult<crate::repo::AssetRow> {
    let _ = (item, existing, server_revision, now);
    todo!("filled in by TG13-004")
}

pub(super) fn sync_change_from_row(
    row: &crate::service::SyncChangeRow,
) -> crate::ServerResult<zk_domain::SyncChange> {
    let _ = row;
    todo!("filled in by TG13-003")
}
