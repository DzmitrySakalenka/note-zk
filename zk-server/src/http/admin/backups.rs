//! Admin backup endpoints.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zk_backup::manifest::{BackupManifest, BackupManifestEncryption};

use crate::{
    ServerError,
    auth::AuthenticatedAdmin,
    http::ApiError,
    service::{self, BackupStartError},
    state::ServerState,
};

const RETRY_AFTER_SECONDS: u64 = 60;
const LOG_TARGET: &str = "zk_server::backup";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartBackupRequest {}

#[derive(Debug, Serialize)]
pub(super) struct StartBackupResponse {
    backup_id: String,
    started_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(super) struct ListBackupsResponse {
    items: Vec<ListedBackup>,
    in_flight: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ListedBackup {
    backup_id: String,
    created_at: DateTime<Utc>,
    size_bytes: u64,
    encryption_type: &'static str,
}

pub(super) async fn start(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Json(_request): Json<StartBackupRequest>,
) -> Result<(StatusCode, Json<StartBackupResponse>), ApiError> {
    let reserved = match service::reserve_backup(&state, Utc::now()).await {
        Ok(reserved) => reserved,
        Err(BackupStartError::InFlight) => {
            return Err(ApiError::with_retry_after(
                ServerError::ServiceUnavailable("backup already in flight".to_string()),
                RETRY_AFTER_SECONDS,
            ));
        }
        Err(error) => return Err(ApiError::from(error.into_server_error())),
    };
    let response = StartBackupResponse {
        backup_id: reserved.backup_id().to_string(),
        started_at: reserved.started_at(),
    };
    let backup_id = response.backup_id.clone();
    let state_for_cleanup = state.clone();

    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || reserved.run()).await;
        match result {
            Ok(Ok(manifest)) => {
                tracing::info!(
                    target: LOG_TARGET,
                    backup_id = %manifest.backup_id,
                    "admin backup completed"
                );
            }
            Ok(Err(error)) => {
                tracing::error!(target: LOG_TARGET, error = %error, "admin backup failed");
            }
            Err(error) => {
                tracing::error!(
                    target: LOG_TARGET,
                    error = %error,
                    "admin backup blocking task failed"
                );
            }
        }
        service::clear_in_flight_backup(&state_for_cleanup, &backup_id).await;
    });

    Ok((StatusCode::ACCEPTED, Json(response)))
}

pub(super) async fn list(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
) -> Result<Json<ListBackupsResponse>, ApiError> {
    let mut manifests = service::list_backup_manifests(&state)?;
    manifests.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.backup_id.cmp(&left.backup_id))
    });
    let items = manifests.into_iter().map(listed_backup).collect();
    let in_flight = service::in_flight_backup_ids(&state).await;

    Ok(Json(ListBackupsResponse { items, in_flight }))
}

pub(super) async fn get(
    State(state): State<ServerState>,
    _admin: AuthenticatedAdmin,
    Path(backup_id): Path<String>,
) -> Result<Json<BackupManifest>, ApiError> {
    Ok(Json(service::read_backup_manifest(&state, &backup_id)?))
}

fn listed_backup(manifest: BackupManifest) -> ListedBackup {
    ListedBackup {
        backup_id: manifest.backup_id,
        created_at: manifest.created_at,
        size_bytes: manifest.archive.size_bytes,
        encryption_type: encryption_type(&manifest.encryption),
    }
}

fn encryption_type(encryption: &BackupManifestEncryption) -> &'static str {
    match encryption {
        BackupManifestEncryption::None => "none",
        BackupManifestEncryption::Age { .. } => "age",
    }
}
