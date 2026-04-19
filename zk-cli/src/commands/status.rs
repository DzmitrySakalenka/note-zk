use crate::{
    cli::{SharedFlags, StatusArgs},
    error::CliError,
};

pub(crate) fn run(_args: &StatusArgs, _shared: &SharedFlags) -> Result<i32, CliError> {
    unimplemented!("TG9-004 wires the status command implementation");
}
