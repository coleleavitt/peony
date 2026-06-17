//! Feature tests: --gc-sections, custom entry, alignment, many objects,
//! differential vs GNU ld, and ELF structural validity.

mod common;
use common::*;

const GC_SRC: &str = "
    .section .text.used,\"ax\",@progbits
    .globl used
used:
    movl $42, %eax
    ret
    .section .text.unused,\"ax\",@progbits
    .globl unused
unused:
    movl $99, %eax
    ret
    .text
    .globl _start
_start:
    call used
    movl %eax, %edi
    movl $60, %eax
    syscall
";

/// --gc-sections removes a function unreachable from the entry point.
#[test]
fn gc_drops_unused() {
    let dir = workdir("gc_drop");
    let o = assemble(&dir, "gc", GC_SRC);
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--gc-sections"]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-s"]);
    assert!(!syms.contains("unused"), "gc should have dropped `unused`");
    assert!(syms.contains("used"), "`used` must be kept");
}

/// Without --gc-sections, the unused function is retained.
#[test]
fn no_gc_keeps_unused() {
    let dir = workdir("gc_keep");
    let o = assemble(&dir, "gc", GC_SRC);
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
    assert!(readelf(&exe, &["-s"]).contains("unused"));
}

/// Transitively-referenced sections survive GC.
#[test]
fn gc_keeps_referenced_chain() {
    let dir = workdir("gc_chain");
    let o = assemble(
        &dir,
        "chain",
        "
        .section .text.b,\"ax\",@progbits
        .globl bee
    bee:
        movl $42, %eax
        ret
        .section .text.a,\"ax\",@progbits
        .globl ay
    ay:
        call bee
        ret
        .text
        .globl _start
    _start:
        call ay
        movl %eax, %edi
        movl $60, %eax
        syscall
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--gc-sections"]);
    assert_eq!(run(&exe), 42); // bee reached transitively via ay
}

/// Custom entry symbol via `--entry`.
#[test]
fn custom_entry() {
    let dir = workdir("entry");
    let o = assemble(
        &dir,
        "entry",
        ".text\n.globl mymain\nmymain:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--entry", "mymain"]);
    assert_eq!(run(&exe), 42);
    assert!(readelf(&exe, &["-h"]).contains("Entry point"));
}

/// `-T` linker scripts provide ENTRY, base address, output section naming, and
/// output-section order for the subset Peony's fixed segment model supports.
#[test]
fn linker_script_sections_entry_and_base() {
    let dir = workdir("script_sections");
    let script = dir.join("layout.ld");
    std::fs::write(
        &script,
        "ENTRY(custom_start)\nSECTIONS {\n  . = 0x600000 + SIZEOF_HEADERS;\n  .fast : { *(.text.hot*) }\n  .text : { *(.text*) }\n  .rodata : { *(.rodata*) }\n  .data : { *(.data*) }\n  .bss : { *(.bss*) }\n}\n",
    )
    .unwrap();
    let o = assemble(
        &dir,
        "script_sections",
        ".section .text.cold,\"ax\",@progbits\n.globl cold\ncold:\n movl $99,%edi\n movl $60,%eax\n syscall\n.section .text.hot,\"ax\",@progbits\n.globl custom_start\ncustom_start:\n movl $42,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["-T", script.to_str().unwrap()]);

    assert_eq!(run(&exe), 42);
    let hdr = readelf(&exe, &["-h"]);
    assert!(
        hdr.contains("0x601000"),
        "script base should move entry into the 0x600000 image:\n{hdr}"
    );
    let sections = readelf(&exe, &["-SW"]);
    let fast = sections.find(" .fast ").expect("script-created .fast");
    let text = sections.find(" .text ").expect("script-created .text");
    assert!(
        fast < text,
        ".fast should follow script order before .text:\n{sections}"
    );
}

/// Compiler LTO reaches Peony through GNU's `-plugin` linker API. Peony hands
/// that path to the system GNU linker so the real plugin can produce native
/// replacement objects instead of treating slim LTO objects as ordinary ELF.
#[test]
fn gcc_lto_plugin_link_handoff() {
    let dir = workdir("lto_plugin");
    let bindir = dir.join("ldbin");
    std::fs::create_dir_all(&bindir).unwrap();
    std::fs::copy(PEONY, bindir.join("ld")).unwrap();
    let src = dir.join("main.c");
    std::fs::write(
        &src,
        "static int answer(void) { return 42; }\nint main(void) { return answer(); }\n",
    )
    .unwrap();
    let exe = dir.join("a.out");
    let status = std::process::Command::new("cc")
        .arg(format!("-B{}/", bindir.display()))
        .args(["-flto", "-fuse-linker-plugin", "-fno-pie", "-no-pie", "-o"])
        .arg(&exe)
        .arg(&src)
        .status()
        .expect("run cc with LTO");
    if !status.success() {
        eprintln!("host compiler/linker does not support this LTO plugin test");
        return;
    }
    assert_eq!(run(&exe), 42);
}

/// An over-aligned section gets a correctly aligned address.
#[test]
fn section_alignment() {
    let dir = workdir("align");
    let o = assemble(
        &dir,
        "align",
        "
        .text
        .globl _start
    _start:
        movl aligned, %edi
        movl $60, %eax
        syscall
        .section .data
        .p2align 12
        .globl aligned
    aligned:
        .long 42
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);
    // `aligned` must sit on a 4 KiB boundary.
    let syms = readelf(&exe, &["-s"]);
    let line = syms
        .lines()
        .find(|l| l.ends_with(" aligned"))
        .expect("aligned symbol");
    let val = u64::from_str_radix(line.split_whitespace().nth(1).unwrap(), 16).unwrap();
    assert_eq!(val % 0x1000, 0, "symbol not 4 KiB aligned: {val:#x}");
}

/// Many objects, each contributing to the result.
#[test]
fn many_objects() {
    let dir = workdir("many");
    let mut objs = Vec::new();
    // six adders of value 7 each => 42
    for i in 0..6 {
        objs.push(assemble(
            &dir,
            &format!("add{i}"),
            &format!(".text\n.globl add{i}\nadd{i}:\n addl $7, %edi\n ret\n"),
        ));
    }
    let mut main = String::from(".text\n.globl _start\n_start:\n xorl %edi, %edi\n");
    for i in 0..6 {
        main.push_str(&format!(" call add{i}@PLT\n"));
    }
    main.push_str(" movl $60, %eax\n syscall\n");
    objs.insert(0, assemble(&dir, "main", &main));
    let exe = dir.join("a.out");
    link(&exe, &objs, &[]);
    assert_eq!(run(&exe), 42);
}

/// Differential: peony and GNU ld produce binaries with the same exit code.
#[test]
fn differential_vs_ld() {
    let dir = workdir("diff");
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call val@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let lib = assemble(
        &dir,
        "lib",
        ".text\n.globl val\nval:\n movl $42,%eax\n ret\n",
    );

    let peony_exe = dir.join("peony.out");
    link(&peony_exe, &[main.clone(), lib.clone()], &[]);
    let peony_rc = run(&peony_exe);

    let ld_exe = dir.join("ld.out");
    assert!(ld_link(&ld_exe, &[main, lib]), "ld link failed");
    let ld_rc = run(&ld_exe);

    assert_eq!(peony_rc, 42);
    assert_eq!(peony_rc, ld_rc, "peony and ld disagree");
}

/// COMDAT group deduplication: the same group in two objects is kept once
/// (otherwise the shared symbol would be a duplicate-definition error).
#[test]
fn comdat_group_dedup() {
    let dir = workdir("comdat");
    let grp = ".section .text.inl,\"axG\",@progbits,inl,comdat\n.weak inl\n.globl inl\ninl:\n movl $42,%eax\n ret\n";
    let a = assemble(
        &dir,
        "a",
        &format!("{grp}.text\n.globl afn\nafn:\n call inl\n ret\n"),
    );
    let b = assemble(
        &dir,
        "b",
        &format!(
            "{grp}.text\n.globl _start\n_start:\n call inl\n movl %eax,%edi\n movl $60,%eax\n syscall\n"
        ),
    );
    let exe = dir.join("a.out");
    link(&exe, &[a, b], &[]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-s"]);
    let count = syms.lines().filter(|l| l.ends_with(" inl")).count();
    assert_eq!(count, 1, "COMDAT should keep exactly one `inl`");
}

/// `--build-id` emits a `.note.gnu.build-id` (+ PT_NOTE) and is deterministic.
#[test]
fn build_id_note() {
    let dir = workdir("buildid");
    let src = ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n";
    let o = assemble(&dir, "b", src);

    let exe1 = dir.join("a.out");
    link(&exe1, std::slice::from_ref(&o), &["--build-id"]);
    assert_eq!(run(&exe1), 42);

    let notes = readelf(&exe1, &["-n"]);
    assert!(
        notes.contains("Build ID:"),
        "build-id note missing: {notes}"
    );
    assert!(readelf(&exe1, &["-l"]).contains("NOTE"), "PT_NOTE missing");

    // Deterministic: same inputs → identical build-id.
    let exe2 = dir.join("b.out");
    link(&exe2, std::slice::from_ref(&o), &["--build-id"]);
    let id = |s: &str| {
        s.lines()
            .find(|l| l.contains("Build ID:"))
            .map(|l| l.trim().to_string())
    };
    assert_eq!(
        id(&notes),
        id(&readelf(&exe2, &["-n"])),
        "build-id not deterministic"
    );
}

/// `-L <dir> -l <name>` resolves `lib<name>.a` on the search path.
#[test]
fn library_search_flags() {
    let dir = workdir("lflag");
    let foo = assemble(
        &dir,
        "foo",
        ".text\n.globl foo\nfoo:\n movl $42,%eax\n ret\n",
    );
    archive(&dir, "libfoo", &[foo]); // → libfoo.a in `dir`
    let main = assemble(
        &dir,
        "main",
        ".text\n.globl _start\n_start:\n call foo@PLT\n movl %eax,%edi\n movl $60,%eax\n syscall\n",
    );
    let exe = dir.join("a.out");
    let dir_str = dir.to_str().unwrap();
    link(&exe, &[main], &["-L", dir_str, "-l", "foo"]);
    assert_eq!(run(&exe), 42);
}

/// `-s` / `--strip-all` omits the symbol table while keeping a runnable binary.
#[test]
fn strip_all() {
    let dir = workdir("strip");
    let o = assemble(
        &dir,
        "s",
        "
        .section .debug_info,\"\",@progbits
        .quad _start
        .text
        .globl _start
    _start:
        movl $60,%eax
        movl $42,%edi
        syscall
        ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["-s"]);
    assert_eq!(run(&exe), 42);
    let syms = readelf(&exe, &["-s"]);
    assert!(
        !syms.contains(" _start"),
        "symtab should be stripped: {syms}"
    );
    let sections = readelf(&exe, &["-S"]);
    assert!(
        !sections.contains(".debug_info"),
        "debug sections should be stripped by -s: {sections}"
    );
}

/// `-S` / `--strip-debug` drops DWARF but keeps the ordinary symbol table.
#[test]
fn strip_debug_keeps_symtab() {
    let dir = workdir("strip_debug");
    let o = assemble(
        &dir,
        "sdbg",
        "
        .section .debug_info,\"\",@progbits
        .quad _start
        .text
        .globl _start
    _start:
        movl $60,%eax
        movl $42,%edi
        syscall
        ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["-S"]);
    assert_eq!(run(&exe), 42);

    let syms = readelf(&exe, &["-s"]);
    assert!(syms.contains(" _start"), "symtab should be kept: {syms}");
    let sections = readelf(&exe, &["-S"]);
    assert!(
        !sections.contains(".debug_info"),
        "debug sections should be stripped by -S: {sections}"
    );
}

/// Non-alloc DWARF sections are preserved and still get normal relocations.
#[test]
fn debug_sections_are_preserved() {
    let dir = workdir("debug_sections");
    let o = assemble(
        &dir,
        "dbg",
        "
        .section .debug_info,\"\",@progbits
        .quad _start
        .section .debug_str,\"MS\",@progbits,1
        .asciz \"peony-debug\"
        .text
        .globl _start
    _start:
        movl $60,%eax
        movl $42,%edi
        syscall
    ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);

    let sections = readelf(&exe, &["-S"]);
    assert!(
        sections.contains(".debug_info"),
        "missing .debug_info:\n{sections}"
    );
    assert!(
        sections.contains(".debug_str"),
        "missing .debug_str:\n{sections}"
    );

    let debug_info = readelf(&exe, &["-x", ".debug_info"]);
    assert!(
        !debug_info.contains("00000000 00000000"),
        ".debug_info relocation was not applied:\n{debug_info}"
    );
}

/// ELF `SHF_COMPRESSED` debug inputs are decompressed before relocation/output.
#[test]
fn compressed_debug_sections_are_decompressed_and_relocated() {
    let dir = workdir("compressed_debug");
    let o = assemble_with_args(
        &dir,
        "cdbg",
        "
        .section .debug_info,\"\",@progbits
        .byte 0x11
        .quad _start
        .byte 0x22
        .rept 256
        .byte 0x33
        .endr
        .text
        .globl _start
    _start:
        movl $60,%eax
        movl $42,%edi
        syscall
        ",
        &["--compress-debug-sections=zlib-gabi"],
    );
    let input_sections = readelf(&o, &["-S", "-W"]);
    assert!(
        input_sections
            .lines()
            .any(|line| line.contains(".debug_info") && line.split_whitespace().any(|f| f == "C")),
        "assembler did not produce SHF_COMPRESSED .debug_info:\n{input_sections}"
    );

    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);

    let bytes = section_bytes(&exe, ".debug_info");
    assert_eq!(bytes.first(), Some(&0x11));
    assert_eq!(bytes.get(9), Some(&0x22));
    assert!(
        bytes[1..9].iter().any(|&b| b != 0),
        ".debug_info relocation was not applied"
    );
}

/// Legacy GNU `.zdebug_*` inputs are decompressed and renamed to `.debug_*`.
#[test]
fn gnu_zdebug_sections_are_decompressed_and_renamed() {
    let dir = workdir("zdebug");
    let o = assemble_with_args(
        &dir,
        "zdbg",
        "
        .section .debug_info,\"\",@progbits
        .byte 0x44
        .quad _start
        .byte 0x55
        .rept 256
        .byte 0x66
        .endr
        .text
        .globl _start
    _start:
        movl $60,%eax
        movl $42,%edi
        syscall
        ",
        &["--compress-debug-sections=zlib-gnu"],
    );
    let input_sections = readelf(&o, &["-S", "-W"]);
    assert!(
        input_sections.contains(".zdebug_info"),
        "assembler did not produce GNU .zdebug_info:\n{input_sections}"
    );

    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);

    let output_sections = readelf(&exe, &["-S", "-W"]);
    assert!(
        output_sections.contains(".debug_info"),
        "missing normalized .debug_info:\n{output_sections}"
    );
    assert!(
        !output_sections.contains(".zdebug_info"),
        "decompressed output must not keep .zdebug_info name:\n{output_sections}"
    );
    let bytes = section_bytes(&exe, ".debug_info");
    assert_eq!(bytes.first(), Some(&0x44));
    assert_eq!(bytes.get(9), Some(&0x55));
    assert!(
        bytes[1..9].iter().any(|&b| b != 0),
        ".debug_info relocation was not applied"
    );
}

/// GNU ld keeps `.gnu_debugaltlink` even when `-S` strips local DWARF.
#[test]
fn gnu_debugaltlink_is_preserved_with_strip_debug() {
    let dir = workdir("gnu_debugaltlink");
    let o = assemble(
        &dir,
        "alt",
        "
        .section .gnu_debugaltlink,\"\",@progbits
        .asciz \"alt.dwo\"
        .quad 0x1122334455667788
        .section .debug_info,\"\",@progbits
        .quad _start
        .text
        .globl _start
    _start:
        movl $60,%eax
        movl $42,%edi
        syscall
        ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["-S"]);
    assert_eq!(run(&exe), 42);

    let sections = readelf(&exe, &["-S", "-W"]);
    assert!(
        sections.contains(".gnu_debugaltlink"),
        "missing .gnu_debugaltlink:\n{sections}"
    );
    assert!(
        !sections.contains(".debug_info"),
        "ordinary DWARF should still be stripped by -S:\n{sections}"
    );
    let bytes = section_bytes(&exe, ".gnu_debugaltlink");
    assert!(
        bytes.starts_with(b"alt.dwo\0"),
        ".gnu_debugaltlink payload was not copied: {bytes:02x?}"
    );
}

/// `.note.gnu.property` is loadable metadata and must be exposed via
/// `PT_GNU_PROPERTY`, not dropped with ordinary notes.
#[test]
fn gnu_property_note_is_preserved_with_segment() {
    let dir = workdir("gnu_property");
    let o = assemble(
        &dir,
        "prop",
        "
        .section .note.gnu.property,\"a\",@note
        .p2align 3
        .long 4
        .long 16
        .long 5
        .asciz \"GNU\"
        .p2align 3
        .long 0xc0000002
        .long 4
        .long 1
        .long 0

        .text
        .globl _start
    _start:
        movl $60,%eax
        movl $42,%edi
        syscall
        ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);
    assert_eq!(run(&exe), 42);

    let sections = readelf(&exe, &["-S", "-W"]);
    assert!(
        sections
            .lines()
            .any(|line| line.contains(".note.gnu.property") && line.contains("NOTE")),
        "missing SHT_NOTE .note.gnu.property:\n{sections}"
    );
    let phdrs = readelf(&exe, &["-lW"]);
    assert!(
        phdrs.contains("GNU_PROPERTY"),
        "missing PT_GNU_PROPERTY:\n{phdrs}"
    );
}

/// `--emit-relocs` preserves output relocation records linked to `.symtab`.
#[test]
fn emit_relocs_outputs_rela_sections() {
    let dir = workdir("emit_relocs");
    let o = assemble(
        &dir,
        "rel",
        "
        .text
        .globl _start
    _start:
        movl value(%rip),%edi
        movl $60,%eax
        syscall
        .data
        .globl value
    value:
        .long 42
        ",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--emit-relocs"]);
    assert_eq!(run(&exe), 42);

    let relocs = readelf(&exe, &["-rW"]);
    assert!(
        relocs.contains(".rela.text")
            && relocs.contains("R_X86_64_PC32")
            && relocs.contains("value"),
        "missing emitted relocation:\n{relocs}"
    );
}

fn section_bytes(path: &std::path::Path, name: &str) -> Vec<u8> {
    let sections = readelf(path, &["-S", "-W"]);
    let line = sections
        .lines()
        .find(|line| line.split_whitespace().any(|field| field == name))
        .unwrap_or_else(|| panic!("missing section {name}:\n{sections}"));
    let fields: Vec<&str> = line.split_whitespace().collect();
    let name_idx = fields
        .iter()
        .position(|field| *field == name)
        .unwrap_or_else(|| panic!("malformed section line: {line}"));
    let offset = usize::from_str_radix(fields[name_idx + 3], 16).unwrap();
    let size = usize::from_str_radix(fields[name_idx + 4], 16).unwrap();
    let file = std::fs::read(path).unwrap();
    file[offset..offset + size].to_vec()
}

/// `--pie` produces an `ET_DYN` position-independent executable that the kernel
/// loads at a randomized base and runs correctly (PC-relative code).
#[test]
fn pie_executable() {
    let dir = workdir("pie");
    let o = assemble(
        &dir,
        "p",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--pie"]);
    assert!(readelf(&exe, &["-h"]).contains("DYN"), "should be ET_DYN");
    assert_eq!(run(&exe), 42);
}

/// A PIE with RIP-relative data access runs correctly under load bias.
#[test]
fn pie_pc_relative_data() {
    let dir = workdir("pie_data");
    let o = assemble(
        &dir,
        "pd",
        ".text\n.globl _start\n_start:\n movl val(%rip), %edi\n movl $60,%eax\n syscall\n.section .rodata\nval:\n .long 42\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &["--pie"]);
    assert_eq!(run(&exe), 42);
}

/// The output is a well-formed ET_EXEC x86-64 ELF with loadable segments,
/// and is marked executable on disk.
#[test]
fn valid_elf_structure() {
    let dir = workdir("valid");
    let o = assemble(
        &dir,
        "v",
        ".text\n.globl _start\n_start:\n movl $60,%eax\n movl $42,%edi\n syscall\n",
    );
    let exe = dir.join("a.out");
    link(&exe, &[o], &[]);

    let h = readelf(&exe, &["-h"]);
    assert!(h.contains("EXEC"), "should be ET_EXEC");
    assert!(h.contains("X86-64"), "should be x86-64");

    let l = readelf(&exe, &["-l"]);
    assert!(l.contains("LOAD"), "should have PT_LOAD segments");
    assert!(l.contains("R E"), "should have an executable segment");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "output must be executable");
    }
    assert_eq!(run(&exe), 42);
}
