mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{PEONY, workdir};

#[test]
fn lto_plugin_host_spike_claims_rustc_bitcode() {
    let Some(plugin) = llvm_gold_plugin() else {
        eprintln!("skipping: LLVMgold.so unavailable");
        return;
    };

    let dir = workdir("lto_spike");
    let src = dir.join("main.rs");
    let obj = dir.join("main.o");
    std::fs::write(
        &src,
        "#[no_mangle]\npub extern \"C\" fn peony_lto_answer() -> i32 { 42 }\nfn main() {}\n",
    )
    .unwrap();

    let rustc = Command::new("rustc")
        .args(["-Clinker-plugin-lto", "--emit=obj", "-o"])
        .arg(&obj)
        .arg(&src)
        .output()
        .expect("run rustc");
    if !rustc.status.success() {
        eprintln!(
            "skipping: rustc could not emit linker-plugin LTO object: {}",
            String::from_utf8_lossy(&rustc.stderr)
        );
        return;
    }

    let output = Command::new(PEONY)
        .env("PEONY_LTO_DUMP_SYMBOLS", "1")
        .arg("-plugin")
        .arg(&plugin)
        .arg(&obj)
        .output()
        .expect("run peony LTO dump");

    assert!(
        output.status.success(),
        "P0 LTO dump failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("claimed: yes"), "{stdout}");
    assert!(stdout.contains("peony_lto_answer"), "{stdout}");
}

fn llvm_gold_plugin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PEONY_LLVMGOLD") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    let clang = Command::new("clang")
        .arg("--print-file-name=LLVMgold.so")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| PathBuf::from(stdout.trim()));
    if let Some(path) = clang.filter(|path| plugin_path_exists(path)) {
        return Some(path);
    }

    [
        "/usr/lib/llvm/22/lib/LLVMgold.so",
        "/usr/lib/llvm/22/lib64/LLVMgold.so",
        "/usr/lib/llvm/21/lib/LLVMgold.so",
        "/usr/lib/llvm/21/lib64/LLVMgold.so",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| plugin_path_exists(path))
}

fn plugin_path_exists(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "LLVMgold.so") && path.exists()
}
