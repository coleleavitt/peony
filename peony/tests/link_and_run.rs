//! Core "it links and runs" tests (GAP_ANALYSIS §7): each assembles real
//! fixtures, links with peony, executes, and asserts the exit code.

mod common;

use std::process::Command;

use common::*;

/// Freestanding `exit(42)` — no relocations. Validates header/segment/entry.
#[test]
fn freestanding_exit() {
    let dir = workdir("exit");
    let o = assemble(
        &dir,
        "exit",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// PC32 (RIP-relative) + R_X86_64_64 (absolute pointer in `.data`).
#[test]
fn data_relocations() {
    let dir = workdir("data");
    let o = assemble(
        &dir,
        "data",
        "
        .text
        .globl _start
    _start:
        movq ptr(%rip), %rax
        movl (%rax), %edi
        movl $60, %eax
        syscall
        .data
    ptr:
        .quad answer
    answer:
        .long 42
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// R_X86_64_32S absolute load from `.rodata` (read-only segment).
#[test]
fn rodata_absolute() {
    let dir = workdir("rodata");
    let o = assemble(
        &dir,
        "rodata",
        ".text\n.globl _start\n_start:\n mov val, %edi\n movl $60,%eax\n syscall\n.section .rodata\nval:\n .long 42\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Cross-object PLT32 call + GOTPCREL data load (synthetic `.got`).
#[test]
fn multi_object_plt_and_got() {
    let dir = workdir("multi");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call get_answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let lib = assemble(
        &dir,
        "lib",
        ".text\n.globl get_answer\nget_answer:\n movq the_answer@GOTPCREL(%rip), %rax\n movl (%rax), %eax\n ret\n.data\n.globl the_answer\nthe_answer:\n .long 42\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[main, lib], &[]);
    assert_eq!(run(&exe), 42);
}

/// `.bss` (NOBITS): store then load — validates p_memsz > p_filesz.
#[test]
fn bss_section() {
    let dir = workdir("bss");
    let o = assemble(
        &dir,
        "bss",
        ".text\n.globl _start\n_start:\n movl $42, slot(%rip)\n movl slot(%rip), %edi\n movl $60,%eax\n syscall\n.bss\nslot:\n .skip 4\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Linking against a static archive; only the needed member is pulled in.
#[test]
fn static_archive() {
    let dir = workdir("archive");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let used = assemble(
        &dir,
        "used",
        ".text\n.globl answer\nanswer:\n movl $42,%eax\n ret\n",
    );
    let unused = assemble(&dir, "unused", ".text\n.globl never\nnever:\n ret\n");
    let ar = archive(&dir, "libstuff", &[used, unused]);
    let exe = dir.join("a.out");
    link(&exe, &[main, ar], &[]);
    assert_eq!(run(&exe), 42);
}

#[test]
fn whole_archive_includes_unreferenced_members() {
    let dir = workdir("whole_archive");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let used = assemble(
        &dir,
        "used",
        ".text\n.globl answer\nanswer:\n movl $42,%eax\n ret\n",
    );
    let unused = assemble(
        &dir,
        "unused",
        ".text\n.globl never\nnever:\n movl $7,%eax\n ret\n",
    );
    let ar = archive(&dir, "libwhole", &[used, unused]);
    let exe = dir.join("a.out");
    let out = Command::new(PEONY)
        .arg("-o")
        .arg(&exe)
        .arg(&main)
        .arg("--whole-archive")
        .arg(&ar)
        .arg("--no-whole-archive")
        .output()
        .expect("run peony");
    assert!(
        out.status.success(),
        "peony link failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-sW"]);
    assert!(
        syms.contains("never"),
        "--whole-archive should include unreferenced member:\n{syms}"
    );
}

#[test]
fn start_lib_members_are_lazy_objects() {
    let dir = workdir("start_lib");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let used = assemble(
        &dir,
        "used",
        ".text\n.globl answer\nanswer:\n movl $42,%eax\n ret\n",
    );
    let unused = assemble(
        &dir,
        "unused",
        ".text\n.globl never\nnever:\n movl $7,%eax\n ret\n",
    );
    let exe = dir.join("a.out");
    let out = Command::new(PEONY)
        .arg("-o")
        .arg(&exe)
        .arg(&main)
        .arg("--start-lib")
        .arg(&used)
        .arg(&unused)
        .arg("--end-lib")
        .output()
        .expect("run peony");
    assert!(
        out.status.success(),
        "peony link failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-sW"]);
    assert!(
        !syms.contains("never"),
        "--start-lib should not include unreferenced object:\n{syms}"
    );
}

#[test]
fn require_defined_pulls_archive_member() {
    let dir = workdir("require_defined");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n movl $42,%edi\n movl $60,%eax\n syscall\n",
    );
    let forced = assemble(&dir, "forced", ".text\n.globl forced\nforced:\n ret\n");
    let ar = archive(&dir, "libforced", &[forced]);
    let exe = dir.join("a.out");
    link(&exe, &[main, ar], &["--require-defined", "forced"]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-sW"]);
    assert!(
        syms.contains("forced"),
        "--require-defined should seed archive pull:\n{syms}"
    );
}

#[test]
fn relocatable_handoff_produces_linkable_object() {
    let dir = workdir("relocatable_handoff");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let used = assemble(
        &dir,
        "used",
        ".text\n.globl answer\nanswer:\n movl $42,%eax\n ret\n",
    );
    let combined = dir.join("combined.o");
    let out = Command::new(PEONY)
        .args(["-r", "-o"])
        .arg(&combined)
        .arg(&main)
        .arg(&used)
        .output()
        .expect("run peony -r");
    assert!(
        out.status.success(),
        "peony -r failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let hdr = readelf(&combined, &["-h"]);
    assert!(
        hdr.contains("REL (Relocatable file)"),
        "-r output should be ET_REL:\n{hdr}"
    );

    let exe = dir.join("a.out");
    link(&exe, &[combined], &[]);
    assert_eq!(run(&exe), 42);
}

#[test]
fn weak_undefined_does_not_pull_archive_member() {
    let dir = workdir("archive_weak_undef");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n movl $42,%edi\n movl $60,%eax\n syscall\n.weak foo\n",
    );
    let unused = assemble(
        &dir,
        "unused",
        ".text\n.globl foo\nfoo:\n ret\n.globl _start\n_start:\n movl $1,%edi\n movl $60,%eax\n syscall\n",
    );
    let ar = archive(&dir, "libweak", &[unused]);
    let exe = dir.join("a.out");
    link(&exe, &[main, ar], &[]);
    assert_eq!(run(&exe), 42);
}

/// Cross-archive cyclic dependency: archive A's member calls a symbol defined in
/// archive B, and B's member calls back into A. Pulling A's member introduces a
/// new undefined ref (into B), which must be satisfied in a LATER fixpoint round,
/// and pulling B's member introduces a ref back into A. This exercises the
/// incremental-undefined-set archive fixpoint (the O(N²)→O(N) rewrite) across
/// multiple rounds — if the incremental set tracking drops a newly-introduced
/// undef, the link fails with an undefined symbol. The program returns
/// f(g(0)) where f adds 7 (in A) and g adds 35 (in B) → 42.
#[test]
fn cross_archive_cyclic_dependency() {
    let dir = workdir("archive_cycle");
    // _start calls f (archive A). f calls g (archive B). g calls h (archive A).
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n xor %edi,%edi\n call f@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    // Archive A: f (adds 7 then calls g), h (adds 0). f introduces an undef `g`
    // that only archive B satisfies → forces a second round.
    let af = assemble(
        &dir,
        "af",
        ".text\n.globl f\nf:\n addl $7,%edi\n call g@PLT\n ret\n.globl h\nh:\n ret\n",
    );
    let a = archive(&dir, "libA", std::slice::from_ref(&af));
    // Archive B: g (adds 35 then calls h back in A). g introduces undef `h`.
    let bg = assemble(
        &dir,
        "bg",
        ".text\n.globl g\ng:\n addl $35,%edi\n call h@PLT\n movl %edi,%eax\n ret\n",
    );
    let b = archive(&dir, "libB", std::slice::from_ref(&bg));
    let exe = dir.join("a.out");
    // Order A then B; the cycle forces the fixpoint to iterate.
    link(&exe, &[main, a, b], &[]);
    assert_eq!(
        run(&exe),
        42,
        "cyclic archive deps must resolve across rounds"
    );
}

/// Incremental: no-change relink reuses; an input change triggers a correct relink.
#[test]
fn incremental_reuse_and_relink() {
    let dir = workdir("incr");
    let src = ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n";
    let o = assemble(&dir, "prog", src);
    let exe = dir.join("a.out");

    let o1 = link_raw(&exe, std::slice::from_ref(&o), &["--incremental"]);
    assert!(o1.status.success());
    assert_eq!(run(&exe), 42);

    let o2 = link_raw(&exe, std::slice::from_ref(&o), &["--incremental"]);
    assert!(o2.status.success());
    assert!(
        String::from_utf8_lossy(&o2.stderr).contains("reused cached output"),
        "expected cache reuse"
    );
    assert_eq!(run(&exe), 42);

    let changed = assemble(&dir, "prog", &src.replace("$42", "$7"));
    let o3 = link_raw(&exe, std::slice::from_ref(&changed), &["--incremental"]);
    assert!(o3.status.success());
    assert!(
        !String::from_utf8_lossy(&o3.stderr).contains("reused cached output"),
        "should have relinked after a change"
    );
    assert_eq!(run(&exe), 7);
}
