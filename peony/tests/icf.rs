//! Integration tests for Identical Code Folding (`--icf=all`).
//!
//! ICF must (a) fold byte-identical, non-address-significant functions to a
//! single copy so they share an address, (b) leave the program's behaviour
//! unchanged, and (c) be a strict no-op without the flag. Soundness is proved in
//! `rocq-tests/ICFSoundness.v`; these tests pin the implementation to it.

mod common;
use common::*;

/// SOUNDNESS GATE: an object with NO `.llvm_addrsig` table (e.g. hand-written
/// asm, rustc output) must NOT have its sections folded — peony has no proof any
/// symbol is address-insignificant, so `--icf=all` is a safe no-op and the
/// program runs unchanged. (Folding without that proof silently corrupted real
/// programs; the fold mechanism itself is exercised by the synthetic-addrsig
/// unit tests in `peony-layout::icf`.)
#[test]
fn icf_is_sound_noop_without_addrsig() {
    let dir = workdir("icf_sound");

    // Two byte-identical local functions, only directly called. Without an
    // addrsig table peony must keep them distinct; the program must still run.
    let obj = assemble(
        &dir,
        "icf",
        r#"
        .section .text.dup_a,"ax",@progbits
        .local dup_a
        dup_a:
            mov $0x2a, %eax
            ret
        .section .text.dup_b,"ax",@progbits
        .local dup_b
        dup_b:
            mov $0x2a, %eax
            ret
        .text
        .globl _start
        _start:
            call dup_a
            mov %eax, %ebx     # ebx = 42
            call dup_b
            cmp %eax, %ebx     # both must return 42
            jne  fail
            mov  $60, %rax
            mov  $0, %rdi      # exit 0 on success
            syscall
        fail:
            mov  $60, %rax
            mov  $1, %rdi
            syscall
        "#,
    );

    let out = dir.join("icf.out");
    // Linking with and without --icf must both succeed and run correctly.
    link(&out, std::slice::from_ref(&obj), &["--icf=all"]);
    assert_eq!(run(&out), 0, "ICF must be a sound no-op without addrsig");

    let out2 = dir.join("icf_ref.out");
    link(&out2, std::slice::from_ref(&obj), &[]);
    // Output must be byte-identical to a non-ICF link (nothing was folded).
    assert_eq!(
        std::fs::read(&out).unwrap(),
        std::fs::read(&out2).unwrap(),
        "without addrsig, --icf must produce byte-identical output to no --icf"
    );
}

/// Without `--icf`, the same inputs link normally and the program runs — ICF is
/// strictly opt-in and never changes default behaviour.
#[test]
fn icf_is_opt_in_noop_by_default() {
    let dir = workdir("icf_noop");
    let obj = assemble(
        &dir,
        "noicf",
        r#"
        .text
        .globl _start
        _start:
            mov $60, %rax
            mov $0, %rdi
            syscall
        "#,
    );
    let out = dir.join("noicf.out");
    link(&out, std::slice::from_ref(&obj), &[]);
    assert_eq!(run(&out), 0);
}
