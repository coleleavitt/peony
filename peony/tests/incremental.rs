//! Incremental layout-reuse regression tests (blueprint Phase 2).
//!
//! The non-negotiable gate: a one-object size-stable relink that reuses the
//! cached layout MUST produce a byte-identical output to a full link. These
//! tests exercise the real driver end-to-end (compile → `--incremental` link →
//! size-stable edit → relink → `cmp` vs a full link), so a regression in the
//! hazard fingerprint or the layout (de)serialization fails loudly.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{PEONY, run, workdir};

/// A freestanding `_start` that calls `compute()` and exits with its return
/// value — no libc, so the link is small, deterministic, and self-contained.
const START_S: &str = r#"
.intel_syntax noprefix
.global _start
.text
_start:
    call compute
    mov edi, eax
    mov eax, 60
    syscall
"#;

fn assemble_start(dir: &Path) -> PathBuf {
    let s = dir.join("start.s");
    let o = dir.join("start.o");
    std::fs::write(&s, START_S).unwrap();
    let st = Command::new("as")
        .args(["--64", "-o"])
        .arg(&o)
        .arg(&s)
        .status()
        .expect("run `as`");
    assert!(st.success(), "assembling start.s failed");
    o
}

/// Compile `compute.c` (freestanding, no PIC) with the given body, always from a
/// file literally named `compute.c` so the `STT_FILE` symbol is stable across
/// edits (a realistic "edit and recompile" scenario).
fn compile_compute(dir: &Path, body: &str) -> PathBuf {
    let c = dir.join("compute.c");
    let o = dir.join("compute.o");
    std::fs::write(&c, body).unwrap();
    let st = Command::new("cc")
        .args([
            "-c",
            "-fno-pic",
            "-fno-asynchronous-unwind-tables",
            "-ffreestanding",
            "-O2",
            "-o",
        ])
        .arg(&o)
        .arg(&c)
        .status()
        .expect("run `cc`");
    assert!(st.success(), "compiling compute.c failed");
    o
}

/// Run a peony link, returning (success, stderr).
fn peony_link(out: &Path, inputs: &[&PathBuf], extra: &[&str]) -> (bool, String) {
    let mut cmd = Command::new(PEONY);
    cmd.arg("-o").arg(out);
    for a in extra {
        cmd.arg(a);
    }
    for i in inputs {
        cmd.arg(i);
    }
    let o = cmd
        .env("PEONY_LOG", "peony=info")
        .output()
        .expect("run peony");
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stderr).into(),
    )
}

/// The core gate: a size-stable one-object relink reuses the cached layout and
/// stays byte-identical to a full link.
#[test]
fn layout_reuse_is_byte_identical_to_full_link() {
    let dir = workdir("inc-reuse");
    let start = assemble_start(&dir);

    // Seed: first incremental link records the front-end snapshot.
    let _ = compile_compute(&dir, "int compute(void){ return 42; }\n");
    let app = dir.join("app");
    let (ok, _) = peony_link(&app, &[&start, &dir.join("compute.o")], &["--incremental"]);
    assert!(ok, "seed link failed");
    assert_eq!(run(&app), 42, "seed output must exit 42");

    // Size-stable edit: only the returned immediate changes (42 → 77).
    let _ = compile_compute(&dir, "int compute(void){ return 77; }\n");

    // Incremental relink — must take the layout-reuse fast path.
    let (ok, stderr) = peony_link(&app, &[&start, &dir.join("compute.o")], &["--incremental"]);
    assert!(ok, "incremental relink failed");
    assert!(
        stderr.contains("reusing cached layout"),
        "expected the layout-reuse fast path to fire on a size-stable relink; stderr:\n{stderr}"
    );

    // Full reference link of the same inputs (no incremental machinery).
    let full = dir.join("full");
    let (ok, _) = peony_link(&full, &[&start, &dir.join("compute.o")], &[]);
    assert!(ok, "full link failed");

    // The non-negotiable gate.
    let app_bytes = std::fs::read(&app).unwrap();
    let full_bytes = std::fs::read(&full).unwrap();
    assert_eq!(
        app_bytes, full_bytes,
        "incremental layout-reuse output must be byte-identical to a full link"
    );
    assert_eq!(run(&app), 77, "relinked output must exit 77");
}

/// Alternating size-stable relinks stay byte-identical across many iterations
/// (guards against drift in the persisted snapshot or stale id↔name reuse).
#[test]
fn layout_reuse_stable_across_alternating_relinks() {
    let dir = workdir("inc-altern");
    let start = assemble_start(&dir);
    let app = dir.join("app");
    let full = dir.join("full");
    let inputs = [&start, &dir.join("compute.o")];

    let bodies = [
        ("int compute(void){ return 42; }\n", 42),
        ("int compute(void){ return 77; }\n", 77),
    ];
    let mut reused = 0;
    for i in 0..8 {
        let (body, code) = bodies[i % 2];
        let _ = compile_compute(&dir, body);
        let (ok, stderr) = peony_link(&app, &inputs, &["--incremental"]);
        assert!(ok, "incremental link {i} failed");
        if stderr.contains("reusing cached layout") {
            reused += 1;
        }
        let (ok, _) = peony_link(&full, &inputs, &[]);
        assert!(ok, "full link {i} failed");
        assert_eq!(
            std::fs::read(&app).unwrap(),
            std::fs::read(&full).unwrap(),
            "iter {i}: incremental output diverged from full link"
        );
        assert_eq!(run(&app), code, "iter {i}: wrong exit code");
    }
    // The first iteration seeds the cache (full emit); the rest must reuse.
    assert!(
        reused >= 6,
        "expected most relinks to reuse the cached layout, got {reused}/8"
    );
}
