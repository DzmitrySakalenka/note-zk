use assert_cmd::cargo::CommandCargoExt;
use std::process::{Command, Stdio};

#[test]
fn scan_help_exits_cleanly_when_stdout_reader_closes_early() {
    let mut command = Command::cargo_bin("zk-cli").expect("zk-cli binary should build");
    command
        .arg("scan")
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = command.spawn().expect("zk-cli should spawn");
    drop(child.stdout.take());

    let status = child.wait().expect("zk-cli should exit");
    assert!(
        status.success(),
        "help path should exit cleanly when stdout closes early: {status}"
    );
}
