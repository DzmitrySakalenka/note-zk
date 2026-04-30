//! Local operator administration commands.

use std::io::{self, IsTerminal, Read, Write};

use chrono::Utc;

use crate::{
    ServerDb, ServerError, ServerResult,
    cli::{AdminCommand, CreateUserArgs},
    config::ServerConfig,
    repo::UserRepo,
    service,
};

pub(crate) fn dispatch(command: AdminCommand) -> ServerResult<()> {
    match command {
        AdminCommand::CreateUser(args) => create_user(args),
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
