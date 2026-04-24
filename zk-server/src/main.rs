#![warn(missing_docs)]
//! HTTP server binary entry point.

fn main() -> zk_server::ServerResult<()> {
    zk_server::run()
}
