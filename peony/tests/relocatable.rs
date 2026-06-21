//! Native relocatable (`-r`) output — partial linking.
//!
//! The gate: `peony -r a.o b.o -o combined.o` must produce a valid `ET_REL`
//! object whose merged sections, symbol table, and (un-applied) relocations
//! re-link into a correct executable — i.e. a cross-object call survives the
//! merge and resolves to the right address on the second link.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{PEONY, readelf, run, workdir};

fn assemble_start(dir: &Path) -> PathBuf {
    let s = dir.join("start.s");
    let o = dir.join("start.o");
    std::fs::write(
        &s,
        ".intel_syntax noprefix\n.global _start\n.text\n_start:\n    call a\n    mov edi, eax\n    mov eax, 60\n    syscall\n",
    )
    .unwrap();
    assert!(
        Command::new("as")
            .args(["--64", "-o"])
            .arg(&o)
            .arg(&s)
            .status()
            .expect("as")
            .success(),
        "assemble start.s"
    );
    o
}

fn cc_obj(dir: &Path, name: &str, body: &str) -> PathBuf {
    let c = dir.join(format!("{name}.c"));
    let o = dir.join(format!("{name}.o"));
    std::fs::write(&c, body).unwrap();
    assert!(
        Command::new("cc")
            .args(["-c", "-fno-pic", "-ffreestanding", "-O2", "-o"])
            .arg(&o)
            .arg(&c)
            .status()
            .expect("cc")
            .success(),
        "compile {name}.c"
    );
    o
}

fn peony(out: &Path, args: &[&str]) -> bool {
    Command::new(PEONY)
        .arg("-o")
        .arg(out)
        .args(args)
        .status()
        .expect("run peony")
        .success()
}

/// `-r` merges two objects (one calling the other) into a valid `ET_REL` that
/// re-links into a correct executable.
#[test]
fn relocatable_merges_and_relinks() {
    let dir = workdir("reloc-r");
    let start = assemble_start(&dir);
    // `a` calls `b` (a cross-object PLT32/PC32 relocation that must survive `-r`).
    let a = cc_obj(
        &dir,
        "a",
        "extern int b(void);\nint a(void){ return 10 + b(); }\n",
    );
    let b = cc_obj(&dir, "b", "int b(void){ return 5; }\n");

    let combined = dir.join("combined.o");
    assert!(
        peony(&combined, &["-r", a.to_str().unwrap(), b.to_str().unwrap()]),
        "peony -r failed"
    );

    // It must be a relocatable object, with the merged symbols and the kept
    // cross-object relocation.
    let hdr = readelf(&combined, &["-h"]);
    assert!(
        hdr.contains("REL (Relocatable file)"),
        "combined.o should be ET_REL:\n{hdr}"
    );
    let syms = readelf(&combined, &["-sW"]);
    assert!(
        syms.contains(" a") && syms.contains(" b"),
        "merged symtab missing a/b:\n{syms}"
    );
    let relocs = readelf(&combined, &["-rW"]);
    assert!(
        relocs.contains(" b "),
        "kept relocation to b missing:\n{relocs}"
    );

    // The non-negotiable check: re-link the merged object into an executable
    // and run it. `a()` = 10 + `b()` = 10 + 5 = 15.
    let exe = dir.join("app");
    assert!(
        peony(&exe, &[start.to_str().unwrap(), combined.to_str().unwrap()]),
        "re-link of combined.o failed"
    );
    assert_eq!(run(&exe), 15, "merged-then-relinked program must exit 15");
}

/// `peony -r a.o` on a single object round-trips (a trivial partial link).
#[test]
fn relocatable_single_object_round_trips() {
    let dir = workdir("reloc-r1");
    let start = assemble_start(&dir);
    let a = cc_obj(
        &dir,
        "a",
        "static int helper(void){ return 9; }\nint a(void){ return helper() + 33; }\n",
    );
    let combined = dir.join("combined.o");
    assert!(
        peony(&combined, &["-r", a.to_str().unwrap()]),
        "peony -r failed"
    );
    assert!(readelf(&combined, &["-h"]).contains("REL (Relocatable file)"));

    let exe = dir.join("app");
    assert!(
        peony(&exe, &[start.to_str().unwrap(), combined.to_str().unwrap()]),
        "re-link failed"
    );
    assert_eq!(run(&exe), 42, "9 + 33");
}
