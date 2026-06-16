//! Additional end-to-end cases covering more real-world scenarios that peony's
//! static linker already handles. Each computes its result through the feature,
//! so a regression surfaces as a wrong exit code.

mod common;
use common::*;

/// Call a function through a pointer stored in `.data` (R_X86_64_64 + indirect call).
#[test]
fn function_pointer_via_data() {
    let dir = workdir("fnptr");
    let o = assemble(
        &dir,
        "fnptr",
        "
        .text
        .globl _start
    _start:
        movq fp(%rip), %rax
        call *%rax
        movl %eax, %edi
        movl $60, %eax
        syscall
    target:
        movl $42, %eax
        ret
        .data
    fp:
        .quad target
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Relocation with a non-zero addend (`sym+8`).
#[test]
fn nonzero_addend() {
    let dir = workdir("addend");
    let o = assemble(
        &dir,
        "addend",
        "
        .text
        .globl _start
    _start:
        movl arr+8(%rip), %edi   # arr[2]
        movl $60, %eax
        syscall
        .data
    arr:
        .long 0, 0, 42
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// One object reads a global variable defined in another object.
#[test]
fn cross_object_data() {
    let dir = workdir("xdata");
    let a = assemble(&dir, "a", ".data\n.globl gvar\ngvar:\n .long 42\n");
    let b = assemble(
        &dir,
        "b",
        ".text\n.globl _start\n_start:\n movl gvar(%rip), %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[a, b], &[]);
    assert_eq!(run(&exe), 42);
}

/// Default entry point `_start` (no `--entry`).
#[test]
fn default_entry() {
    let dir = workdir("dentry");
    let o = assemble(
        &dir,
        "d",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// A hidden-visibility global still resolves within the link.
#[test]
fn hidden_symbol() {
    let dir = workdir("hidden");
    let o = assemble(
        &dir,
        "h",
        ".text\n.hidden hsym\n.globl hsym\nhsym:\n movl $42,%eax\n ret\n.globl _start\n_start:\n call hsym\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// A large `.bss` (NOBITS) is mapped (p_memsz) without bloating the file.
#[test]
fn large_bss() {
    let dir = workdir("lbss");
    let o = assemble(
        &dir,
        "lbss",
        "
        .text
        .globl _start
    _start:
        movl $42, big+65536(%rip)
        movl big+65536(%rip), %edi
        movl $60, %eax
        syscall
        .bss
    big:
        .skip 131072
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
    // .bss must not inflate the on-disk file.
    let file_len = std::fs::metadata(&exe).unwrap().len();
    assert!(file_len < 65536, "bss inflated the file: {file_len} bytes");
}

/// An empty (size-0) section is ignored without breaking layout.
#[test]
fn empty_section_ignored() {
    let dir = workdir("empty");
    let o = assemble(
        &dir,
        "empty",
        ".section .empty,\"a\",@progbits\n.text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Byte read from a `.rodata` string.
#[test]
fn rodata_string_byte() {
    let dir = workdir("rostr");
    let o = assemble(
        &dir,
        "rostr",
        ".text\n.globl _start\n_start:\n movzbl msg, %edi\n movl $60,%eax\n syscall\n.section .rodata\nmsg:\n .ascii \"*\"\n", // '*' == 42
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Archive resolution chain: pulling member A pulls member B that A needs.
#[test]
fn archive_chain() {
    let dir = workdir("achain");
    let a = assemble(&dir, "a", ".text\n.globl afn\nafn:\n call bfn@PLT\n ret\n");
    let b = assemble(&dir, "b", ".text\n.globl bfn\nbfn:\n movl $42,%eax\n ret\n");
    let ar = archive(&dir, "libchain", &[a, b]);
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call afn@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[main, ar], &[]);
    assert_eq!(run(&exe), 42);
}

/// A weak definition that is *used* (no strong override) is kept.
#[test]
fn weak_definition_used() {
    let dir = workdir("weakdef");
    let o = assemble(
        &dir,
        "wd",
        ".text\n.weak wfn\n.globl wfn\nwfn:\n movl $42,%eax\n ret\n.globl _start\n_start:\n call wfn\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
}

/// Incremental link of two inputs: changing one relinks with the new result.
#[test]
fn incremental_partial_change() {
    let dir = workdir("incr2");
    let lib_v = |v: i32| format!(".text\n.globl getval\ngetval:\n movl ${v}, %eax\n ret\n");
    let lib = assemble(&dir, "lib", &lib_v(42));
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call getval@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");

    link(&exe, &[main.clone(), lib], &["--incremental"]);
    assert_eq!(run(&exe), 42);

    let lib2 = assemble(&dir, "lib", &lib_v(7));
    let out = link_raw(&exe, &[main, lib2], &["--incremental"]);
    assert!(out.status.success());
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("reused cached output"),
        "must relink after a changed input"
    );
    assert_eq!(run(&exe), 7);
}
