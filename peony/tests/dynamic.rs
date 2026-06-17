//! Dynamic-linking tests: link against a real `cc -shared` library, let the
//! system `ld.so` resolve the imports at load, and check the program runs.

mod common;
use common::*;

/// Import a data symbol from a shared library (resolved via `R_X86_64_GLOB_DAT`).
#[test]
fn dynamic_data_import() {
    let dir = workdir("dyn_data");
    compile_shared(&dir, "foo", "int forty_two = 42;\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movq forty_two@GOTPCREL(%rip), %rax\n movl (%rax), %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-l", "foo"]);

    // It must be a dynamic executable with a NEEDED entry.
    let d = readelf(&exe, &["-d"]);
    assert!(d.contains("libfoo.so"), "missing DT_NEEDED libfoo.so: {d}");
    assert!(d.contains("(RELA)"), "missing DT_RELA: {d}");

    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

/// A direct reference to DSO data uses executable-owned copy storage plus
/// `R_X86_64_COPY`, not a GOT `GLOB_DAT` import.
#[test]
fn dynamic_copy_relocation_for_direct_data_import() {
    let dir = workdir("dyn_copy");
    compile_shared(&dir, "copy", "int shared_value = 42;\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movl shared_value(%rip), %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-l", "copy"]);

    let relocs = readelf(&exe, &["-rW"]);
    assert!(
        relocs.contains("R_X86_64_COPY") && relocs.contains("shared_value"),
        "missing copy relocation:\n{relocs}"
    );
    let syms = readelf(&exe, &["-sW"]);
    assert!(
        syms.lines().any(|line| {
            line.contains("shared_value")
                && line.contains("OBJECT")
                && line.contains("GLOBAL")
                && !line.contains("UND")
        }),
        "copy symbol should be executable-defined:\n{syms}"
    );
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

/// Import a function from a shared library and call it through the GOT.
#[test]
fn dynamic_function_import() {
    let dir = workdir("dyn_func");
    compile_shared(&dir, "bar", "int get_answer(void){ return 42; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movq get_answer@GOTPCREL(%rip), %rax\n call *%rax\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-l", "bar"]);
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

/// A GNU `ld` linker script (`GROUP(...)`, like the system `libc.so`) is
/// expanded to the real shared object it references.
#[test]
fn linker_script_group() {
    let dir = workdir("ldscript");
    compile_shared(&dir, "real", "int forty_two = 42;\n"); // → libreal.so
    let script = dir.join("libwrap.so");
    std::fs::write(
        &script,
        format!(
            "/* GNU ld script */\nGROUP ( {}/libreal.so )\n",
            dir.display()
        ),
    )
    .unwrap();
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movq forty_two@GOTPCREL(%rip), %rax\n movl (%rax), %edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[m, script], &[]);
    assert_eq!(
        run_env(&exe, &[("LD_LIBRARY_PATH", dir.to_str().unwrap())]),
        42
    );
}

/// Direct `call foo@PLT` to a shared-library function (PLT stub + JUMP_SLOT).
#[test]
fn dynamic_plt_call() {
    let dir = workdir("dyn_plt");
    compile_shared(&dir, "baz", "int compute(void){ return 42; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n call compute@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-l", "baz"]);
    let d = readelf(&exe, &["-d"]);
    assert!(
        d.contains("(JMPREL)") && d.contains("(PLTGOT)"),
        "missing PLT tags: {d}"
    );
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

/// Shared objects keep GNU2 TLSDESC accesses and emit loader-filled descriptor
/// relocations.
#[test]
fn shared_object_emits_tlsdesc_relocation() {
    let dir = workdir("tlsdesc");
    let o = assemble(
        &dir,
        "tlsdesc",
        ".text\n.globl read_tls\nread_tls:\n leaq tls_value@TLSDESC(%rip), %rax\n call *tls_value@TLSCALL(%rax)\n movl (%rax), %eax\n ret\n.section .tdata,\"awT\",@progbits\ntls_value:\n .long 42\n",
    );
    let so = dir.join("libtlsdesc.so");
    link(&so, &[o], &["-shared"]);

    let relocs = readelf(&so, &["-rW"]);
    assert!(
        relocs.contains("R_X86_64_TLSDESC"),
        "missing TLSDESC dynamic relocation:\n{relocs}"
    );
    assert!(
        !relocs.contains("R_X86_64_GOTPC32_TLSDESC"),
        "static TLSDESC relocation leaked into output:\n{relocs}"
    );
}

/// Executables relax TLSDESC to local-exec code and do not carry descriptor
/// relocations.
#[test]
fn executable_relaxes_tlsdesc_to_local_exec() {
    let dir = workdir("tlsdesc_exe");
    let o = assemble(
        &dir,
        "tlsdesc_exe",
        ".text\n.globl _start\n_start:\n leaq tls_first@TLSDESC(%rip), %rax\n call *tls_first@TLSCALL(%rax)\n movl $60,%eax\n syscall\n.section .tdata,\"awT\",@progbits\ntls_first:\n .long 11\ntls_second:\n .long 22\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);

    let relocs = readelf(&exe, &["-rW"]);
    assert!(
        !relocs.contains("R_X86_64_TLSDESC"),
        "executable should not keep TLSDESC relocations:\n{relocs}"
    );
    let disasm = objdump(&exe, &["-dr"]);
    assert!(
        disasm.contains("mov    $0xfffffffffffffff8,%rax") && disasm.contains("xchg   %ax,%ax"),
        "TLSDESC sequence was not relaxed like GNU ld:\n{disasm}"
    );
}

/// A dynamic executable peony produces runs identically to one from GNU `ld`.
#[test]
fn dynamic_matches_ld() {
    let dir = workdir("dyn_diff");
    compile_shared(&dir, "v", "int value = 42;\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movq value@GOTPCREL(%rip), %rax\n movl (%rax), %edi\n movl $60,%eax\n syscall\n",
    );
    let dir_str = dir.to_str().unwrap();

    let peony_exe = dir.join("peony.out");
    link(
        &peony_exe,
        std::slice::from_ref(&m),
        &["-L", dir_str, "-l", "v"],
    );
    let peony_rc = run_env(&peony_exe, &[("LD_LIBRARY_PATH", dir_str)]);

    // Reference with GNU ld.
    let ld_exe = dir.join("ld.out");
    let ok = std::process::Command::new("ld")
        .args(["-o"])
        .arg(&ld_exe)
        .args(["-dynamic-linker", "/lib64/ld-linux-x86-64.so.2"])
        .arg(&m)
        .args(["-L", dir_str, "-lv"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    assert_eq!(peony_rc, 42);
    if ok {
        let ld_rc = run_env(&ld_exe, &[("LD_LIBRARY_PATH", dir_str)]);
        assert_eq!(peony_rc, ld_rc, "peony and ld disagree on dynamic link");
    }
}
