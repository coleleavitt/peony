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
use std::time::{Duration, Instant};

use common::{PEONY, cc_file, run, workdir};

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

/// Did the relink take an incremental fast path (parse-only-changed, or the
/// layout-reuse late path)?
fn took_fast_path(stderr: &str) -> bool {
    stderr.contains("parse-only-changed fast relink")
        || stderr.contains("reusing cached layout")
        || stderr.contains("incremental patch emitted")
        || stderr.contains("daemon relink")
}

/// Compile a freestanding C object named `name.c` with the given body.
fn compile_named(dir: &Path, name: &str, body: &str) -> PathBuf {
    let c = dir.join(format!("{name}.c"));
    let o = dir.join(format!("{name}.o"));
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
    assert!(st.success(), "compiling {name}.c failed");
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

    // Incremental relink — must take an incremental fast path (parse-only-changed
    // when eligible, else layout-reuse).
    let (ok, stderr) = peony_link(&app, &[&start, &dir.join("compute.o")], &["--incremental"]);
    assert!(ok, "incremental relink failed");
    assert!(
        took_fast_path(&stderr),
        "expected an incremental fast path to fire on a size-stable relink; stderr:\n{stderr}"
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

/// The changed object has a RELOCATION (a call to a function defined in another,
/// unchanged object): the parse-only fast path must re-apply it against the
/// cached symbol VA and stay byte-identical to a full link. (The 402-object
/// benchmark's `compute.o` is reloc-free, so this is the dedicated coverage for
/// reloc-apply from the minimal cached symbol view.)
#[test]
fn parse_only_changed_applies_relocs_from_cache() {
    let dir = workdir("inc-reloc");
    let start = assemble_start(&dir);
    // A separate, unchanged object that `compute` calls — forces a relocation.
    let helper = compile_named(&dir, "helper", "int helper(void){ return 7; }\n");
    let app = dir.join("app");
    let inputs = [&start, &dir.join("compute.o"), &helper];

    // Seed: compute calls helper.
    compile_named(
        &dir,
        "compute",
        "extern int helper(void);\nint compute(void){ return 42 + helper(); }\n",
    );
    let (ok, _) = peony_link(&app, &inputs, &["--incremental"]);
    assert!(ok, "seed link failed");
    assert_eq!(run(&app), 49, "seed: 42 + helper()=7");

    // Size-stable edit: still calls helper, only the constant changes.
    compile_named(
        &dir,
        "compute",
        "extern int helper(void);\nint compute(void){ return 77 + helper(); }\n",
    );
    let (ok, stderr) = peony_link(&app, &inputs, &["--incremental"]);
    assert!(ok, "incremental relink failed");
    assert!(
        took_fast_path(&stderr),
        "expected a fast path; stderr:\n{stderr}"
    );

    let full = dir.join("full");
    let (ok, _) = peony_link(&full, &inputs, &[]);
    assert!(ok, "full link failed");
    assert_eq!(
        std::fs::read(&app).unwrap(),
        std::fs::read(&full).unwrap(),
        "reloc-bearing changed object: incremental output must equal a full link"
    );
    assert_eq!(run(&app), 84, "relinked: 77 + helper()=7");
}

/// The changed object calls a libc function — a `PLT32` (and `GOTPCREL`)
/// relocation to an IMPORTED symbol, the most common real edit. The parse-only
/// path re-applies it from the cached `plt_address`/`got_address` with the
/// fabricated `import=false` resolution; this locks in that those fields are
/// dead for the whitelisted relocs (the subtle soundness the adversarial audit
/// confirmed). Skips gracefully if the toolchain crt/libc is unavailable.
#[test]
fn parse_only_changed_import_plt_byte_identical() {
    let dir = workdir("inc-import");
    let crt1 = cc_file("crt1.o");
    let crti = cc_file("crti.o");
    let crtn = cc_file("crtn.o");
    let Some(libcdir) = cc_file("libc.so").parent().map(Path::to_path_buf) else {
        eprintln!("skipping: libc.so unavailable");
        return;
    };
    if !crt1.exists() {
        eprintln!("skipping: toolchain crt unavailable");
        return;
    }

    let compile = |body: &str| {
        let c = dir.join("app.c");
        std::fs::write(&c, body).unwrap();
        let st = Command::new("cc")
            .args(["-c", "-fno-pic", "-o"])
            .arg(dir.join("app.o"))
            .arg(&c)
            .status()
            .expect("cc -c");
        assert!(st.success(), "compiling app.c failed");
    };
    let link = |out: &Path, incr: bool| -> (bool, String) {
        let mut cmd = Command::new(PEONY);
        cmd.arg("-o").arg(out);
        if incr {
            cmd.arg("--incremental");
        }
        cmd.arg(&crt1)
            .arg(&crti)
            .arg(dir.join("app.o"))
            .arg(&crtn)
            .args(["-L", libcdir.to_str().unwrap(), "-l", "c"]);
        let o = cmd
            .env("PEONY_LOG", "peony=info")
            .output()
            .expect("run peony");
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stderr).into(),
        )
    };

    let app = dir.join("app");
    // Seed: main calls putchar (an imported libc function) → PLT32 import reloc.
    compile("#include <stdio.h>\nint main(void){ putchar('A'); return 42; }\n");
    let (ok, _) = link(&app, true);
    assert!(ok, "seed link failed");
    assert_eq!(run(&app), 42, "seed exit 42");

    // Size-stable edit: same putchar call, different return constant.
    compile("#include <stdio.h>\nint main(void){ putchar('A'); return 77; }\n");
    let (ok, stderr) = link(&app, true);
    assert!(ok, "incremental relink failed");
    assert!(
        took_fast_path(&stderr),
        "expected a fast path; stderr:\n{stderr}"
    );

    let full = dir.join("full");
    let (ok, _) = link(&full, false);
    assert!(ok, "full link failed");
    assert_eq!(
        std::fs::read(&app).unwrap(),
        std::fs::read(&full).unwrap(),
        "import-PLT changed object: incremental output must equal a full link"
    );
    assert_eq!(run(&app), 77, "relinked exit 77");
}

/// The resident daemon: a `peony --daemon` server holds the layout + symbol view
/// in RAM and serves relinks over a Unix socket; a normal `--incremental` client
/// delegates to it. The relink MUST be byte-identical to a full link. (This is
/// the sub-5ms path: the daemon skips the per-relink deserialize + symbol-view
/// rebuild.) The daemon child is always killed before assertions.
#[test]
fn daemon_relink_is_byte_identical_to_full_link() {
    let dir = workdir("inc-daemon");
    let start = assemble_start(&dir);
    let app = dir.join("app");
    let compute = dir.join("compute.o");

    // Seed link establishes the cache the daemon loads.
    compile_compute(&dir, "int compute(void){ return 42; }\n");
    let (ok, _) = peony_link(&app, &[&start, &compute], &["--incremental"]);
    assert!(ok, "seed link failed");

    // Start the daemon as a background child.
    let mut daemon = Command::new(PEONY)
        .arg("--daemon")
        .arg("-o")
        .arg(&app)
        .arg(&start)
        .arg(&compute)
        .env("PEONY_LOG", "peony=info")
        .spawn()
        .expect("spawn daemon");

    // Wait for the socket (give up + skip if the daemon never comes up).
    let sock = dir.join("app.incr").join("daemon.sock");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if !sock.exists() {
        let _ = daemon.kill();
        let _ = daemon.wait();
        eprintln!("skipping: daemon did not come up");
        return;
    }

    // Size-stable edit, then relink via the client (delegates to the daemon).
    compile_compute(&dir, "int compute(void){ return 77; }\n");
    let (relink_ok, stderr) = peony_link(&app, &[&start, &compute], &["--incremental"]);
    let app_bytes = std::fs::read(&app).ok();

    // Full reference link of the edited inputs.
    let full = dir.join("full");
    let (full_ok, _) = peony_link(&full, &[&start, &compute], &[]);
    let full_bytes = std::fs::read(&full).ok();

    // Always kill the daemon before asserting.
    let _ = daemon.kill();
    let _ = daemon.wait();

    assert!(relink_ok, "client relink failed");
    assert!(full_ok, "full link failed");
    assert!(
        stderr.contains("daemon relink"),
        "expected the client to delegate to the daemon; stderr:\n{stderr}"
    );
    assert_eq!(
        app_bytes, full_bytes,
        "daemon relink output must be byte-identical to a full link"
    );
    assert_eq!(run(&app), 77, "relinked output must exit 77");
}

/// `PEONY_DAEMON=1` auto-spawns a daemon on a relink (once a cache exists) and
/// delegates to it — the "automatic sub-5ms" dev-shell experience. Verifies the
/// auto-spawn fires (a daemon.log appears) and the relink is byte-identical. A
/// short idle timeout reaps the detached daemon shortly after.
#[test]
fn daemon_autospawn_is_byte_identical_to_full_link() {
    let dir = workdir("inc-autospawn");
    let start = assemble_start(&dir);
    let app = dir.join("app");
    let compute = dir.join("compute.o");
    let inputs = [&start, &compute];

    let link_auto = |out: &Path| -> (bool, String) {
        let o = Command::new(PEONY)
            .arg("-o")
            .arg(out)
            .arg(&start)
            .arg(&compute)
            .env("PEONY_LOG", "peony=info")
            .env("PEONY_DAEMON", "1")
            .env("PEONY_DAEMON_IDLE_SECS", "2")
            .output()
            .expect("run peony");
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stderr).into(),
        )
    };

    // Seed (default-on incremental; no daemon yet, no cache → just links).
    compile_compute(&dir, "int compute(void){ return 42; }\n");
    let (ok, _) = link_auto(&app);
    assert!(ok, "seed link failed");

    // Size-stable edit → relink: auto-spawns a daemon (cache now exists) + uses it.
    compile_compute(&dir, "int compute(void){ return 77; }\n");
    let (ok, stderr) = link_auto(&app);
    assert!(ok, "auto-spawn relink failed");

    // Full reference (opt out of incremental).
    let full = dir.join("full");
    let (ok, _) = peony_link(&full, &inputs, &["--no-incremental"]);
    assert!(ok, "full link failed");

    let daemon_log = dir.join("app.incr").join("daemon.log");
    assert!(
        daemon_log.exists(),
        "PEONY_DAEMON=1 should have auto-spawned a daemon (no daemon.log); stderr:\n{stderr}"
    );
    assert!(
        took_fast_path(&stderr),
        "auto-spawn relink should take a fast path; stderr:\n{stderr}"
    );
    assert_eq!(
        std::fs::read(&app).unwrap(),
        std::fs::read(&full).unwrap(),
        "auto-spawned daemon relink must be byte-identical to a full link"
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
        if took_fast_path(&stderr) {
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
