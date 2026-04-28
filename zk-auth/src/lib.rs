#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
#![doc = "Authentication primitives shared across the workspace. Owns password hashing, device-token issuance, session-id generation, and the auth-state types consumed by `zk-server`."]

pub mod error;
pub mod password;
pub mod session;
pub mod token;

pub use error::{AuthError, AuthResult};

// TG14-001 fixes the crate's agreed dependency surface before the primitive
// modules consume it in TG14-002 through TG14-004.
use argon2 as _;
use chrono as _;
use data_encoding as _;
use rand as _;
use serde as _;
use serde_json as _;
use sha2 as _;
use subtle as _;
