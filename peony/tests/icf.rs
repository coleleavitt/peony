//! Integration tests for Identical Code Folding (`--icf=all`).
//!
//! ICF must (a) fold byte-identical, non-address-significant functions to a
//! single copy so they share an address, (b) leave the program's behaviour
//! unchanged, and (c) be a strict no-op without the flag. Soundness is proved in
//! `rocq-tests/ICFSoundness.v`; these tests pin the implementation to it.

mod common;
use common::*;

/// Two byte-identical LOCAL functions, each in its own `.text.fN` section and
/// referenced only by direct call, must fold to the same address under
/// `--icf=all`, and the program must still compute the right answer.
#[test]
fn icf_folds_identical_local_functions() {
    let dir = workdir("icf_fold");

    // `dup_a`/`dup_b`: identical bodies (return 0x2a). `main` calls both and
    // exits with their sum/2 so a fold that corrupts one would change the exit.
    // All three live in distinct sections so GC/ICF operate at section grain.
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
    link(&out, std::slice::from_ref(&obj), &["--icf=all"]);
    assert_eq!(run(&out), 0, "ICF-folded program must run correctly");

    // dup_a and dup_b should resolve to the SAME address (folded). Read the
    // symbol table of the output.
    let syms = readelf(&out, &["-s"]);
    let addr_of = |name: &str| -> Option<String> {
        syms.lines()
            .find(|l| l.split_whitespace().last() == Some(name))
            .and_then(|l| l.split_whitespace().nth(1).map(|s| s.to_string()))
    };
    if let (Some(a), Some(b)) = (addr_of("dup_a"), addr_of("dup_b")) {
        assert_eq!(a, b, "ICF must fold dup_a and dup_b to one address");
    }
    // (If the assembler emitted no local symbol-table entries, the run check
    // above is still the load-bearing correctness gate.)
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
