//! Business-logic services built on top of the server repositories.

mod conflict;
mod devices;

pub use conflict::{
    AcceptOutcome, accept_or_conflict, is_fresh_create, is_stale_revision, next_server_revision,
};
pub use devices::{RegisterDeviceInput, RegisterDeviceOutcome, register_device};
