//! Runs the real `jev` binary, the way every later command test will.

use std::process::Command;

#[test]
fn prints_its_version_on_stdout_and_nothing_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_jev"))
        .output()
        .expect("failed to run jev");

    assert!(output.status.success(), "exit status: {:?}", output.status);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("jev {}\n", env!("CARGO_PKG_VERSION")),
    );
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
