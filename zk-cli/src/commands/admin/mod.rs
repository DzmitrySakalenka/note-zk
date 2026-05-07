//! Remote operator administration command handlers.

use crate::{
    cli::{AdminAiCommand, AdminBackupCommand, AdminCommand, SharedFlags},
    error::CliError,
};

pub(crate) mod ai;
pub(crate) mod backup;

pub(crate) fn run(command: AdminCommand, shared: &SharedFlags) -> Result<i32, CliError> {
    match command {
        AdminCommand::Ai(command) => match command {
            AdminAiCommand::Recompute(args) => ai::recompute(&args, shared),
            AdminAiCommand::Jobs(args) => ai::jobs(&args, shared),
            AdminAiCommand::Cancel(args) => ai::cancel(&args, shared),
        },
        AdminCommand::Backup(command) => match command {
            AdminBackupCommand::List(args) => backup::list(&args, shared),
            AdminBackupCommand::Show(args) => backup::show(&args, shared),
            AdminBackupCommand::Trigger(args) => backup::trigger(&args, shared),
        },
    }
}
