//! Remote backup administration command handlers.

use std::{
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Context;
use reqwest::{
    Method, StatusCode,
    blocking::{Client, RequestBuilder, Response},
    header::RETRY_AFTER,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    cli::{
        AdminBackupListArgs, AdminBackupShowArgs, AdminBackupTriggerArgs, OutputMode, SharedFlags,
    },
    error::{CliError, EXIT_OK},
    output::write_json_line,
};

const ADMIN_BACKUPS_PATH: &str = "/admin/backups";
const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) fn list(args: &AdminBackupListArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let transport = admin_transport(shared, &args.token_env)?;
    let backups = transport.list_backups()?;

    match OutputMode::from(args.json) {
        OutputMode::Human => write_backup_list_human(&backups),
        OutputMode::Json => write_json_line(&backups),
    }
    .context("while attempting to write admin backup list output")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

pub(crate) fn show(args: &AdminBackupShowArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let transport = admin_transport(shared, &args.token_env)?;
    let backup_id = resolve_backup_id_prefix(&transport, &args.backup_id)?;
    let manifest = transport
        .get_manifest(&backup_id)?
        .ok_or_else(|| CliError::Issue("backup not found".to_string()))?;

    match OutputMode::from(args.json) {
        OutputMode::Human => write_manifest_human(&manifest),
        OutputMode::Json => write_json_line(&manifest),
    }
    .context("while attempting to write admin backup manifest output")
    .map_err(CliError::Internal)?;

    Ok(EXIT_OK)
}

pub(crate) fn trigger(
    args: &AdminBackupTriggerArgs,
    shared: &SharedFlags,
) -> Result<i32, CliError> {
    let transport = admin_transport(shared, &args.token_env)?;
    let started = transport.start_backup()?;
    let mut stdout = io::stdout().lock();

    writeln!(stdout, "backup_id: {}", started.backup_id)
        .context("while attempting to write admin backup trigger output")
        .map_err(CliError::Internal)?;

    if args.wait {
        wait_for_manifest(
            &transport,
            &started.backup_id,
            Duration::from_secs(args.timeout_secs),
        )?;
    }

    Ok(EXIT_OK)
}

fn admin_transport(shared: &SharedFlags, token_env: &str) -> Result<AdminTransport, CliError> {
    let server_url = shared.server_url.clone().ok_or_else(|| {
        CliError::Argument("admin backup subcommands require --server-url".to_string())
    })?;
    let bearer_token = std::env::var(token_env).map_err(|_| {
        CliError::Issue(format!(
            "environment variable {token_env} is required for admin backup subcommand"
        ))
    })?;

    AdminTransport::new(server_url, bearer_token, token_env.to_string())
}

fn resolve_backup_id_prefix(
    transport: &AdminTransport,
    backup_id_prefix: &str,
) -> Result<String, CliError> {
    if backup_id_prefix.is_empty() {
        return Err(CliError::Issue(
            "backup id prefix must not be empty".to_string(),
        ));
    }

    let mut matches: Vec<_> = transport
        .list_backups()?
        .items
        .into_iter()
        .filter(|backup| backup.backup_id.starts_with(backup_id_prefix))
        .map(|backup| backup.backup_id)
        .collect();
    matches.sort();

    match matches.len() {
        0 => Err(CliError::Issue("backup not found".to_string())),
        1 => matches
            .pop()
            .ok_or_else(|| CliError::Issue("backup not found".to_string())),
        _ => Err(CliError::Issue(format!(
            "backup id prefix is ambiguous; candidates: {}",
            matches.join(", ")
        ))),
    }
}

fn wait_for_manifest(
    transport: &AdminTransport,
    backup_id: &str,
    timeout: Duration,
) -> Result<(), CliError> {
    let started_at = Instant::now();

    loop {
        if transport.get_manifest(backup_id)?.is_some() {
            return Ok(());
        }

        if started_at.elapsed() >= timeout || timeout < POLL_INTERVAL {
            return Err(CliError::Request(format!(
                "timed out waiting for backup {backup_id}"
            )));
        }

        let remaining = timeout.saturating_sub(started_at.elapsed());
        if remaining < POLL_INTERVAL {
            return Err(CliError::Request(format!(
                "timed out waiting for backup {backup_id}"
            )));
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

#[derive(Debug, Clone)]
struct AdminTransport {
    client: Client,
    base_url: Url,
    bearer_token: String,
    token_env: String,
}

impl AdminTransport {
    fn new(base_url: Url, bearer_token: String, token_env: String) -> Result<Self, CliError> {
        let client = Client::builder().build().map_err(request_error)?;

        Ok(Self {
            client,
            base_url,
            bearer_token,
            token_env,
        })
    }

    fn list_backups(&self) -> Result<ListBackupsResponse, CliError> {
        self.get_json(ADMIN_BACKUPS_PATH)
    }

    fn get_manifest(&self, backup_id: &str) -> Result<Option<BackupManifest>, CliError> {
        let path = format!("{ADMIN_BACKUPS_PATH}/{backup_id}");
        let response = self.get(&path)?.send().map_err(request_error)?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = self.response_for_status(response)?;
        response
            .json::<BackupManifest>()
            .map(Some)
            .map_err(request_error)
    }

    fn start_backup(&self) -> Result<StartBackupResponse, CliError> {
        self.post_json(ADMIN_BACKUPS_PATH, &StartBackupRequest {})
    }

    fn get_json<T>(&self, path: &str) -> Result<T, CliError>
    where
        T: DeserializeOwned,
    {
        let response = self.get(path)?.send().map_err(request_error)?;
        let response = self.response_for_status(response)?;

        response.json::<T>().map_err(request_error)
    }

    fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T, CliError>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        let response = self
            .request(Method::POST, path)?
            .json(body)
            .send()
            .map_err(request_error)?;
        let response = self.response_for_status(response)?;

        response.json::<T>().map_err(request_error)
    }

    fn get(&self, path: &str) -> Result<RequestBuilder, CliError> {
        self.request(Method::GET, path)
    }

    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, CliError> {
        let url = self.base_url.join(path).map_err(request_error)?;

        Ok(self
            .client
            .request(method, url)
            .bearer_auth(&self.bearer_token))
    }

    fn response_for_status(&self, response: Response) -> Result<Response, CliError> {
        if response.status().is_success() {
            return Ok(response);
        }

        Err(self.api_error(response))
    }

    fn api_error(&self, response: Response) -> CliError {
        let status = response.status();
        let retry_after = retry_after_seconds(&response);
        let body = response.text().unwrap_or_else(|error| error.to_string());

        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                CliError::Request(format!("auth failed; check ${} scope", self.token_env))
            }
            StatusCode::CONFLICT => CliError::Request("server has no [backup] config".to_string()),
            StatusCode::SERVICE_UNAVAILABLE => CliError::Request(format!(
                "another backup in flight; retry in {}s",
                retry_after.as_deref().unwrap_or("unknown")
            )),
            status if status.is_server_error() => CliError::Request(format!(
                "server error {status}: {}",
                response_body_message(&body)
            )),
            _ => CliError::Request(response_body_message(&body)),
        }
    }
}

fn retry_after_seconds(response: &Response) -> Option<String> {
    response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()
        .map(str::to_string)
}

fn response_body_message(body: &str) -> String {
    serde_json::from_str::<ApiErrorEnvelope>(body).map_or_else(
        |_| body.to_string(),
        |envelope| format!("{}: {}", envelope.error.code, envelope.error.message),
    )
}

fn write_backup_list_human(backups: &ListBackupsResponse) -> io::Result<()> {
    let mut stdout = io::stdout().lock();

    if backups.items.is_empty() && backups.in_flight.is_empty() {
        return writeln!(stdout, "no backups found");
    }

    if !backups.items.is_empty() {
        writeln!(stdout, "BACKUP_ID  CREATED_AT  SIZE  ENCRYPTION")?;
        for backup in &backups.items {
            writeln!(
                stdout,
                "{}  {}  {}  {}",
                backup.backup_id, backup.created_at, backup.size_bytes, backup.encryption_type
            )?;
        }
    }

    if !backups.in_flight.is_empty() {
        writeln!(stdout, "in_flight: {}", backups.in_flight.join(", "))?;
    }

    Ok(())
}

fn write_manifest_human(manifest: &BackupManifest) -> io::Result<()> {
    let mut stdout = io::stdout().lock();

    writeln!(stdout, "backup_id: {}", manifest.backup_id)?;
    writeln!(stdout, "schema_version: {}", manifest.schema_version)?;
    writeln!(stdout, "created_at: {}", manifest.created_at)?;
    writeln!(stdout, "archive: {}", manifest.archive.file_name)?;
    writeln!(stdout, "sha256: {}", manifest.archive.sha256_hex)?;
    writeln!(stdout, "size_bytes: {}", manifest.archive.size_bytes)?;
    write_encryption_details(&mut stdout, &manifest.encryption)?;
    writeln!(
        stdout,
        "source_vault_path: {}",
        manifest.source_summary.vault_path.display()
    )?;
    writeln!(stdout, "note_count: {}", manifest.source_summary.note_count)?;
    writeln!(
        stdout,
        "asset_count: {}",
        manifest.source_summary.asset_count
    )?;
    writeln!(
        stdout,
        "server_db_included: {}",
        manifest.source_summary.server_db_included
    )?;
    match manifest.source_summary.server_db_size_bytes {
        Some(size_bytes) => writeln!(stdout, "server_db_size_bytes: {size_bytes}"),
        None => writeln!(stdout, "server_db_size_bytes: none"),
    }
}

fn write_encryption_details(
    stdout: &mut impl Write,
    encryption: &BackupManifestEncryption,
) -> io::Result<()> {
    writeln!(stdout, "encryption: {}", encryption_type(encryption))?;
    if let BackupManifestEncryption::Age {
        recipient_fingerprint,
    } = encryption
    {
        writeln!(stdout, "recipient_fingerprint: {recipient_fingerprint}")?;
    }

    Ok(())
}

fn encryption_type(encryption: &BackupManifestEncryption) -> &'static str {
    match encryption {
        BackupManifestEncryption::None => "none",
        BackupManifestEncryption::Age { .. } => "age",
    }
}

fn request_error(error: impl std::error::Error) -> CliError {
    CliError::Request(error.to_string())
}

#[derive(Debug, Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct StartBackupRequest {}

#[derive(Debug, Deserialize)]
struct StartBackupResponse {
    backup_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ListBackupsResponse {
    items: Vec<ListedBackup>,
    in_flight: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ListedBackup {
    backup_id: String,
    created_at: String,
    size_bytes: u64,
    encryption_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BackupManifest {
    schema_version: u32,
    backup_id: String,
    created_at: String,
    source_summary: BackupSourceSummary,
    archive: BackupArchiveDescriptor,
    encryption: BackupManifestEncryption,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BackupSourceSummary {
    vault_path: PathBuf,
    note_count: u32,
    asset_count: u32,
    server_db_included: bool,
    server_db_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BackupArchiveDescriptor {
    file_name: String,
    size_bytes: u64,
    sha256_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BackupManifestEncryption {
    None,
    Age { recipient_fingerprint: String },
}
