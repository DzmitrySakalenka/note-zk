mod db;
mod error;

pub use db::{open_memory, open_or_create};
pub use error::IndexError;
