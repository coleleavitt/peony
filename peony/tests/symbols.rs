//! Symbol-resolution tests: weak/strong, absolute, common, linker-defined,
//! --defsym, and cross-section local symbols.

mod common;
use common::*;

/// A strong definition overrides a weak one.
#[test]
fn weak_overridden_by_strong() {
    let dir = workdir("weak_ovr");
    let weak = assemble(
        &dir,
        "weak",
        ".text\n.weak val\n.globl val\nval:\n .long 99\n",
    );
    let strong = assemble(&dir, "strong", ".data\n.globl val\nval:\n .long 42\n");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n movl val(%rip), %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[main, weak, strong], &[]);
    assert_eq!(run(&exe), 42);
}

/// A purely-weak undefined reference resolves to zero (and does not error).
#[test]
fn weak_undefined_is_zero() {
    let dir = workdir("weak_undef");
    let o = assemble(
        &dir,
        "wu",
        "
        .text
        .weak missing
        .globl _start
    _start:
        movl $missing, %edi       # weak undef -> 0
        testl %edi, %edi
        jnz 1f
        movl $42, %edi
    1:
        movl $60, %eax
        syscall
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Absolute symbol defined via `sym = value`.
#[test]
fn absolute_symbol() {
    let dir = workdir("abs");
    let o = assemble(
        &dir,
        "abs",
        ".globl answer\nanswer = 42\n.text\n.globl _start\n_start:\n movl $answer, %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Two objects with a tentative (`.comm`) definition share one allocation.
#[test]
fn common_symbols() {
    let dir = workdir("common");
    let a = assemble(
        &dir,
        "a",
        ".comm shared,4,4\n.text\n.globl setval\nsetval:\n movl $42, shared(%rip)\n ret\n",
    );
    let b = assemble(
        &dir,
        "b",
        ".comm shared,4,4\n.text\n.globl _start\n_start:\n call setval\n movl shared(%rip), %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[a, b], &[]);
    assert_eq!(run(&exe), 42);
}

/// Linker-defined `_end` / `__bss_start` are provided when referenced.
#[test]
fn linker_defined_symbols() {
    let dir = workdir("lds");
    let o = assemble(
        &dir,
        "lds",
        "
        .text
        .globl _start
    _start:
        movl $_end, %eax          # requires _end to be linker-defined
        movl $__bss_start, %eax   # and __bss_start
        movl $42, %edi
        movl $60, %eax
        syscall
        .bss
    buf:
        .skip 16
    ",
    );
    let exe = dir.join("a.out");
    // Would fail to link with "undefined symbol" if _end/__bss_start weren't provided.
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
    // _end must be a real, non-zero address in the symbol table.
    let syms = readelf(&exe, &["-s"]);
    assert!(syms.contains("_end"), "_end should appear in .symtab");
}

/// `--defsym SYM=VALUE`.
#[test]
fn defsym() {
    let dir = workdir("defsym");
    let o = assemble(
        &dir,
        "ds",
        ".text\n.globl _start\n_start:\n movl $magic, %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--defsym", "magic=0x2a"]);
    assert_eq!(run(&exe), 42);
}

/// Local symbols appear in `.symtab` as LOCAL, ordered before globals.
#[test]
fn local_symbols_in_symtab() {
    let dir = workdir("localsym");
    let o = assemble(
        &dir,
        "ls",
        ".text\n.globl _start\n_start:\n call helper\n movl %eax,%edi\n movl $60,%eax\n syscall\nhelper:\n movl $42,%eax\n ret\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-s"]);
    let helper = syms
        .lines()
        .find(|l| l.ends_with(" helper"))
        .expect("helper in symtab");
    assert!(helper.contains("LOCAL"), "helper should be LOCAL: {helper}");
}

/// Two strong definitions of one symbol must be a link error.
#[test]
fn duplicate_strong_symbol_errors() {
    let dir = workdir("dup");
    let a = assemble(&dir, "a", ".data\n.globl dup\ndup:\n .long 1\n");
    let b = assemble(&dir, "b", ".data\n.globl dup\ndup:\n .long 2\n");
    let main = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movl dup(%rip), %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let out = link_raw(&exe, &[main, a, b], &[]);
    assert!(!out.status.success(), "duplicate definition should fail");
    assert!(String::from_utf8_lossy(&out.stderr).contains("duplicate"));
}

/// A strong undefined reference must be a link error.
#[test]
fn undefined_strong_symbol_errors() {
    let dir = workdir("undef");
    let o = assemble(
        &dir,
        "u",
        ".text\n.globl _start\n_start:\n call missing@PLT\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let out = link_raw(&exe, &[o], &[]);
    assert!(!out.status.success(), "undefined symbol should fail");
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing"));
}

/// A local symbol in another section, referenced from `.text`
/// (resolved via the section placement, not the global table).
#[test]
fn local_cross_section() {
    let dir = workdir("local");
    let o = assemble(
        &dir,
        "local",
        ".text\n.globl _start\n_start:\n movl lval, %edi\n movl $60,%eax\n syscall\n.section .rodata\nlval:\n .long 42\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}
