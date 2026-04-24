//! HTTP router, handlers, and response mapping.

mod error;
mod health;
mod router;

pub use error::ApiError;
pub use health::HealthResponse;
pub use router::build_router;
