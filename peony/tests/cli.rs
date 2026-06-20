mod common;

use std::process::Command;

use common::{PEONY, assemble, run, workdir};

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

#[test]
fn response_file_expands_linker_arguments() {
    let dir = workdir("rsp");
    let obj = assemble(
        &dir,
        "rsp",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    let rsp = dir.join("args.rsp");
    std::fs::write(
        &rsp,
        format!("-o '{}'\n'{}'\n", exe.display(), obj.display()),
    )
    .unwrap();

    let output = Command::new(PEONY)
        .arg(format!("@{}", rsp.display()))
        .output()
        .expect("run peony with response file");
    assert!(
        output.status.success(),
        "stderr was {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(run(&exe), 42);
}

#[test]
fn unsupported_semantic_flags_fail_explicitly() {
    let output = Command::new(PEONY)
        .args(["--wrap", "malloc"])
        .output()
        .expect("run peony");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported output-affecting linker flag"),
        "stderr was {stderr}"
    );
}

#[test]
fn unsupported_z_flags_fail_explicitly() {
    let output = Command::new(PEONY)
        .args(["-z", "execstack"])
        .output()
        .expect("run peony");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported output-affecting linker flag")
            && stderr.contains("-z execstack"),
        "stderr was {stderr}"
    );
}
