//! Local operator administration commands.

use std::io::{self, ErrorKind, IsTerminal, Read, Write};

use chrono::Utc;
use serde::Serialize;
use ulid::Ulid;
use zk_backup::{
    BackupError,
    manifest::{
        BackupManifest, BackupManifestEncryption, list_manifests, manifest_file_name, read_manifest,
    },
};

use crate::{
    ServerDb, ServerError, ServerResult,
    cli::{
        AdminCommand, BackupCommand, BackupListArgs, BackupRunArgs, BackupShowArgs, CreateUserArgs,
    },
    config::ServerConfig,
    repo::UserRepo,
    service::{self, BackupStartError},
};

pub(crate) fn dispatch(command: AdminCommand) -> ServerResult<()> {
    match command {
        AdminCommand::CreateUser(args) => create_user(args),
        AdminCommand::Backup(command) => backup(command),
    }
}

pub(crate) fn create_user(args: CreateUserArgs) -> ServerResult<()> {
    let CreateUserArgs { username, force } = args;
    let config = ServerConfig::load()?;
    let database = ServerDb::open(&config.database_path)?;
    let existing_users = database.with_conn(UserRepo::count)?;

    if existing_users > 0 && !force {
        return Err(ServerError::Conflict(
            "at least one user exists; pass --force to create another".to_string(),
        ));
    }

    let password = read_password()?;
    if password.is_empty() {
        return Err(ServerError::InvalidRequest(
            "password must not be empty".to_string(),
        ));
    }

    let registered = database.with_conn(|connection| {
        service::register_user(connection, &username, &password, Utc::now())
    })?;

    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "Created user {} ({})",
        registered.username, registered.user_id
    )
    .map_err(wrap_internal_error)
}

fn backup(command: BackupCommand) -> ServerResult<()> {
    match command {
        BackupCommand::Run(args) => backup_run(&args),
        BackupCommand::List(args) => backup_list(&args),
        BackupCommand::Show(args) => backup_show(&args),
    }
}

fn backup_run(args: &BackupRunArgs) -> ServerResult<()> {
    let config = ServerConfig::load_with_config_path(args.config.as_deref())?;
    let service = service::backup_service_from_config(&config)
        .map_err(BackupStartError::into_server_error)?;
    let manifest = service
        .run_now(Utc::now())
        .map_err(server_error_from_backup)?;
    let mut stdout = io::stdout().lock();

    writeln!(stdout, "backup_id: {}", manifest.backup_id).map_err(wrap_internal_error)?;
    writeln!(stdout, "archive: {}", manifest.archive.file_name).map_err(wrap_internal_error)?;
    writeln!(stdout, "sha256: {}", manifest.archive.sha256_hex).map_err(wrap_internal_error)?;
    writeln!(stdout, "size_bytes: {}", manifest.archive.size_bytes).map_err(wrap_internal_error)
}

fn backup_list(args: &BackupListArgs) -> ServerResult<()> {
    let config = ServerConfig::load_with_config_path(args.config.as_deref())?;
    let output_dir = service::backup_output_dir_from_config(&config)?;
    let mut manifests = list_manifests(output_dir).map_err(server_error_from_backup)?;
    sort_manifests_desc(&mut manifests);

    if args.json {
        return write_json(&manifests);
    }
    if manifests.is_empty() {
        let mut stdout = io::stdout().lock();
        return writeln!(stdout, "no backups found").map_err(wrap_internal_error);
    }

    write_manifest_table(&manifests)
}

fn backup_show(args: &BackupShowArgs) -> ServerResult<()> {
    if Ulid::from_string(&args.backup_id).is_err() {
        return Err(backup_not_found());
    }

    let config = ServerConfig::load_with_config_path(args.config.as_deref())?;
    let output_dir = service::backup_output_dir_from_config(&config)?;
    let manifest_path = output_dir.join(manifest_file_name(&args.backup_id));
    let manifest = read_manifest(&manifest_path).map_err(|error| match error {
        BackupError::Io(error) if error.kind() == ErrorKind::NotFound => backup_not_found(),
        error => server_error_from_backup(error),
    })?;

    if args.json {
        return write_json(&manifest);
    }

    write_manifest_details(&manifest)
}

fn write_manifest_table(manifests: &[BackupManifest]) -> ServerResult<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "BACKUP_ID  CREATED_AT  SIZE  ENCRYPTION  ARCHIVE")
        .map_err(wrap_internal_error)?;

    for manifest in manifests {
        writeln!(
            stdout,
            "{}  {}  {}  {}  {}",
            manifest.backup_id,
            manifest.created_at.to_rfc3339(),
            manifest.archive.size_bytes,
            encryption_type(&manifest.encryption),
            manifest.archive.file_name
        )
        .map_err(wrap_internal_error)?;
    }

    Ok(())
}

fn write_manifest_details(manifest: &BackupManifest) -> ServerResult<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "backup_id: {}", manifest.backup_id).map_err(wrap_internal_error)?;
    writeln!(stdout, "created_at: {}", manifest.created_at.to_rfc3339())
        .map_err(wrap_internal_error)?;
    writeln!(stdout, "archive: {}", manifest.archive.file_name).map_err(wrap_internal_error)?;
    writeln!(stdout, "sha256: {}", manifest.archive.sha256_hex).map_err(wrap_internal_error)?;
    writeln!(stdout, "size_bytes: {}", manifest.archive.size_bytes).map_err(wrap_internal_error)?;
    write_encryption_details(&mut stdout, &manifest.encryption)?;
    writeln!(
        stdout,
        "source_vault_path: {}",
        manifest.source_summary.vault_path.display()
    )
    .map_err(wrap_internal_error)?;
    writeln!(stdout, "note_count: {}", manifest.source_summary.note_count)
        .map_err(wrap_internal_error)?;
    writeln!(
        stdout,
        "asset_count: {}",
        manifest.source_summary.asset_count
    )
    .map_err(wrap_internal_error)?;
    writeln!(
        stdout,
        "server_db_included: {}",
        manifest.source_summary.server_db_included
    )
    .map_err(wrap_internal_error)?;
    match manifest.source_summary.server_db_size_bytes {
        Some(size_bytes) => writeln!(stdout, "server_db_size_bytes: {size_bytes}"),
        None => writeln!(stdout, "server_db_size_bytes: none"),
    }
    .map_err(wrap_internal_error)
}

fn write_encryption_details(
    stdout: &mut impl Write,
    encryption: &BackupManifestEncryption,
) -> ServerResult<()> {
    writeln!(stdout, "encryption: {}", encryption_type(encryption)).map_err(wrap_internal_error)?;
    if let BackupManifestEncryption::Age {
        recipient_fingerprint,
    } = encryption
    {
        writeln!(stdout, "recipient_fingerprint: {recipient_fingerprint}")
            .map_err(wrap_internal_error)?;
    }

    Ok(())
}

fn write_json<T: Serialize>(value: &T) -> ServerResult<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).map_err(wrap_internal_error)?;
    writeln!(stdout).map_err(wrap_internal_error)
}

fn sort_manifests_desc(manifests: &mut [BackupManifest]) {
    manifests.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.backup_id.cmp(&left.backup_id))
    });
}

fn encryption_type(encryption: &BackupManifestEncryption) -> &'static str {
    match encryption {
        BackupManifestEncryption::None => "none",
        BackupManifestEncryption::Age { .. } => "age",
    }
}

fn backup_not_found() -> ServerError {
    ServerError::NotFound("backup not found".to_string())
}

fn server_error_from_backup(error: BackupError) -> ServerError {
    ServerError::Internal(Box::new(error))
}

fn read_password() -> ServerResult<String> {
    if io::stdin().is_terminal() {
        return read_password_from_tty();
    }

    let mut password = String::new();
    io::stdin()
        .read_to_string(&mut password)
        .map_err(wrap_internal_error)?;
    trim_trailing_newlines(&mut password);

    Ok(password)
}

fn read_password_from_tty() -> ServerResult<String> {
    let password = rpassword::prompt_password("Password: ").map_err(wrap_internal_error)?;
    let confirmation = rpassword::prompt_password("Confirm: ").map_err(wrap_internal_error)?;

    if password != confirmation {
        return Err(ServerError::InvalidRequest("password mismatch".to_string()));
    }

    Ok(password)
}

fn trim_trailing_newlines(value: &mut String) {
    while matches!(value.as_bytes().last(), Some(b'\n' | b'\r')) {
        value.pop();
    }
}

fn wrap_internal_error(error: impl std::error::Error + Send + Sync + 'static) -> ServerError {
    ServerError::Internal(Box::new(error))
}
