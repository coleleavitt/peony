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
