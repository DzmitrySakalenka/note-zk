//! Business-logic services built on top of the server repositories.

mod auth;
mod backups;
mod conflict;
mod devices;
mod events;

pub use auth::{
    CreateSessionSpec, IssueTokenSpec, IssuedSession, IssuedToken, RegisteredUser, RotateTokenSpec,
    SessionLifetime, VerifiedUser, create_web_session, issue_device_token, refresh_web_session,
    register_user, revoke_device_token, revoke_web_session, rotate_device_token,
    sweep_expired_sessions, verify_password,
};
pub(crate) use backups::{
    BackupStartError, backup_output_dir_from_config, backup_service_from_config,
    clear_in_flight_backup, in_flight_backup_ids, list_backup_manifests, read_backup_manifest,
    reserve_backup,
};
pub use conflict::{
    AcceptOutcome, accept_or_conflict, is_fresh_create, is_stale_revision, next_server_revision,
};
pub(crate) use devices::validate_device_id;
pub use devices::{RegisterDeviceInput, RegisterDeviceOutcome, register_device};
pub use events::{
    FetchChangesOutcome, RecordAssetChange, RecordNoteChange, RecordedEvent, SyncChangeRow,
    fetch_changes_since, record_asset_change, record_note_change,
};
pub(crate) use events::{record_asset_change_in_conn, record_note_change_in_conn};
