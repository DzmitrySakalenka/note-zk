#![warn(missing_docs)]
// Test targets inherit dev-dependencies used by integration tests.
#![cfg_attr(test, allow(unused_crate_dependencies))]
#![doc = "Sync core: local change planning and remote change application for the filesystem-first knowledge system."]

pub mod bootstrap;
pub mod conflict;
pub mod error;
mod http_transport;
pub mod plan;
pub mod pull;
pub mod push;
pub mod transport;

#[rustfmt::skip]
pub use bootstrap::{detect_initial_mode, InitialSyncMode};
#[rustfmt::skip]
pub use conflict::{
    detect_pull_conflict, detect_push_conflict, local_unchanged_since_sync, revision_matches,
    write_conflict_copy,
};
pub use error::{SyncError, SyncResult};
pub use http_transport::HttpTransport;
#[rustfmt::skip]
pub use plan::{PlannerContext, PlannedOp, PlannedOperation, plan_local_changes};
#[rustfmt::skip]
pub use pull::{PullApplyContext, PullApplyOutcome, apply_pull_response};
#[rustfmt::skip]
pub use push::{PushApplyOutcome, apply_push_response, build_push_request};
#[rustfmt::skip]
pub use transport::{NoopTransport, SyncTransport};
