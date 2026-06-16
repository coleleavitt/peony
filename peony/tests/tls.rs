//! Thread-Local Storage (Local-Exec model) tests. A freestanding program sets up
//! `%fs` itself (`arch_prctl`) and accesses a `.tbss` variable via `@tpoff`
//! (`R_X86_64_TPOFF32`); peony must emit `PT_TLS` and compute the TP offset.

mod common;
use common::*;

#[test]
fn tls_local_exec() {
    let dir = workdir("tls");
    let o = assemble(
        &dir,
        "t",
        "
        .section .tbss,\"awT\",@nobits
        .globl tv
        .align 4
    tv:
        .zero 4
        .text
        .globl _start
    _start:
        leaq buf(%rip), %rsi
        addq $4096, %rsi          # TP = buf+4096; TP + tpoff lands inside buf
        movl $158, %eax           # arch_prctl
        movl $0x1002, %edi        # ARCH_SET_FS
        syscall
        movl $42, %fs:tv@tpoff    # R_X86_64_TPOFF32
        movl %fs:tv@tpoff, %edi
        movl $60, %eax
        syscall
        .bss
        .align 64
    buf:
        .zero 8192
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert!(readelf(&exe, &["-l"]).contains("TLS"), "missing PT_TLS");
    assert_eq!(run(&exe), 42);
}

/// Two TLS variables in `.tbss` get distinct TP offsets.
#[test]
fn tls_two_vars() {
    let dir = workdir("tls2");
    let o = assemble(
        &dir,
        "t",
        "
        .section .tbss,\"awT\",@nobits
        .globl a
        .globl b
        .align 4
    a:
        .zero 4
    b:
        .zero 4
        .text
        .globl _start
    _start:
        leaq buf(%rip), %rsi
        addq $4096, %rsi
        movl $158, %eax
        movl $0x1002, %edi
        syscall
        movl $40, %fs:a@tpoff
        movl $2,  %fs:b@tpoff
        movl %fs:a@tpoff, %edi
        addl %fs:b@tpoff, %edi    # 40 + 2 = 42
        movl $60, %eax
        syscall
        .bss
        .align 64
    buf:
        .zero 8192
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}
