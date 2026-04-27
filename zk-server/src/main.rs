#![warn(missing_docs)]
// This thin binary delegates to the library target, where package dependencies are used.
#![allow(unused_crate_dependencies)]
//! HTTP server binary entry point.

fn main() -> zk_server::ServerResult<()> {
    zk_server::run()
}
