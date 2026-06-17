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

/// A `.tbss` variable with alignment GREATER than `.tdata`'s forces the static
/// TLS block base to be over-aligned. peony must align the TLS segment's
/// `p_vaddr` to the MAX TLS-section alignment, or the highly-aligned variable
/// lands at the wrong address (the bug that crashed large TLS-heavy binaries
/// like a self-hosted toolchain — "TLS during destruction" in std). Verified by
/// reading both values back through libc + a real `__thread` access.
#[test]
fn tls_overaligned_block_base() {
    let dir = workdir("tls_align");
    let Some(exe) = cc_b(
        &dir,
        "talign",
        &[(
            "t.c",
            "#include <stdio.h>\n\
             __thread int small = 5;\n\
             __thread _Alignas(128) long big[4] = {11, 22, 33, 44};\n\
             int main(void) {\n\
                 // Touch both; the over-aligned `big` must be readable/correct.\n\
                 if (small != 5) return 1;\n\
                 for (int i = 0; i < 4; i++) if (big[i] != 11 * (i + 1)) return 2;\n\
                 // Its runtime address must satisfy the 128-byte alignment.\n\
                 if (((unsigned long)&big[0] & 127) != 0) return 3;\n\
                 printf(\"tls-align ok\\n\");\n\
                 return 42;\n\
             }\n",
        )],
        false,
    ) else {
        eprintln!("skipping: toolchain unavailable");
        return;
    };
    // PT_TLS p_vaddr must be aligned to the block's max alignment (>= 128).
    let phdrs = readelf(&exe, &["-lW"]);
    let tls_line = phdrs
        .lines()
        .find(|l| l.trim_start().starts_with("TLS"))
        .expect("PT_TLS present");
    // Columns: TLS off vaddr paddr filesz memsz flg align
    let cols: Vec<&str> = tls_line.split_whitespace().collect();
    let vaddr = u64::from_str_radix(cols[2].trim_start_matches("0x"), 16).unwrap();
    let align = parse_align(cols.last().unwrap());
    assert!(align >= 128, "PT_TLS align {align} should be >= 128");
    assert_eq!(
        vaddr % align,
        0,
        "PT_TLS vaddr {vaddr:#x} not aligned to {align}"
    );
    assert_eq!(run(&exe), 42);
}

/// Parse a program-header alignment field, which readelf prints as either a hex
/// (`0x80`) or decimal value depending on version.
fn parse_align(s: &str) -> u64 {
    if let Some(hex) = s.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).unwrap_or(1)
    } else {
        s.parse().unwrap_or(1)
    }
}
