//! Feature tests: --gc-sections, custom entry, alignment, many objects,
//! differential vs GNU ld, and ELF structural validity.

mod common;
use common::*;

const GC_SRC: &str = "
    .section .text.used,\"ax\",@progbits
    .globl used
used:
    movl $42, %eax
    ret
    .section .text.unused,\"ax\",@progbits
    .globl unused
unused:
    movl $99, %eax
    ret
    .text
    .globl _start
_start:
    call used
    movl %eax, %edi
    movl $60, %eax
    syscall
";

/// --gc-sections removes a function unreachable from the entry point.
#[test]
fn gc_drops_unused() {
    let dir = workdir("gc_drop");
    let o = assemble(&dir, "gc", GC_SRC);
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--gc-sections"]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-s"]);
    assert!(!syms.contains("unused"), "gc should have dropped `unused`");
    assert!(syms.contains("used"), "`used` must be kept");
}

/// Without --gc-sections, the unused function is retained.
#[test]
fn no_gc_keeps_unused() {
    let dir = workdir("gc_keep");
    let o = assemble(&dir, "gc", GC_SRC);
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
    assert!(readelf(&exe, &["-s"]).contains("unused"));
}

/// Transitively-referenced sections survive GC.
#[test]
fn gc_keeps_referenced_chain() {
    let dir = workdir("gc_chain");
    let o = assemble(
        &dir,
        "chain",
        "
        .section .text.b,\"ax\",@progbits
        .globl bee
    bee:
        movl $42, %eax
        ret
        .section .text.a,\"ax\",@progbits
        .globl ay
    ay:
        call bee
        ret
        .text
        .globl _start
    _start:
        call ay
        movl %eax, %edi
        movl $60, %eax
        syscall
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--gc-sections"]);
    assert_eq!(run(&exe), 42); // bee reached transitively via ay
}

/// Custom entry symbol via `--entry`.
#[test]
fn custom_entry() {
    let dir = workdir("entry");
    let o = assemble(
        &dir,
        "entry",
        ".text\n.globl mymain\nmymain:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--entry", "mymain"]);
    assert_eq!(run(&exe), 42);
    assert!(readelf(&exe, &["-h"]).contains("Entry point"));
}

/// An over-aligned section gets a correctly aligned address.
#[test]
fn section_alignment() {
    let dir = workdir("align");
    let o = assemble(
        &dir,
        "align",
        "
        .text
        .globl _start
    _start:
        movl aligned, %edi
        movl $60, %eax
        syscall
        .section .data
        .p2align 12
        .globl aligned
    aligned:
        .long 42
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
    // `aligned` must sit on a 4 KiB boundary.
    let syms = readelf(&exe, &["-s"]);
    let line = syms
        .lines()
        .find(|l| l.ends_with(" aligned"))
        .expect("aligned symbol");
    let val = u64::from_str_radix(line.split_whitespace().nth(1).unwrap(), 16).unwrap();
    assert_eq!(val % 0x1000, 0, "symbol not 4 KiB aligned: {val:#x}");
}

/// Many objects, each contributing to the result.
#[test]
fn many_objects() {
    let dir = workdir("many");
    let mut objs = Vec::new();
    // six adders of value 7 each => 42
    for i in 0..6 {
        objs.push(assemble(
            &dir,
            &format!("add{i}"),
            &format!(".text\n.globl add{i}\nadd{i}:\n addl $7, %edi\n ret\n"),
        ));
    }
    let mut main = String::from(".text\n.globl _start\n_start:\n xorl %edi, %edi\n");
    for i in 0..6 {
        main.push_str(&format!(" call add{i}@PLT\n"));
    }
    main.push_str(" movl $60, %eax\n syscall\n");
    objs.insert(0, assemble(&dir, "main", &main));
    let exe = dir.join("a.out");
    link(&exe, &objs, &[]);
    assert_eq!(run(&exe), 42);
}

/// Differential: peony and GNU ld produce binaries with the same exit code.
#[test]
fn differential_vs_ld() {
    let dir = workdir("diff");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call val@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let lib = assemble(
        &dir,
        "lib",
        ".text\n.globl val\nval:\n movl $42,%eax\n ret\n",
    );

    let peony_exe = dir.join("peony.out");
    link(&peony_exe, &[main.clone(), lib.clone()], &[]);
    let peony_rc = run(&peony_exe);

    let ld_exe = dir.join("ld.out");
    assert!(ld_link(&ld_exe, &[main, lib]), "ld link failed");
    let ld_rc = run(&ld_exe);

    assert_eq!(peony_rc, 42);
    assert_eq!(peony_rc, ld_rc, "peony and ld disagree");
}

/// COMDAT group deduplication: the same group in two objects is kept once
/// (otherwise the shared symbol would be a duplicate-definition error).
#[test]
fn comdat_group_dedup() {
    let dir = workdir("comdat");
    let grp = ".section .text.inl,\"axG\",@progbits,inl,comdat\n.weak inl\n.globl inl\ninl:\n movl $42,%eax\n ret\n";
    let a = assemble(
        &dir,
        "a",
        &format!("{grp}.text\n.globl afn\nafn:\n call inl\n ret\n"),
    );
    let b = assemble(
        &dir,
        "b",
        &format!(
            "{grp}.text\n.globl _start\n_start:\n call inl\n movl %eax,%edi\n movl $60,%eax\n syscall\n"
        ),
    );
    let exe = dir.join("a.out");
    link(&exe, &[a, b], &[]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-s"]);
    let count = syms.lines().filter(|l| l.ends_with(" inl")).count();
    assert_eq!(count, 1, "COMDAT should keep exactly one `inl`");
}

/// `--build-id` emits a `.note.gnu.build-id` (+ PT_NOTE) and is deterministic.
#[test]
fn build_id_note() {
    let dir = workdir("buildid");
    let src = ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n";
    let o = assemble(&dir, "b", src);

    let exe1 = dir.join("a.out");
    link(&exe1, std::slice::from_ref(&o), &["--build-id"]);
    assert_eq!(run(&exe1), 42);

    let notes = readelf(&exe1, &["-n"]);
    assert!(
        notes.contains("Build ID:"),
        "build-id note missing: {notes}"
    );
    assert!(readelf(&exe1, &["-l"]).contains("NOTE"), "PT_NOTE missing");

    // Deterministic: same inputs → identical build-id.
    let exe2 = dir.join("b.out");
    link(&exe2, std::slice::from_ref(&o), &["--build-id"]);
    let id = |s: &str| {
        s.lines()
            .find(|l| l.contains("Build ID:"))
            .map(|l| l.trim().to_string())
    };
    assert_eq!(
        id(&notes),
        id(&readelf(&exe2, &["-n"])),
        "build-id not deterministic"
    );
}

/// `-L <dir> -l <name>` resolves `lib<name>.a` on the search path.
#[test]
fn library_search_flags() {
    let dir = workdir("lflag");
    let foo = assemble(
        &dir,
        "foo",
        ".text\n.globl foo\nfoo:\n movl $42,%eax\n ret\n",
    );
    archive(&dir, "libfoo", &[foo]); // → libfoo.a in `dir`
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call foo@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[main], &["-L", dir_str, "-l", "foo"]);
    assert_eq!(run(&exe), 42);
}

/// `-s` / `--strip-all` omits the symbol table while keeping a runnable binary.
#[test]
fn strip_all() {
    let dir = workdir("strip");
    let o = assemble(
        &dir,
        "s",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["-s"]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-s"]);
    assert!(
        !syms.contains(" _start"),
        "symtab should be stripped: {syms}"
    );
}

/// `--pie` produces an `ET_DYN` position-independent executable that the kernel
/// loads at a randomized base and runs correctly (PC-relative code).
#[test]
fn pie_executable() {
    let dir = workdir("pie");
    let o = assemble(
        &dir,
        "p",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--pie"]);
    assert!(readelf(&exe, &["-h"]).contains("DYN"), "should be ET_DYN");
    assert_eq!(run(&exe), 42);
}

/// A PIE with RIP-relative data access runs correctly under load bias.
#[test]
fn pie_pc_relative_data() {
    let dir = workdir("pie_data");
    let o = assemble(
        &dir,
        "pd",
        ".text\n.globl _start\n_start:\n movl val(%rip), %edi\n movl $60,%eax\n syscall\n.section .rodata\nval:\n .long 42\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--pie"]);
    assert_eq!(run(&exe), 42);
}

/// The output is a well-formed ET_EXEC x86-64 ELF with loadable segments,
/// and is marked executable on disk.
#[test]
fn valid_elf_structure() {
    let dir = workdir("valid");
    let o = assemble(
        &dir,
        "v",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);

    let h = readelf(&exe, &["-h"]);
    assert!(h.contains("EXEC"), "should be ET_EXEC");
    assert!(h.contains("X86-64"), "should be x86-64");

    let l = readelf(&exe, &["-l"]);
    assert!(l.contains("LOAD"), "should have PT_LOAD segments");
    assert!(l.contains("R E"), "should have an executable segment");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "output must be executable");
    }
    assert_eq!(run(&exe), 42);
}
