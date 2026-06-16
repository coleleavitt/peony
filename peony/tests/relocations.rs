//! Per-relocation-type tests. Each computes a value through one relocation kind
//! so a miscalculation surfaces as a wrong exit code.

mod common;
use common::*;

/// R_X86_64_64 + R_X86_64_PC32: pointer table walked at runtime.
#[test]
fn abs64_and_pc32() {
    let dir = workdir("r64");
    let o = assemble(
        &dir,
        "r64",
        "
        .text
        .globl _start
    _start:
        leaq table(%rip), %rax    # PC32 (RIP-relative)
        movq (%rax), %rcx         # *table == &answer  (R64)
        movl (%rcx), %edi
        movl $60, %eax
        syscall
        .data
    table:
        .quad answer              # R_X86_64_64
    answer:
        .long 42
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// R_X86_64_PC64: cross-object PC-relative 64-bit difference.
#[test]
fn pc64_cross_object() {
    let dir = workdir("pc64");
    let a = assemble(
        &dir,
        "a",
        "
        .data
        .globl gptr
    gptr:
        .quad target - gptr       # R_X86_64_PC64
        .text
        .globl _start
    _start:
        leaq gptr(%rip), %rax
        movq (%rax), %rcx
        addq %rcx, %rax           # &gptr + (target - gptr) == &target
        movl (%rax), %edi
        movl $60, %eax
        syscall
    ",
    );
    let b = assemble(&dir, "b", ".data\n.globl target\ntarget:\n .long 42\n");
    let exe = dir.join("a.out");
    link(&exe, &[a, b], &[]);
    assert_eq!(run(&exe), 42);
}

/// R_X86_64_8: a one-byte absolute relocation to a small (defsym) symbol.
#[test]
fn byte_reloc_r8() {
    let dir = workdir("r8");
    let o = assemble(
        &dir,
        "r8",
        "
        .text
        .globl _start
    _start:
        movzbl b(%rip), %edi
        movl $60, %eax
        syscall
        .data
    b:
        .byte small               # R_X86_64_8 absolute
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--defsym", "small=7"]);
    assert_eq!(run(&exe), 7);
}

/// Many relocations across two objects: PLT32 call + GOTPCREL + PC32 data,
/// summed so any single wrong relocation changes the result.
#[test]
fn combined_relocations() {
    let dir = workdir("combined");
    let main = assemble(
        &dir,
        "main",
        "
        .text
        .globl _start
    _start:
        call get20@PLT            # PLT32 -> 20
        movl %eax, %ebx
        movq twentytwo@GOTPCREL(%rip), %rax  # GOTPCREL
        addl (%rax), %ebx         # + 22  => 42
        movl %ebx, %edi
        movl $60, %eax
        syscall
    ",
    );
    let lib = assemble(
        &dir,
        "lib",
        "
        .text
        .globl get20
    get20:
        movl $20, %eax
        ret
        .data
        .globl twentytwo
    twentytwo:
        .long 22
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[main, lib], &[]);
    assert_eq!(run(&exe), 42);
}
