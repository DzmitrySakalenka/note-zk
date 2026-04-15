mod db;
mod error;
mod migration;
mod schema;

pub use db::{open_memory, open_or_create};
pub use error::IndexError;
pub use migration::ensure_schema;
