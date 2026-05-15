//! Built-in TUI screens and modal overlays.

pub mod admin;
pub mod auth;
pub(crate) mod body_preview;
#[cfg(feature = "ai")]
pub mod chat;
pub mod doctor;
pub mod help;
pub mod notes;
pub mod overview;
pub mod resolve_conflict;
#[cfg(feature = "ai")]
pub mod search;
pub mod sync;
pub mod trash;
#[cfg(feature = "ai")]
pub mod wiki;
