//! Business-logic services built on top of the server repositories.

mod conflict;
mod devices;
mod events;

pub use conflict::{
    AcceptOutcome, accept_or_conflict, is_fresh_create, is_stale_revision, next_server_revision,
};
pub(crate) use devices::validate_device_id;
pub use devices::{RegisterDeviceInput, RegisterDeviceOutcome, register_device};
pub use events::{
    FetchChangesOutcome, RecordAssetChange, RecordNoteChange, RecordedEvent, SyncChangeRow,
    fetch_changes_since, record_asset_change, record_note_change,
};
