use crate::{
    cli::{ScanArgs, SharedFlags},
    error::CliError,
};

pub(crate) fn run(_args: &ScanArgs, _shared: &SharedFlags) -> Result<i32, CliError> {
    unimplemented!("TG9-003 wires the scan command implementation");
}
