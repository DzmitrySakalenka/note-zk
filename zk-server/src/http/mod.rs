//! HTTP router, handlers, and response mapping.

mod admin;
mod auth;
mod error;
pub mod extract;
mod health;
mod router;
mod sync;

#[doc(hidden)]
pub(crate) use admin::admin_routes;
#[doc(hidden)]
pub(crate) use auth::auth_routes;
pub use error::ApiError;
pub use health::HealthResponse;
pub use router::{build_router, serve};
#[doc(hidden)]
pub use sync::sync_routes;
