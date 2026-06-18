mod common;

use std::process::Command;

use common::PEONY;

#[test]
fn help_flag_prints_usage_without_inputs() {
    let output = Command::new(PEONY)
        .arg("--help")
        .output()
        .expect("run peony");

    assert!(
        output.status.success(),
        "stderr was {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("peony [OPTIONS] <inputs>"),
        "stdout was {stdout}"
    );
    assert!(stdout.contains("--trace-stack"), "stdout was {stdout}");
}
