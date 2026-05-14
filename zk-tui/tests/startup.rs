// This integration test exercises the binary through Cargo's executable path.
#![allow(unused_crate_dependencies)]

use std::process::Command;

#[test]
fn startup_prints_name_and_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_zk-tui"))
        .output()
        .expect("zk-tui binary should run");

    assert!(output.status.success(), "zk-tui should exit successfully");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be valid UTF-8"),
        format!("zk-tui {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        output.stderr.is_empty(),
        "zk-tui should not write to stderr"
    );
}
