//! Built-in TUI screens and modal overlays.

pub mod admin;
pub mod auth;
pub(crate) mod body_preview;
pub mod doctor;
pub mod help;
pub mod notes;
pub mod overview;
pub mod resolve_conflict;
#[cfg(feature = "ai")]
pub mod search;
pub mod sync;
pub mod trash;
