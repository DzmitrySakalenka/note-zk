use crate::{
    cli::{DoctorArgs, SharedFlags},
    error::CliError,
};

pub(crate) fn run(_args: &DoctorArgs, _shared: &SharedFlags) -> Result<i32, CliError> {
    unimplemented!("TG9-005 wires the doctor command implementation");
}
