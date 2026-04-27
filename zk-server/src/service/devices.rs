use chrono::{DateTime, Utc};

use crate::{
    ServerError, ServerResult,
    db::ServerDb,
    repo::{DeviceCursorRepo, DeviceRepo, DeviceRow},
};

/// Input payload for idempotent device registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterDeviceInput<'a> {
    /// Stable client installation identifier.
    pub device_id: &'a str,
    /// User-facing device label.
    pub device_name: &'a str,
    /// Optional client platform string.
    pub platform: Option<&'a str>,
    /// Optional client application version string.
    pub app_version: Option<&'a str>,
    /// Opaque capabilities payload stored as raw JSON.
    pub capabilities_json: &'a str,
}

/// Outcome returned from one device registration attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterDeviceOutcome {
    /// Whether the device row was created during this call.
    pub first_registration: bool,
    /// Stable device registration timestamp.
    pub created_at: DateTime<Utc>,
    /// Most recent authenticated or registration timestamp.
    pub last_seen_at: DateTime<Utc>,
}

/// Registers one device idempotently and initializes its sync cursor once.
///
/// # Errors
///
/// Returns [`ServerError::InvalidRequest`] when `device_id` or
/// `capabilities_json` is invalid, or [`ServerError`] if the transactional
/// repository operations fail.
pub fn register_device(
    db: &ServerDb,
    input: RegisterDeviceInput<'_>,
    now: DateTime<Utc>,
) -> ServerResult<RegisterDeviceOutcome> {
    validate_device_id(input.device_id)?;
    validate_capabilities_json(input.capabilities_json)?;

    db.with_tx(|connection| {
        let existing = DeviceRepo::get(connection, input.device_id)?;
        let first_registration = existing.is_none();
        let created_at = existing.as_ref().map_or(now, |row| row.created_at);
        let device_row = DeviceRow {
            device_id: input.device_id.to_string(),
            device_name: input.device_name.to_string(),
            platform: input.platform.map(str::to_string),
            app_version: input.app_version.map(str::to_string),
            capabilities_json: input.capabilities_json.to_string(),
            created_at,
            last_seen_at: now,
        };

        DeviceRepo::upsert(connection, &device_row)?;
        if first_registration {
            DeviceCursorRepo::initialize(connection, input.device_id, now)?;
        }

        Ok(RegisterDeviceOutcome {
            first_registration,
            created_at,
            last_seen_at: now,
        })
    })
}

fn validate_device_id(device_id: &str) -> ServerResult<()> {
    if device_id.is_empty() {
        return Err(ServerError::InvalidRequest(
            "device_id must not be empty".to_string(),
        ));
    }

    if device_id.len() > 128 {
        return Err(ServerError::InvalidRequest(
            "device_id must be at most 128 characters".to_string(),
        ));
    }

    if !device_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ServerError::InvalidRequest(
            "device_id must contain only ASCII letters, digits, '-' or '_'".to_string(),
        ));
    }

    Ok(())
}

fn validate_capabilities_json(capabilities_json: &str) -> ServerResult<()> {
    serde_json::from_str::<serde_json::Value>(capabilities_json).map_err(|_| {
        ServerError::InvalidRequest("capabilities_json must be valid JSON".to_string())
    })?;

    Ok(())
}
