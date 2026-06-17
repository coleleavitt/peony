//! Shared-object (`-shared`, ET_DYN) tests: peony links a `.so` whose exported
//! symbols are resolvable via `dlopen`/`dlsym`, mirroring the cdylib/proc-macro
//! output rustc/cargo need. Each test compiles a PIC object, links it with
//! `peony -shared`, then loads it from a C harness and calls into it.

mod common;
use std::path::Path;
use std::process::Command;

use common::*;

/// Compile C source as PIC (suitable for a shared object) → `.o`.
fn compile_pic(dir: &Path, name: &str, src: &str) -> std::path::PathBuf {
    let c = dir.join(format!("{name}.c"));
    let o = dir.join(format!("{name}.o"));
    std::fs::write(&c, src).unwrap();
    let st = Command::new("cc")
        .args(["-c", "-fpic", "-o"])
        .arg(&o)
        .arg(&c)
        .status()
        .expect("run `cc`");
    assert!(st.success(), "compiling {name} failed");
    o
}

/// Build the dlopen harness: `./harness <lib.so>` returns the result of calling
/// the library's exported functions, or a non-zero diagnostic code on failure.
fn build_harness(dir: &Path) -> std::path::PathBuf {
    let c = dir.join("harness.c");
    let bin = dir.join("harness");
    std::fs::write(
        &c,
        r#"
#include <dlfcn.h>
#include <stdio.h>
int main(int argc, char **argv) {
    void *h = dlopen(argv[1], RTLD_NOW);
    if (!h) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 2; }
    int (*add)(int, int) = (int(*)(int,int)) dlsym(h, "add");
    int (*meaning)(void) = (int(*)(void)) dlsym(h, "meaning");
    if (!add || !meaning) { fprintf(stderr, "dlsym: %s\n", dlerror()); return 3; }
    if (add(20, 22) != 42) return 4;
    if (meaning() != 42) return 5;
    return 0;
}
"#,
    )
    .unwrap();
    let st = Command::new("cc")
        .arg(&c)
        .arg("-ldl")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("run cc for harness");
    assert!(st.success(), "building harness failed");
    bin
}

/// peony `-shared` produces an ET_DYN `.so` whose exports `dlsym` can resolve and
/// call correctly.
#[test]
fn shared_object_dlopen_dlsym() {
    let dir = workdir("shared");
    let obj = compile_pic(
        &dir,
        "lib",
        "int add(int a, int b) { return a + b; }\nint meaning(void) { return 42; }\n",
    );
    let so = dir.join("libpeonytest.so");
    link(&so, &[obj], &["-shared", "-soname", "libpeonytest.so"]);

    // Structural: it must be a shared object with no PT_INTERP.
    let rd = Command::new("readelf")
        .args(["-hl"])
        .arg(&so)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&rd.stdout);
    assert!(
        out.contains("DYN (Shared object file)"),
        "not ET_DYN:\n{out}"
    );
    assert!(
        !out.contains("INTERP"),
        "shared object must not have PT_INTERP"
    );

    // Functional: dlopen + dlsym + call.
    let harness = build_harness(&dir);
    let rc = Command::new(&harness)
        .arg(&so)
        .status()
        .expect("run harness")
        .code()
        .expect("harness exit code");
    assert_eq!(rc, 0, "dlopen/dlsym harness failed with code {rc}");
}

/// The exported symbols appear in `.dynsym` as defined (non-UND) FUNC globals.
#[test]
fn shared_object_exports_in_dynsym() {
    let dir = workdir("shared-dynsym");
    let obj = compile_pic(
        &dir,
        "lib",
        "int add(int a, int b) { return a + b; }\nint meaning(void) { return 42; }\n",
    );
    let so = dir.join("libpeonytest2.so");
    link(&so, &[obj], &["-shared"]);

    let rd = Command::new("readelf")
        .args(["--dyn-syms"])
        .arg(&so)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&rd.stdout);
    // Both must be present as defined (a real section index, not UND).
    for sym in ["add", "meaning"] {
        let line = out
            .lines()
            .find(|l| l.split_whitespace().last() == Some(sym))
            .unwrap_or_else(|| panic!("export `{sym}` missing from .dynsym:\n{out}"));
        assert!(
            !line.contains("UND"),
            "export `{sym}` is UND (not defined):\n{line}"
        );
        assert!(line.contains("FUNC"), "export `{sym}` is not FUNC:\n{line}");
    }
}

/// Build the TLS dlopen harness: `./harness <lib.so>` exercises a thread-local
/// in the loaded library (from the main thread and a second thread) and checks
/// the results. Each thread sees its own instance.
fn build_tls_harness(dir: &Path) -> std::path::PathBuf {
    let c = dir.join("tlsharness.c");
    let bin = dir.join("tlsharness");
    std::fs::write(
        &c,
        r#"
#include <dlfcn.h>
#include <pthread.h>
#include <stdio.h>
typedef int (*bump_t)(int);
static bump_t bump;
static void *worker(void *arg) {
    // A fresh thread: its thread-local starts at the initializer (100).
    long r = bump(5);              // 100 + 5 = 105
    return (void *)r;
}
int main(int argc, char **argv) {
    void *h = dlopen(argv[1], RTLD_NOW);
    if (!h) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 2; }
    bump = (bump_t) dlsym(h, "bump");
    if (!bump) { fprintf(stderr, "dlsym: %s\n", dlerror()); return 3; }
    if (bump(10) != 110) return 4;          // main thread: 100 + 10
    if (bump(10) != 120) return 5;          // accumulates within the thread
    pthread_t t;
    pthread_create(&t, 0, worker, 0);
    void *res;
    pthread_join(t, &res);
    if ((long)res != 105) return 6;         // worker had its own instance
    if (bump(0) != 120) return 7;           // main thread unchanged by worker
    return 0;
}
"#,
    )
    .unwrap();
    let st = Command::new("cc")
        .arg(&c)
        .args(["-ldl", "-lpthread", "-o"])
        .arg(&bin)
        .status()
        .expect("run cc for tls harness");
    assert!(st.success(), "building tls harness failed");
    bin
}

/// peony `-shared` keeps General-Dynamic TLS (does not relax to Local-Exec):
/// a `__thread` variable in a dlopen'd `.so` is per-thread and reads/writes
/// correctly. This is the case that breaks if TLSGD is wrongly relaxed.
#[test]
fn shared_object_tls_general_dynamic() {
    let dir = workdir("shared-tls-gd");
    // `__thread` with the global-dynamic model → R_X86_64_TLSGD relocations.
    let obj = compile_pic_tls(
        &dir,
        "tlslib",
        "global-dynamic",
        "__thread int counter = 100;\nint bump(int n) { counter += n; return counter; }\n",
    );
    // General-Dynamic keeps a `call __tls_get_addr@PLT`, so libc must be linked
    // (that is where `__tls_get_addr` lives).
    let so = dir.join("libtlsgd.so");
    let libc_dir = libc_search_dir();
    link(
        &so,
        &[obj],
        &["-shared", "-soname", "libtlsgd.so", "-L", &libc_dir, "-lc"],
    );

    // Structural: GD relocs present, no LE relaxation.
    let rd = Command::new("readelf")
        .args(["-rW"])
        .arg(&so)
        .output()
        .unwrap();
    let rel = String::from_utf8_lossy(&rd.stdout);
    assert!(
        rel.contains("R_X86_64_DTPMOD64"),
        "shared TLS must emit DTPMOD64 (General-Dynamic), got:\n{rel}"
    );
    // `__tls_get_addr` must be a real PLT import (the GD `call` is kept, not
    // relaxed) — it appears as a JUMP_SLOT relocation against the imported symbol.
    assert!(
        rel.contains("__tls_get_addr"),
        "shared GD TLS must import __tls_get_addr (kept GD call), got relocs:\n{rel}"
    );

    // Functional: per-thread semantics via dlopen.
    let harness = build_tls_harness(&dir);
    let rc = Command::new(&harness)
        .arg(&so)
        .status()
        .expect("run tls harness")
        .code()
        .expect("tls harness exit code");
    assert_eq!(rc, 0, "TLS dlopen harness failed with code {rc}");
}

/// peony `-shared` handles Initial-Exec TLS (`R_X86_64_GOTTPOFF`) via a GOT slot
/// with an `R_X86_64_TPOFF64` dynamic relocation (not a static Local-Exec value).
#[test]
fn shared_object_tls_initial_exec() {
    let dir = workdir("shared-tls-ie");
    let obj = compile_pic_tls(
        &dir,
        "ielib",
        "initial-exec",
        "__thread int counter = 100;\nint bump(int n) { counter += n; return counter; }\n",
    );
    let so = dir.join("libtlsie.so");
    link(&so, &[obj], &["-shared", "-soname", "libtlsie.so"]);

    let rd = Command::new("readelf")
        .args(["-rW"])
        .arg(&so)
        .output()
        .unwrap();
    let rel = String::from_utf8_lossy(&rd.stdout);
    assert!(
        rel.contains("R_X86_64_TPOFF64"),
        "shared Initial-Exec TLS must emit TPOFF64, got:\n{rel}"
    );

    let harness = build_tls_harness(&dir);
    let rc = Command::new(&harness)
        .arg(&so)
        .status()
        .expect("run tls harness")
        .code()
        .expect("tls harness exit code");
    assert_eq!(
        rc, 0,
        "Initial-Exec TLS dlopen harness failed with code {rc}"
    );
}

/// The directory containing the system `libc.so` (for `-L`/`-lc`), discovered
/// via the C compiler. Used by the General-Dynamic TLS test, whose kept
/// `call __tls_get_addr@PLT` must resolve against libc.
fn libc_search_dir() -> String {
    let out = Command::new("cc")
        .args(["-print-file-name=libc.so"])
        .output()
        .expect("cc -print-file-name");
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Path::new(&path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/lib".to_string())
}

/// Compile C source as PIC with an explicit TLS model → `.o`.
fn compile_pic_tls(dir: &Path, name: &str, tls_model: &str, src: &str) -> std::path::PathBuf {
    let c = dir.join(format!("{name}.c"));
    let o = dir.join(format!("{name}.o"));
    std::fs::write(&c, src).unwrap();
    let st = Command::new("cc")
        .args(["-c", "-fpic", &format!("-ftls-model={tls_model}"), "-o"])
        .arg(&o)
        .arg(&c)
        .status()
        .expect("run `cc`");
    assert!(st.success(), "compiling {name} failed");
    o
}
