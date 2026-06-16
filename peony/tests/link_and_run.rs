//! Core "it links and runs" tests (GAP_ANALYSIS §7): each assembles real
//! fixtures, links with peony, executes, and asserts the exit code.

mod common;
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
