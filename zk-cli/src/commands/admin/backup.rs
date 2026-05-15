//! Remote backup administration command handlers.

use std::{
    io::{self, Write},
    time::Duration,
};

use anyhow::Context;
use zk_cli_core::admin::{
    AdminClient, AdminClientError, BackupManifest, BackupManifestEncryption, ListBackupsResponse,
};

use crate::{
    cli::{
        AdminBackupListArgs, AdminBackupShowArgs, AdminBackupTriggerArgs, OutputMode, SharedFlags,
    },
    error::{CliError, EXIT_OK},
    output::write_json_line,
};

pub(crate) fn list(args: &AdminBackupListArgs, shared: &SharedFlags) -> Result<i32, CliError> {
    let transport = admin_transport(shared, &args.token_env)?;
    let backups = transport.list_backups().map_err(admin_error)?;

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
        .get_manifest(&backup_id)
        .map_err(admin_error)?
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
    let started = transport.start_backup().map_err(admin_error)?;
    let mut stdout = io::stdout().lock();

    writeln!(stdout, "backup_id: {}", started.backup_id)
        .context("while attempting to write admin backup trigger output")
        .map_err(CliError::Internal)?;

    if args.wait {
        transport
            .wait_for_backup_manifest(&started.backup_id, Duration::from_secs(args.timeout_secs))
            .map_err(admin_error)?;
    }

    Ok(EXIT_OK)
}

fn admin_transport(shared: &SharedFlags, token_env: &str) -> Result<AdminClient, CliError> {
    let server_url = shared.server_url.clone().ok_or_else(|| {
        CliError::Argument("admin backup subcommands require --server-url".to_string())
    })?;
    let bearer_token = std::env::var(token_env).map_err(|_| {
        CliError::Issue(format!(
            "environment variable {token_env} is required for admin backup subcommand"
        ))
    })?;

    AdminClient::new(server_url, bearer_token, token_env.to_string()).map_err(admin_error)
}

fn resolve_backup_id_prefix(
    transport: &AdminClient,
    backup_id_prefix: &str,
) -> Result<String, CliError> {
    if backup_id_prefix.is_empty() {
        return Err(CliError::Issue(
            "backup id prefix must not be empty".to_string(),
        ));
    }

    let mut matches: Vec<_> = transport
        .list_backups()
        .map_err(admin_error)?
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

fn admin_error(error: AdminClientError) -> CliError {
    match error {
        AdminClientError::Request(message) => CliError::Request(message),
    }
}
