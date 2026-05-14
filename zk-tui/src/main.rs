// This thin binary does not use the package library until TUI modules land.
#![allow(unused_crate_dependencies)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let version = env!("CARGO_PKG_VERSION");

    println!("zk-tui {version}");

    ExitCode::SUCCESS
}
