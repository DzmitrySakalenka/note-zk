//! HTTP router, handlers, and response mapping.

mod error;
pub mod extract;
mod health;
mod router;
mod sync;

pub use error::ApiError;
pub use health::HealthResponse;
pub use router::{build_router, serve};
#[allow(unused_imports)]
pub(super) use sync::sync_routes;
