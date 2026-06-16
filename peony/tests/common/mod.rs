//! Shared harness for peony's end-to-end tests — the Rust equivalent of mold's
//! `test/common.inc`: assemble fixtures with the system toolchain, link them with
//! peony, execute, and inspect with `readelf`. Used by every `tests/*.rs` file.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

pub const PEONY: &str = env!("CARGO_BIN_EXE_peony");

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh, unique temp directory for one test step.
pub fn workdir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("peony-it-{}-{}-{}", std::process::id(), tag, n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create workdir");
    dir
}

/// Assemble x86-64 source → `.o`; returns its path.
pub fn assemble(dir: &Path, name: &str, src: &str) -> PathBuf {
    let s = dir.join(format!("{name}.s"));
    let o = dir.join(format!("{name}.o"));
    std::fs::write(&s, src).unwrap();
    let st = Command::new("as")
        .args(["--64", "-o"])
        .arg(&o)
        .arg(&s)
        .status()
        .expect("run `as`");
    assert!(st.success(), "assembling {name} failed");
    o
}

/// Compile C source (freestanding, no PIC) → `.o`.
pub fn compile_c(dir: &Path, name: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{name}.c"));
    let o = dir.join(format!("{name}.o"));
    std::fs::write(&c, src).unwrap();
    let st = Command::new("cc")
        .args([
            "-c",
            "-fno-pic",
            "-fno-asynchronous-unwind-tables",
            "-ffreestanding",
            "-o",
        ])
        .arg(&o)
        .arg(&c)
        .status()
        .expect("run `cc`");
    assert!(st.success(), "compiling {name} failed");
    o
}

/// Build a static archive `name.a` from objects.
pub fn archive(dir: &Path, name: &str, objs: &[PathBuf]) -> PathBuf {
    let a = dir.join(format!("{name}.a"));
    let _ = std::fs::remove_file(&a);
    let mut cmd = Command::new("ar");
    cmd.arg("rcs").arg(&a);
    for o in objs {
        cmd.arg(o);
    }
    assert!(cmd.status().expect("run `ar`").success(), "ar failed");
    a
}

/// Link with peony; panics with stderr on failure.
pub fn link(out: &Path, inputs: &[PathBuf], extra: &[&str]) {
    let o = link_raw(out, inputs, extra);
    assert!(
        o.status.success(),
        "peony link failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// Link with peony, returning the raw `Output` (for inspecting failure/stderr).
pub fn link_raw(out: &Path, inputs: &[PathBuf], extra: &[&str]) -> Output {
    let mut cmd = Command::new(PEONY);
    cmd.arg("-o").arg(out);
    for a in extra {
        cmd.arg(a);
    }
    for i in inputs {
        cmd.arg(i);
    }
    cmd.env("RUST_LOG", "info").output().expect("run peony")
}

/// Reference link with GNU `ld` (for differential checks).
pub fn ld_link(out: &Path, inputs: &[PathBuf]) -> bool {
    let mut cmd = Command::new("ld");
    cmd.arg("-o").arg(out);
    for i in inputs {
        cmd.arg(i);
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// Run an executable and return its exit code.
pub fn run(path: &Path) -> i32 {
    Command::new(path)
        .status()
        .expect("run executable")
        .code()
        .expect("exit code")
}

/// Run with extra environment variables (e.g. `LD_LIBRARY_PATH`).
pub fn run_env(path: &Path, env: &[(&str, &str)]) -> i32 {
    let mut cmd = Command::new(path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.status()
        .expect("run executable")
        .code()
        .expect("exit code")
}

/// `cc -print-file-name=<name>` (locate crt objects / libc).
pub fn cc_file(name: &str) -> PathBuf {
    let out = Command::new("cc")
        .arg(format!("-print-file-name={name}"))
        .output()
        .expect("run cc -print-file-name");
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Compile C source and link a full dynamic executable against the system libc
/// using **peony** as the linker (crt1/crti/crtn + `-lc`). Returns the exe path,
/// or `None` if the toolchain pieces aren't available.
pub fn link_c(dir: &Path, name: &str, src: &str) -> Option<PathBuf> {
    let c = dir.join(format!("{name}.c"));
    let o = dir.join(format!("{name}.o"));
    std::fs::write(&c, src).unwrap();
    let st = Command::new("cc")
        .args(["-c", "-fno-pic", "-o"])
        .arg(&o)
        .arg(&c)
        .status()
        .expect("cc -c");
    if !st.success() {
        return None;
    }
    let (crt1, crti, crtn) = (cc_file("crt1.o"), cc_file("crti.o"), cc_file("crtn.o"));
    let libcdir = cc_file("libc.so").parent()?.to_path_buf();
    if !crt1.exists() {
        return None;
    }
    let exe = dir.join(name);
    let out = link_raw(
        &exe,
        &[crt1, crti, o, crtn],
        &["-L", libcdir.to_str().unwrap(), "-l", "c"],
    );
    if out.status.success() {
        Some(exe)
    } else {
        eprintln!(
            "peony link_c failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        None
    }
}

/// Compile + link C source(s) using **peony as `cc`'s linker** (`cc -B<peony>`),
/// the way mold's tests drive the toolchain. Returns the exe path, or `None` if
/// the toolchain isn't available. `cxx` selects the C++ driver.
pub fn cc_b(dir: &Path, name: &str, srcs: &[(&str, &str)], cxx: bool) -> Option<PathBuf> {
    let bindir = dir.join("ldbin");
    std::fs::create_dir_all(&bindir).ok()?;
    std::fs::copy(PEONY, bindir.join("ld")).ok()?;
    let mut src_paths = Vec::new();
    for (fname, body) in srcs {
        let p = dir.join(fname);
        std::fs::write(&p, body).ok()?;
        src_paths.push(p);
    }
    let exe = dir.join(name);
    let driver = if cxx { "c++" } else { "cc" };
    let mut cmd = Command::new(driver);
    cmd.arg(format!("-B{}/", bindir.display()))
        .args(["-fno-pie", "-no-pie", "-o"])
        .arg(&exe);
    for p in &src_paths {
        cmd.arg(p);
    }
    match cmd.status() {
        Ok(s) if s.success() => Some(exe),
        _ => None,
    }
}

/// Run an exe and capture (exit code, stdout).
pub fn run_capture(path: &Path) -> (i32, String) {
    let out = Command::new(path).output().expect("run exe");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Compile a shared library `lib<name>.so` from C source; returns its path.
pub fn compile_shared(dir: &Path, name: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{name}.c"));
    let so = dir.join(format!("lib{name}.so"));
    std::fs::write(&c, src).unwrap();
    let st = Command::new("cc")
        .args(["-shared", "-fPIC", "-o"])
        .arg(&so)
        .arg(&c)
        .status()
        .expect("run `cc -shared`");
    assert!(st.success(), "compiling shared lib {name} failed");
    so
}

/// `readelf <args> <path>` stdout as a String.
pub fn readelf(path: &Path, args: &[&str]) -> String {
    let out = Command::new("readelf")
        .args(args)
        .arg(path)
        .output()
        .expect("run readelf");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
