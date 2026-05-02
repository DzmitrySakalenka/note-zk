//! Backup service helpers shared by HTTP handlers and the scheduler.

use std::{io, path::Path};

use chrono::{DateTime, Utc};
use ulid::Ulid;
use zk_backup::{
    BackupError, BackupResult,
    manifest::{BackupManifest, list_manifests, manifest_file_name, read_manifest},
    retention::RetentionPolicy,
    service::{BackupRun, BackupService, BackupServiceConfig},
};

use crate::{ServerError, ServerResult, config::ServerConfig, state::ServerState};

const BACKUP_DISABLED_MESSAGE: &str = "backup config is disabled";
const BACKUP_IN_FLIGHT_MESSAGE: &str = "backup already in flight";

#[derive(Debug)]
pub(crate) enum BackupStartError {
    Disabled,
    InFlight,
    Failed(BackupError),
}

impl BackupStartError {
    pub(crate) fn into_server_error(self) -> ServerError {
        match self {
            Self::Disabled => ServerError::Conflict(BACKUP_DISABLED_MESSAGE.to_string()),
            Self::InFlight => ServerError::ServiceUnavailable(BACKUP_IN_FLIGHT_MESSAGE.to_string()),
            Self::Failed(error) => ServerError::Internal(Box::new(error)),
        }
    }
}

pub(crate) struct ReservedBackup {
    service: BackupService,
    run: BackupRun,
}

impl ReservedBackup {
    pub(crate) fn backup_id(&self) -> &str {
        &self.run.backup_id
    }

    pub(crate) fn started_at(&self) -> DateTime<Utc> {
        self.run.started_at
    }

    pub(crate) fn run(self) -> BackupResult<BackupManifest> {
        self.service.run_reserved(self.run)
    }
}

pub(crate) async fn reserve_backup(
    state: &ServerState,
    started_at: DateTime<Utc>,
) -> Result<ReservedBackup, BackupStartError> {
    let service = backup_service_from_config(state.config())?;
    let run = service
        .reserve_run(started_at)
        .map_err(BackupStartError::Failed)?;
    let mut in_flight_backup_id = state.in_flight_backup_id().lock().await;
    if in_flight_backup_id.is_some() {
        return Err(BackupStartError::InFlight);
    }
    *in_flight_backup_id = Some(run.backup_id.clone());

    Ok(ReservedBackup { service, run })
}

pub(crate) async fn clear_in_flight_backup(state: &ServerState, backup_id: &str) {
    let mut in_flight_backup_id = state.in_flight_backup_id().lock().await;
    if in_flight_backup_id.as_deref() == Some(backup_id) {
        *in_flight_backup_id = None;
    }
}

pub(crate) async fn in_flight_backup_ids(state: &ServerState) -> Vec<String> {
    state
        .in_flight_backup_id()
        .lock()
        .await
        .iter()
        .cloned()
        .collect()
}

pub(crate) fn list_backup_manifests(state: &ServerState) -> ServerResult<Vec<BackupManifest>> {
    let output_dir = backup_output_dir(state)?;

    list_manifests(output_dir).map_err(server_error_from_backup)
}

pub(crate) fn read_backup_manifest(
    state: &ServerState,
    backup_id: &str,
) -> ServerResult<BackupManifest> {
    if Ulid::from_string(backup_id).is_err() {
        return Err(backup_not_found());
    }

    let output_dir = backup_output_dir(state)?;
    let manifest_path = output_dir.join(manifest_file_name(backup_id));

    read_manifest(&manifest_path).map_err(|error| match error {
        BackupError::Io(error) if error.kind() == io::ErrorKind::NotFound => backup_not_found(),
        error => server_error_from_backup(error),
    })
}

pub(crate) fn backup_service_from_config(
    config: &ServerConfig,
) -> Result<BackupService, BackupStartError> {
    let backup = config.backup.as_ref().ok_or(BackupStartError::Disabled)?;
    let service_config = BackupServiceConfig {
        vault_path: config.vault_path.clone(),
        server_db_path: Some(config.database_path.clone()),
        effective_config_toml: None,
        output_dir: backup.output_dir.clone(),
        include_attachments: backup.include_attachments,
        include_server_db: backup.include_server_db,
        encryption: backup.encryption.clone(),
    };
    let retention = RetentionPolicy::new(backup.retention_count);

    BackupService::new(service_config, retention).map_err(BackupStartError::Failed)
}

pub(crate) fn backup_output_dir_from_config(config: &ServerConfig) -> ServerResult<&Path> {
    config
        .backup
        .as_ref()
        .map(|backup| backup.output_dir.as_path())
        .ok_or_else(|| ServerError::Conflict(BACKUP_DISABLED_MESSAGE.to_string()))
}

fn backup_output_dir(state: &ServerState) -> ServerResult<&Path> {
    backup_output_dir_from_config(state.config())
}

fn backup_not_found() -> ServerError {
    ServerError::NotFound("backup not found".to_string())
}

fn server_error_from_backup(error: BackupError) -> ServerError {
    ServerError::Internal(Box::new(error))
}
