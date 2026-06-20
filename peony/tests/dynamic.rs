//! Dynamic-linking tests: link against a real `cc -shared` library, let the
//! system `ld.so` resolve the imports at load, and check the program runs.

mod common;
use std::process::Command;

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

#[test]
fn as_needed_omits_unused_shared_library() {
    let dir = workdir("as_needed_unused");
    compile_shared(&dir, "unused", "int unused(void) { return 7; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movl $42,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "--as-needed", "-l", "unused"]);
    let d = readelf(&exe, &["-d"]);
    assert!(
        !d.contains("libunused.so"),
        "--as-needed should suppress unused DT_NEEDED:\n{d}"
    );
    assert_eq!(run(&exe), 42);
}

#[test]
fn no_as_needed_retains_unused_shared_library() {
    let dir = workdir("no_as_needed_unused");
    compile_shared(&dir, "unused", "int unused(void) { return 7; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movl $42,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(
        &exe,
        &[m],
        &["-L", dir_str, "--no-as-needed", "-l", "unused"],
    );
    let d = readelf(&exe, &["-d"]);
    assert!(
        d.contains("libunused.so"),
        "--no-as-needed should retain unused DT_NEEDED:\n{d}"
    );
}

#[test]
fn rpath_emits_runpath_and_is_used_by_loader() {
    let dir = workdir("rpath_runpath");
    compile_shared(&dir, "rp", "int get_answer(void) { return 42; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n call get_answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-rpath", dir_str, "-l", "rp"]);
    let d = readelf(&exe, &["-d"]);
    assert!(
        d.contains("RUNPATH") && d.contains(dir_str),
        "missing DT_RUNPATH:\n{d}"
    );
    assert_eq!(run(&exe), 42);
}

#[test]
fn dynamic_linker_sets_program_interpreter() {
    let dir = workdir("dynamic_linker");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n movl $42,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(
        &exe,
        &[m],
        &["-dynamic-linker", "/lib64/ld-linux-x86-64.so.2"],
    );
    let phdrs = readelf(&exe, &["-lW"]);
    assert!(
        phdrs.contains("/lib64/ld-linux-x86-64.so.2"),
        "missing custom PT_INTERP:\n{phdrs}"
    );
    assert_eq!(run(&exe), 42);
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

#[test]
fn hash_style_sysv_omits_gnu_hash() {
    let dir = workdir("hash_sysv");
    compile_shared(&dir, "hashsysv", "int get_answer(void){ return 42; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n call get_answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(
        &exe,
        &[m],
        &["-L", dir_str, "--hash-style=sysv", "-l", "hashsysv"],
    );
    let d = readelf(&exe, &["-d"]);
    assert!(d.contains("(HASH)"), "missing DT_HASH:\n{d}");
    assert!(!d.contains("GNU_HASH"), "unexpected DT_GNU_HASH:\n{d}");
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

#[test]
fn hash_style_gnu_omits_sysv_hash_for_import_only_executable() {
    let dir = workdir("hash_gnu");
    compile_shared(&dir, "hashgnu", "int get_answer(void){ return 42; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n call get_answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(
        &exe,
        &[m],
        &["-L", dir_str, "--hash-style=gnu", "-l", "hashgnu"],
    );
    let d = readelf(&exe, &["-d"]);
    assert!(d.contains("GNU_HASH"), "missing DT_GNU_HASH:\n{d}");
    assert!(!d.contains("(HASH)"), "unexpected DT_HASH:\n{d}");
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

#[test]
fn no_undefined_rejects_shared_object_undefineds() {
    let dir = workdir("shared_no_undefined");
    let o = assemble(
        &dir,
        "missing",
        ".text\n.globl call_missing\ncall_missing:\n call missing@PLT\n ret\n",
    );
    let so = dir.join("libmissing.so");
    let out = link_raw(&so, &[o], &["-shared", "--no-undefined"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("missing"), "stderr was {stderr}");
}

#[test]
fn dynamic_executable_emits_non_empty_relro_segment() {
    let dir = workdir("dyn_relro");
    compile_shared(&dir, "relro", "int get_answer(void){ return 42; }\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n call get_answer@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-l", "relro"]);

    let phdrs = readelf(&exe, &["-lW"]);
    let relro = phdrs
        .lines()
        .find(|line| line.contains("GNU_RELRO"))
        .unwrap_or_else(|| panic!("missing GNU_RELRO program header:\n{phdrs}"));
    let fields = relro.split_whitespace().collect::<Vec<_>>();
    assert_ne!(
        fields.get(4),
        Some(&"0x000000"),
        "zero RELRO filesz:\n{phdrs}"
    );
    assert_ne!(
        fields.get(5),
        Some(&"0x000000"),
        "zero RELRO memsz:\n{phdrs}"
    );
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

#[test]
fn executable_imported_initial_exec_tls_gets_tpoff_relocation() {
    let dir = workdir("dyn_tls_ie");
    compile_shared(&dir, "tlsshared", "__thread int tls_answer = 42;\n");

    let bindir = dir.join("ldbin");
    std::fs::create_dir_all(&bindir).unwrap();
    std::fs::copy(PEONY, bindir.join("ld")).unwrap();
    let src = dir.join("main.c");
    std::fs::write(
        &src,
        "extern __thread int tls_answer __attribute__((tls_model(\"initial-exec\")));\n\
         int main(void) { return tls_answer; }\n",
    )
    .unwrap();
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    let status = Command::new("cc")
        .arg(format!("-B{}/", bindir.display()))
        .args(["-fpie", "-pie", "-o"])
        .arg(&exe)
        .arg(&src)
        .args(["-L", dir_str, "-l", "tlsshared"])
        .status()
        .expect("run cc through peony");
    assert!(status.success(), "cc/peony link failed");

    let relocs = readelf(&exe, &["-rW"]);
    assert!(
        relocs.contains("R_X86_64_TPOFF64") && relocs.contains("tls_answer"),
        "missing imported TLS TPOFF64 relocation:\n{relocs}"
    );
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

#[test]
fn executable_imported_general_dynamic_tls_relaxes_to_initial_exec() {
    let dir = workdir("dyn_tls_gd_ie");
    compile_shared(&dir, "tlsgd", "__thread int tls_answer = 42;\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n\
         .byte 0x66\n\
         leaq tls_answer@tlsgd(%rip), %rdi\n\
         .word 0x6666\n\
         rex64\n\
         call __tls_get_addr@plt\n\
         movl (%rax), %edi\n\
         movl $60, %eax\n\
         syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-l", "tlsgd"]);

    let relocs = readelf(&exe, &["-rW"]);
    assert!(
        relocs.contains("R_X86_64_TPOFF64") && relocs.contains("tls_answer"),
        "missing GD→IE imported TLS relocation:\n{relocs}"
    );
    assert_eq!(run_env(&exe, &[("LD_LIBRARY_PATH", dir_str)]), 42);
}

#[test]
fn executable_imported_tlsdesc_relaxes_to_initial_exec() {
    let dir = workdir("dyn_tlsdesc_ie");
    compile_shared(&dir, "tlsdesc", "__thread int tls_answer = 42;\n");
    let m = assemble(
        &dir,
        "m",
        ".text\n.globl _start\n_start:\n\
         leaq tls_answer@tlsdesc(%rip), %rax\n\
         call *tls_answer@tlscall(%rax)\n\
         movl %fs:(%rax), %edi\n\
         movl $60, %eax\n\
         syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[m], &["-L", dir_str, "-l", "tlsdesc"]);

    let relocs = readelf(&exe, &["-rW"]);
    assert!(
        relocs.contains("R_X86_64_TPOFF64") && relocs.contains("tls_answer"),
        "missing TLSDESC→IE imported TLS relocation:\n{relocs}"
    );
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
