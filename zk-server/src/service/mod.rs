//! Business-logic services built on top of the server repositories.

mod devices;

pub use devices::{RegisterDeviceInput, RegisterDeviceOutcome, register_device};
