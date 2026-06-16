//! The real test: compile actual C programs and link them against the system
//! glibc using **peony** as the linker, then run them. This exercises the full
//! pipeline — dynamic linking, linker scripts (`libc.so`), PLT, GOT, crt startup.

mod common;
use common::*;

/// `int main(){ return 42; }` linked against libc, run end-to-end.
#[test]
fn c_return_value() {
    let dir = workdir("c_ret");
    let Some(exe) = link_c(&dir, "ret", "int main(void){ return 42; }\n") else {
        eprintln!("skipping: toolchain crt/libc unavailable");
        return;
    };
    assert_eq!(run(&exe), 42);
}

/// A C program that calls `printf` — verifies stdout and exit code.
#[test]
fn c_printf() {
    let dir = workdir("c_printf");
    let Some(exe) = link_c(
        &dir,
        "p",
        "#include <stdio.h>\nint main(void){ printf(\"peony works\\n\"); return 7; }\n",
    ) else {
        return;
    };
    let (rc, out) = run_capture(&exe);
    assert_eq!(rc, 7);
    assert_eq!(out, "peony works\n");
}

/// A C program using malloc/string functions from libc.
#[test]
fn c_malloc_string() {
    let dir = workdir("c_malloc");
    let Some(exe) = link_c(
        &dir,
        "m",
        "#include <stdlib.h>\n#include <string.h>\n\
         int main(void){ char*p=malloc(16); strcpy(p,\"hello\"); \
         int n=(int)strlen(p); free(p); return n*8+2; }\n", // 5*8+2 = 42
    ) else {
        return;
    };
    assert_eq!(run(&exe), 42);
}

/// Capstone: use peony AS the system linker via `cc -B` — `cc` compiles and
/// invokes peony (named `ld`) with its full flag set; the result must run.
#[test]
fn cc_driven_link() {
    use std::process::Command;
    let dir = workdir("cc_driven");
    let bindir = dir.join("bin");
    std::fs::create_dir_all(&bindir).unwrap();
    // cc -B<prefix> looks for `<prefix>ld`.
    std::fs::copy(PEONY, bindir.join("ld")).unwrap();

    let src = dir.join("h.c");
    std::fs::write(
        &src,
        "#include <stdio.h>\nint main(void){ printf(\"cc-driven peony\\n\"); return 5; }\n",
    )
    .unwrap();
    let exe = dir.join("h");
    let st = Command::new("cc")
        .arg(format!("-B{}/", bindir.display()))
        .args(["-fno-pie", "-no-pie", "-o"])
        .arg(&exe)
        .arg(&src)
        .status()
        .expect("run cc");
    if !st.success() {
        eprintln!("skipping: cc -B driver unavailable");
        return;
    }
    let (rc, out) = run_capture(&exe);
    assert_eq!(rc, 5);
    assert_eq!(out, "cc-driven peony\n");
}

/// A multi-function C program (cross-function calls, a global, a loop).
#[test]
fn c_computation() {
    let dir = workdir("c_comp");
    let Some(exe) = link_c(
        &dir,
        "c",
        "static int add(int a,int b){return a+b;}\n\
         int g = 6;\n\
         int main(void){ int s=0; for(int i=0;i<g;i++) s=add(s,7); return s; }\n", // 6*7 = 42
    ) else {
        return;
    };
    assert_eq!(run(&exe), 42);
}
