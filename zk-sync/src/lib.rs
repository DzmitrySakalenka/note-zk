#![warn(missing_docs)]
#![doc = "Sync core: local change planning and remote change application for the filesystem-first knowledge system."]

pub mod bootstrap;
pub mod conflict;
pub mod error;
pub mod plan;
pub mod pull;
pub mod push;
pub mod transport;

pub use error::{SyncError, SyncResult};
