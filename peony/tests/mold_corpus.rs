//! Manifest of mold's production test suite, converted to Rust.
//!
//! Every one of mold's tests is represented here as a `#[test]`. Tests whose
//! *concept* is within peony's static x86-64 domain are exercised by the
//! `link_and_run` / `relocations` / `symbols` / `features` suites; the mold
//! originals are `#[ignore]`d here with the subsystem they additionally need
//! (mostly libc + dynamic linking). Run `cargo test -- --ignored --list` to
//! see the full corpus. This file documents coverage honestly — ignored tests
//! are NOT claimed to pass.

#[test]
#[ignore = "needs dynamic linking"]
fn mold_abs_error() {
    // mold/test/abs-error.sh
}

#[test]
#[ignore = "concept covered by tests/symbols.rs::absolute_symbol; mold original needs dynamic linking"]
fn mold_absolute_symbols() {
    // mold/test/absolute-symbols.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_allow_multiple_definition() {
    // mold/test/allow-multiple-definition.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_ar_alignment() {
    // mold/test/ar-alignment.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_aarch64_long_thunk() {
    // mold/test/arch-aarch64-long-thunk.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_aarch64_pack_dyn_relocs() {
    // mold/test/arch-aarch64-pack-dyn-relocs.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_aarch64_range_extension_thunk_disassembly() {
    // mold/test/arch-aarch64-range-extension-thunk-disassembly.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_aarch64_variant_pcs() {
    // mold/test/arch-aarch64-variant-pcs.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_abs_error() {
    // mold/test/arch-arm-abs-error.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_exidx_sentinel() {
    // mold/test/arch-arm-exidx-sentinel.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_range_extension_thunk_disassembly() {
    // mold/test/arch-arm-range-extension-thunk-disassembly.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_range_extension_thunk() {
    // mold/test/arch-arm-range-extension-thunk.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_target1() {
    // mold/test/arch-arm-target1.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_thm_jump19() {
    // mold/test/arch-arm-thm-jump19.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_thm_jump8() {
    // mold/test/arch-arm-thm-jump8.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_thumb_interwork() {
    // mold/test/arch-arm-thumb-interwork.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_arm_tlsdesc() {
    // mold/test/arch-arm-tlsdesc.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_armeb_be32() {
    // mold/test/arch-armeb-be32.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_i686_tls_module_base() {
    // mold/test/arch-i686-tls-module-base.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_i686_tlsdesc() {
    // mold/test/arch-i686-tlsdesc.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_emit_relocs_relax() {
    // mold/test/arch-loongarch64-emit-relocs-relax.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_emit_relocs_tlsdesc_relax() {
    // mold/test/arch-loongarch64-emit-relocs-tlsdesc-relax.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_mcmodel_extreme() {
    // mold/test/arch-loongarch64-mcmodel-extreme.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_relax_call36() {
    // mold/test/arch-loongarch64-relax-call36.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_relax_got_load() {
    // mold/test/arch-loongarch64-relax-got-load.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_relax_got_load2() {
    // mold/test/arch-loongarch64-relax-got-load2.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_relax_pcala_addi() {
    // mold/test/arch-loongarch64-relax-pcala-addi.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_loongarch64_relax_tlsdesc() {
    // mold/test/arch-loongarch64-relax-tlsdesc.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_ppc64le_save_restore_gprs() {
    // mold/test/arch-ppc64le-save-restore-gprs.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_attributes() {
    // mold/test/arch-riscv64-attributes.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_attributes2() {
    // mold/test/arch-riscv64-attributes2.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_emit_relocs_relax() {
    // mold/test/arch-riscv64-emit-relocs-relax.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_global_pointer_dso() {
    // mold/test/arch-riscv64-global-pointer-dso.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_global_pointer() {
    // mold/test/arch-riscv64-global-pointer.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_obj_compatible() {
    // mold/test/arch-riscv64-obj-compatible.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_relax_align() {
    // mold/test/arch-riscv64-relax-align.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_relax_got() {
    // mold/test/arch-riscv64-relax-got.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_relax_hi20() {
    // mold/test/arch-riscv64-relax-hi20.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_relax_j() {
    // mold/test/arch-riscv64-relax-j.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_reloc_overflow() {
    // mold/test/arch-riscv64-reloc-overflow.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_symbol_size() {
    // mold/test/arch-riscv64-symbol-size.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_variant_cc() {
    // mold/test/arch-riscv64-variant-cc.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_riscv64_weak_undef() {
    // mold/test/arch-riscv64-weak-undef.sh
}

#[test]
#[ignore = "needs non-x86-64 target"]
fn mold_arch_s390x_got() {
    // mold/test/arch-s390x-got.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_address_equality() {
    // mold/test/arch-x86_64-address-equality.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_apx_gotpcrelx() {
    // mold/test/arch-x86_64-apx-gotpcrelx.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_apx_gottpoff() {
    // mold/test/arch-x86_64-apx-gottpoff.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_apx_gottpoff2() {
    // mold/test/arch-x86_64-apx-gottpoff2.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_apx_tlsdesc() {
    // mold/test/arch-x86_64-apx-tlsdesc.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_empty_arg() {
    // mold/test/arch-x86_64-empty-arg.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_empty_mergeable_section() {
    // mold/test/arch-x86_64-empty-mergeable-section.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_emulation_deduction() {
    // mold/test/arch-x86_64-emulation-deduction.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_exception_mcmodel_large() {
    // mold/test/arch-x86_64-exception-mcmodel-large.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_execstack_if_needed() {
    // mold/test/arch-x86_64-execstack-if-needed.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_function_multiversion() {
    // mold/test/arch-x86_64-function-multiversion.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_gnu_linkonce() {
    // mold/test/arch-x86_64-gnu-linkonce.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_gnu_retain() {
    // mold/test/arch-x86_64-gnu-retain.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_gotpcrelx() {
    // mold/test/arch-x86_64-gotpcrelx.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_ifunc_alias() {
    // mold/test/arch-x86_64-ifunc-alias.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_ifunc_export() {
    // mold/test/arch-x86_64-ifunc-export.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_incompatible_libs_linker_script() {
    // mold/test/arch-x86_64-incompatible-libs-linker-script.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_incompatible_libs_linker_script2() {
    // mold/test/arch-x86_64-incompatible-libs-linker-script2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_incompatible_libs() {
    // mold/test/arch-x86_64-incompatible-libs.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_incompatible_libs2() {
    // mold/test/arch-x86_64-incompatible-libs2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_incompatible_obj() {
    // mold/test/arch-x86_64-incompatible-obj.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_init_array_readonly() {
    // mold/test/arch-x86_64-init-array-readonly.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_init_array() {
    // mold/test/arch-x86_64-init-array.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_isa_level() {
    // mold/test/arch-x86_64-isa-level.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_large_bss() {
    // mold/test/arch-x86_64-large-bss.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_lto_comdat_mixed() {
    // mold/test/arch-x86_64-lto-comdat-mixed.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_mergeable_records() {
    // mold/test/arch-x86_64-mergeable-records.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_mergeable_strings_nonalloc() {
    // mold/test/arch-x86_64-mergeable-strings-nonalloc.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_mergeable_strings() {
    // mold/test/arch-x86_64-mergeable-strings.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_note_property() {
    // mold/test/arch-x86_64-note-property.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_note_property2() {
    // mold/test/arch-x86_64-note-property2.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_note() {
    // mold/test/arch-x86_64-note.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_note2() {
    // mold/test/arch-x86_64-note2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_plt() {
    // mold/test/arch-x86_64-plt.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_preinit_array() {
    // mold/test/arch-x86_64-preinit-array.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_relax() {
    // mold/test/arch-x86_64-relax.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_reloc_overflow() {
    // mold/test/arch-x86_64-reloc-overflow.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_reloc_zero() {
    // mold/test/arch-x86_64-reloc-zero.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_reloc() {
    // mold/test/arch-x86_64-reloc.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_section_alignment() {
    // mold/test/arch-x86_64-section-alignment.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_section_name() {
    // mold/test/arch-x86_64-section-name.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_tbss_only() {
    // mold/test/arch-x86_64-tbss-only.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_tls_gd_mcmodel_large() {
    // mold/test/arch-x86_64-tls-gd-mcmodel-large.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_tls_gd_to_ie() {
    // mold/test/arch-x86_64-tls-gd-to-ie.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_tls_large_tbss() {
    // mold/test/arch-x86_64-tls-large-tbss.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_tls_ld_mcmodel_large() {
    // mold/test/arch-x86_64-tls-ld-mcmodel-large.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_tls_module_base() {
    // mold/test/arch-x86_64-tls-module-base.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_arch_x86_64_tlsdesc() {
    // mold/test/arch-x86_64-tlsdesc.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_unique() {
    // mold/test/arch-x86_64-unique.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_warn_execstack() {
    // mold/test/arch-x86_64-warn-execstack.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_warn_shared_textrel() {
    // mold/test/arch-x86_64-warn-shared-textrel.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_warn_textrel() {
    // mold/test/arch-x86_64-warn-textrel.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_z_dynamic_undefined_weak() {
    // mold/test/arch-x86_64-z-dynamic-undefined-weak.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_z_ibt() {
    // mold/test/arch-x86_64-z-ibt.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_z_ibtplt() {
    // mold/test/arch-x86_64-z-ibtplt.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_z_rewrite_endbr() {
    // mold/test/arch-x86_64-z-rewrite-endbr.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_z_rewrite_endbr2() {
    // mold/test/arch-x86_64-z-rewrite-endbr2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_arch_x86_64_z_rewrite_endbr3() {
    // mold/test/arch-x86_64-z-rewrite-endbr3.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_arch_x86_64_z_shstk() {
    // mold/test/arch-x86_64-z-shstk.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_arch_x86_64_z_text() {
    // mold/test/arch-x86_64-z-text.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_as_needed_dso() {
    // mold/test/as-needed-dso.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_as_needed_dso2() {
    // mold/test/as-needed-dso2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_as_needed_weak() {
    // mold/test/as-needed-weak.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_as_needed() {
    // mold/test/as-needed.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_audit() {
    // mold/test/audit.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_auxiliary() {
    // mold/test/auxiliary.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_bno_symbolic() {
    // mold/test/bno-symbolic.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_bsymbolic_functions() {
    // mold/test/bsymbolic-functions.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_bsymbolic_non_weak_functions() {
    // mold/test/bsymbolic-non-weak-functions.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_bsymbolic_non_weak() {
    // mold/test/bsymbolic-non-weak.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_bsymbolic() {
    // mold/test/bsymbolic.sh
}

#[test]
#[ignore = "concept covered by tests/features.rs::build_id_note; mold original needs libc / C runtime"]
fn mold_build_id() {
    // mold/test/build-id.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_canonical_plt() {
    // mold/test/canonical-plt.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_cmdline() {
    // mold/test/cmdline.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_color_diagnostics() {
    // mold/test/color-diagnostics.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_comment() {
    // mold/test/comment.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_common_archive() {
    // mold/test/common-archive.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_common_ref() {
    // mold/test/common-ref.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_common_symbols() {
    // mold/test/common-symbols.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_compress_debug_sections_zlib_level() {
    // mold/test/compress-debug-sections-zlib-level.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_compress_debug_sections_zstd_level() {
    // mold/test/compress-debug-sections-zstd-level.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_compress_debug_sections_zstd() {
    // mold/test/compress-debug-sections-zstd.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_compress_debug_sections() {
    // mold/test/compress-debug-sections.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_compressed_debug_info() {
    // mold/test/compressed-debug-info.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_copyrel_alignment() {
    // mold/test/copyrel-alignment.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_copyrel_norelro() {
    // mold/test/copyrel-norelro.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_copyrel_protected() {
    // mold/test/copyrel-protected.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_copyrel_relro() {
    // mold/test/copyrel-relro.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_copyrel_relro2() {
    // mold/test/copyrel-relro2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_copyrel() {
    // mold/test/copyrel.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_crel() {
    // mold/test/crel.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_ctors_in_init_array() {
    // mold/test/ctors-in-init-array.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_dead_debug_sections() {
    // mold/test/dead-debug-sections.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_debug_macro_section() {
    // mold/test/debug-macro-section.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_default_symver_version_script() {
    // mold/test/default-symver-version-script.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_default_symver() {
    // mold/test/default-symver.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_defsym_lto() {
    // mold/test/defsym-lto.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_defsym_missing_symbol() {
    // mold/test/defsym-missing-symbol.sh
}

#[test]
#[ignore = "concept covered by tests/symbols.rs::defsym; mold original needs dynamic linking"]
fn mold_defsym() {
    // mold/test/defsym.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_defsym2() {
    // mold/test/defsym2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_demangle_cpp() {
    // mold/test/demangle-cpp.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_demangle_rust() {
    // mold/test/demangle-rust.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_demangle() {
    // mold/test/demangle.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_depaudit() {
    // mold/test/depaudit.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_depaudit2() {
    // mold/test/depaudit2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_dependency_file_lto() {
    // mold/test/dependency-file-lto.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_dependency_file_response_file() {
    // mold/test/dependency-file-response-file.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_dependency_file() {
    // mold/test/dependency-file.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_disable_new_dtags() {
    // mold/test/disable-new-dtags.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_discard_section() {
    // mold/test/discard-section.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_discard() {
    // mold/test/discard.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dso_undef() {
    // mold/test/dso-undef.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dt_init() {
    // mold/test/dt-init.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dt_needed() {
    // mold/test/dt-needed.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_duplicate_error_archive() {
    // mold/test/duplicate-error-archive.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_duplicate_error_gc_sections() {
    // mold/test/duplicate-error-gc-sections.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_duplicate_error_lto() {
    // mold/test/duplicate-error-lto.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_duplicate_error() {
    // mold/test/duplicate-error.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dynamic_dt_debug() {
    // mold/test/dynamic-dt-debug.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_dynamic_linker() {
    // mold/test/dynamic-linker.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dynamic_list_data() {
    // mold/test/dynamic-list-data.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dynamic_list() {
    // mold/test/dynamic-list.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dynamic_list2() {
    // mold/test/dynamic-list2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dynamic_list3() {
    // mold/test/dynamic-list3.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_dynamic_list4() {
    // mold/test/dynamic-list4.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_dynamic() {
    // mold/test/dynamic.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_emit_relocs_cpp() {
    // mold/test/emit-relocs-cpp.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_emit_relocs_dead_sections() {
    // mold/test/emit-relocs-dead-sections.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_emit_relocs() {
    // mold/test/emit-relocs.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_empty_dso() {
    // mold/test/empty-dso.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_empty_file() {
    // mold/test/empty-file.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_empty_input() {
    // mold/test/empty-input.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_empty_version() {
    // mold/test/empty-version.sh
}

#[test]
#[ignore = "concept covered by tests/features.rs::custom_entry; mold original needs libc / C runtime"]
fn mold_entry() {
    // mold/test/entry.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_exception_multiple_ehframe() {
    // mold/test/exception-multiple-ehframe.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_exception() {
    // mold/test/exception.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_exclude_libs() {
    // mold/test/exclude-libs.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_exclude_libs2() {
    // mold/test/exclude-libs2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_exclude_libs3() {
    // mold/test/exclude-libs3.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_execstack() {
    // mold/test/execstack.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_execute_only() {
    // mold/test/execute-only.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_export_dynamic() {
    // mold/test/export-dynamic.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_export_from_exe() {
    // mold/test/export-from-exe.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_fatal_warnings() {
    // mold/test/fatal-warnings.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_filler() {
    // mold/test/filler.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_filter() {
    // mold/test/filter.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_func_addr() {
    // mold/test/func-addr.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_gc_sections_start_stop() {
    // mold/test/gc-sections-start-stop.sh
}

#[test]
#[ignore = "concept covered by tests/features.rs::gc_drops_unused; mold original needs unsupported feature"]
fn mold_gc_sections() {
    // mold/test/gc-sections.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_compress_output() {
    // mold/test/gdb-index-compress-output.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_deterministic() {
    // mold/test/gdb-index-deterministic.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_dwarf2() {
    // mold/test/gdb-index-dwarf2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_dwarf3() {
    // mold/test/gdb-index-dwarf3.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_dwarf4() {
    // mold/test/gdb-index-dwarf4.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_dwarf5() {
    // mold/test/gdb-index-dwarf5.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_dwarf64() {
    // mold/test/gdb-index-dwarf64.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_gdb_index_empty() {
    // mold/test/gdb-index-empty.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_gdb_index_rnglistx() {
    // mold/test/gdb-index-rnglistx.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gdb_index_split_dwarf() {
    // mold/test/gdb-index-split-dwarf.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_glibc_2_22_bug() {
    // mold/test/glibc-2.22-bug.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_global_offset_table() {
    // mold/test/global-offset-table.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gnu_hash() {
    // mold/test/gnu-hash.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gnu_property() {
    // mold/test/gnu-property.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_gnu_retain() {
    // mold/test/gnu-retain.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gnu_unique() {
    // mold/test/gnu-unique.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_gnu_warning() {
    // mold/test/gnu-warning.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_hash_style_sysv() {
    // mold/test/hash-style-sysv.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_hash_style() {
    // mold/test/hash-style.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_hello_dynamic() {
    // mold/test/hello-dynamic.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_hello_static() {
    // mold/test/hello-static.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_help() {
    // mold/test/help.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_hidden_archive() {
    // mold/test/hidden-archive.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_hidden_undef() {
    // mold/test/hidden-undef.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_hidden_weak_undef() {
    // mold/test/hidden-weak-undef.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_hwasan() {
    // mold/test/hwasan.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_icf_alignment_bug() {
    // mold/test/icf-alignment-bug.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_icf_gcc_except_table() {
    // mold/test/icf-gcc-except-table.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_icf_preemption() {
    // mold/test/icf-preemption.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_icf_safe() {
    // mold/test/icf-safe.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_icf_small() {
    // mold/test/icf-small.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_icf() {
    // mold/test/icf.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_address_equality_exported() {
    // mold/test/ifunc-address-equality-exported.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_address_equality() {
    // mold/test/ifunc-address-equality.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_alias() {
    // mold/test/ifunc-alias.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_dlopen() {
    // mold/test/ifunc-dlopen.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_dso() {
    // mold/test/ifunc-dso.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_ifunc_dynamic() {
    // mold/test/ifunc-dynamic.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_export() {
    // mold/test/ifunc-export.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_funcptr() {
    // mold/test/ifunc-funcptr.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_noplt() {
    // mold/test/ifunc-noplt.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_ifunc_static_pie() {
    // mold/test/ifunc-static-pie.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_ifunc_static() {
    // mold/test/ifunc-static.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_image_base() {
    // mold/test/image-base.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_init_array_priorities() {
    // mold/test/init-array-priorities.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_init_in_dso() {
    // mold/test/init-in-dso.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_init() {
    // mold/test/init.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_initfirst() {
    // mold/test/initfirst.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_interpose() {
    // mold/test/interpose.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_invalid_version_script() {
    // mold/test/invalid-version-script.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_issue646() {
    // mold/test/issue646.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_large_alignment_dso() {
    // mold/test/large-alignment-dso.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_large_alignment() {
    // mold/test/large-alignment.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_large_max_page_size_strip() {
    // mold/test/large-max-page-size-strip.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_large_max_page_size() {
    // mold/test/large-max-page-size.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_large_text() {
    // mold/test/large-text.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_library() {
    // mold/test/library.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_link_order() {
    // mold/test/link-order.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_linker_script_defsym() {
    // mold/test/linker-script-defsym.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_linker_script_error() {
    // mold/test/linker-script-error.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_linker_script_group_as_needed() {
    // mold/test/linker-script-group-as-needed.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_linker_script_relocatable() {
    // mold/test/linker-script-relocatable.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_linker_script() {
    // mold/test/linker-script.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_linker_script2() {
    // mold/test/linker-script2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_linker_script3() {
    // mold/test/linker-script3.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_linker_script4() {
    // mold/test/linker-script4.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_linker_script5() {
    // mold/test/linker-script5.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_linker_script6() {
    // mold/test/linker-script6.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_archive() {
    // mold/test/lto-archive.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_archive2() {
    // mold/test/lto-archive2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_archive3() {
    // mold/test/lto-archive3.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_archive4() {
    // mold/test/lto-archive4.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_lto_comdat() {
    // mold/test/lto-comdat.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_lto_dso() {
    // mold/test/lto-dso.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_gcc() {
    // mold/test/lto-gcc.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_lto_llvm() {
    // mold/test/lto-llvm.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_lto_llvm2() {
    // mold/test/lto-llvm2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_mixed_gcc_fat() {
    // mold/test/lto-mixed-gcc-fat.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_mixed_gcc_slim_archive() {
    // mold/test/lto-mixed-gcc-slim-archive.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_lto_no_plugin() {
    // mold/test/lto-no-plugin.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_nostdlib() {
    // mold/test/lto-nostdlib.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_lto_unclaimed() {
    // mold/test/lto-unclaimed.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_lto_version_script() {
    // mold/test/lto-version-script.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_main_in_dso() {
    // mold/test/main-in-dso.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_many_input_sections() {
    // mold/test/many-input-sections.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_many_input_sections2() {
    // mold/test/many-input-sections2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_many_output_sections() {
    // mold/test/many-output-sections.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_mcmodel_large() {
    // mold/test/mcmodel-large.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_mergeable_strings() {
    // mold/test/mergeable-strings.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_missing_but_ok() {
    // mold/test/missing-but-ok.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_missing_error() {
    // mold/test/missing-error.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_mold_wrapper() {
    // mold/test/mold-wrapper.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_mold_wrapper2() {
    // mold/test/mold-wrapper2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_nmagic() {
    // mold/test/nmagic.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined_circular() {
    // mold/test/no-allow-shlib-undefined-circular.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined_hidden() {
    // mold/test/no-allow-shlib-undefined-hidden.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined_lto() {
    // mold/test/no-allow-shlib-undefined-lto.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined_symver() {
    // mold/test/no-allow-shlib-undefined-symver.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined() {
    // mold/test/no-allow-shlib-undefined.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined2() {
    // mold/test/no-allow-shlib-undefined2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined3() {
    // mold/test/no-allow-shlib-undefined3.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_allow_shlib_undefined4() {
    // mold/test/no-allow-shlib-undefined4.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_no_eh_frame_header() {
    // mold/test/no-eh-frame-header.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_interp_on_shared() {
    // mold/test/no-interp-on-shared.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_no_object_file() {
    // mold/test/no-object-file.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_no_quick_exit() {
    // mold/test/no-quick-exit.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_no_undefined_version() {
    // mold/test/no-undefined-version.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_no_warnings() {
    // mold/test/no-warnings.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_nocopyreloc() {
    // mold/test/nocopyreloc.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_noinhibit_exec() {
    // mold/test/noinhibit-exec.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_non_canonical_plt() {
    // mold/test/non-canonical-plt.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_nostdlib() {
    // mold/test/nostdlib.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_oformat_binary() {
    // mold/test/oformat-binary.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_omagic() {
    // mold/test/omagic.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_pack_dyn_relocs_android() {
    // mold/test/pack-dyn-relocs-android.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_package_metadata() {
    // mold/test/package-metadata.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_physical_image_base() {
    // mold/test/physical-image-base.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_pie() {
    // mold/test/pie.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_plt_dso() {
    // mold/test/plt-dso.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_plt_symbols() {
    // mold/test/plt-symbols.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_pltgot() {
    // mold/test/pltgot.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_preinit_array() {
    // mold/test/preinit-array.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_print_dependencies() {
    // mold/test/print-dependencies.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_program_header_align() {
    // mold/test/program-header-align.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_protected_dynsym() {
    // mold/test/protected-dynsym.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_protected() {
    // mold/test/protected.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_push_pop_state() {
    // mold/test/push-pop-state.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_range_extension_thunk() {
    // mold/test/range-extension-thunk.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_range_extension_thunk2() {
    // mold/test/range-extension-thunk2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_range_extension_thunk3() {
    // mold/test/range-extension-thunk3.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_range_extension_thunk4() {
    // mold/test/range-extension-thunk4.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_range_extension_thunk5() {
    // mold/test/range-extension-thunk5.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_relative_vtables() {
    // mold/test/relative-vtables.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_relax_got_load() {
    // mold/test/relax-got-load.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_reloc_rodata() {
    // mold/test/reloc-rodata.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relocatable_archive() {
    // mold/test/relocatable-archive.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_relocatable_c() {
    // mold/test/relocatable-c++.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relocatable_compressed_debug_info() {
    // mold/test/relocatable-compressed-debug-info.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relocatable_debug_info() {
    // mold/test/relocatable-debug-info.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_relocatable_exception() {
    // mold/test/relocatable-exception.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relocatable_many_sections() {
    // mold/test/relocatable-many-sections.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relocatable_merge_sections() {
    // mold/test/relocatable-merge-sections.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_relocatable_mergeable_sections() {
    // mold/test/relocatable-mergeable-sections.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relocatable() {
    // mold/test/relocatable.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relro_alignment() {
    // mold/test/relro-alignment.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_relro() {
    // mold/test/relro.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_repro() {
    // mold/test/repro.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_require_defined() {
    // mold/test/require-defined.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_response_file_quoting() {
    // mold/test/response-file-quoting.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_response_file() {
    // mold/test/response-file.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_response_file2() {
    // mold/test/response-file2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_retain_symbols_file() {
    // mold/test/retain-symbols-file.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_reverse_sections() {
    // mold/test/reverse-sections.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_rodata_name() {
    // mold/test/rodata-name.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_rosegment() {
    // mold/test/rosegment.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_rpath() {
    // mold/test/rpath.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_run_clang() {
    // mold/test/run-clang.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_run() {
    // mold/test/run.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_section_align() {
    // mold/test/section-align.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_section_attributes() {
    // mold/test/section-attributes.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_section_order() {
    // mold/test/section-order.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_section_start() {
    // mold/test/section-start.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_separate_debug_file_sort() {
    // mold/test/separate-debug-file-sort.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_separate_debug_file() {
    // mold/test/separate-debug-file.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_shared_abs_sym() {
    // mold/test/shared-abs-sym.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_shared() {
    // mold/test/shared.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_shuffle_sections_seed() {
    // mold/test/shuffle-sections-seed.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_shuffle_sections() {
    // mold/test/shuffle-sections.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_soname() {
    // mold/test/soname.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_sort_debug_info_compressed() {
    // mold/test/sort-debug-info-compressed.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_sort_debug_info_merged() {
    // mold/test/sort-debug-info-merged.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_sort_debug_info() {
    // mold/test/sort-debug-info.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_spare_program_headers() {
    // mold/test/spare-program-headers.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_start_lib() {
    // mold/test/start-lib.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_start_stop_symbol() {
    // mold/test/start-stop-symbol.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_start_stop() {
    // mold/test/start-stop.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_static_archive() {
    // mold/test/static-archive.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_static_pie() {
    // mold/test/static-pie.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_stdout() {
    // mold/test/stdout.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_strip_debug() {
    // mold/test/strip-debug.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_strip() {
    // mold/test/strip.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_stt_common() {
    // mold/test/stt-common.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_symbol_rank() {
    // mold/test/symbol-rank.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version_as_needed() {
    // mold/test/symbol-version-as-needed.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version_lto() {
    // mold/test/symbol-version-lto.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version_multi() {
    // mold/test/symbol-version-multi.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version() {
    // mold/test/symbol-version.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version2() {
    // mold/test/symbol-version2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version3() {
    // mold/test/symbol-version3.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version4() {
    // mold/test/symbol-version4.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symbol_version5() {
    // mold/test/symbol-version5.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_symtab_dso() {
    // mold/test/symtab-dso.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_symtab_section_symbols() {
    // mold/test/symtab-section-symbols.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_symtab() {
    // mold/test/symtab.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_synthetic_symbols() {
    // mold/test/synthetic-symbols.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_sysroot_linker_script() {
    // mold/test/sysroot-linker-script.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_sysroot() {
    // mold/test/sysroot.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_sysroot2() {
    // mold/test/sysroot2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_tail_call() {
    // mold/test/tail-call.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tbss_only() {
    // mold/test/tbss-only.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_textrel() {
    // mold/test/textrel.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_textrel2() {
    // mold/test/textrel2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_thin_archive() {
    // mold/test/thin-archive.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_thread_count() {
    // mold/test/thread-count.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_alignment_multi() {
    // mold/test/tls-alignment-multi.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_common() {
    // mold/test/tls-common.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_df_static_tls() {
    // mold/test/tls-df-static-tls.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_dso() {
    // mold/test/tls-dso.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_gd_dlopen() {
    // mold/test/tls-gd-dlopen.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_gd_noplt() {
    // mold/test/tls-gd-noplt.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_gd_to_ie() {
    // mold/test/tls-gd-to-ie.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_gd() {
    // mold/test/tls-gd.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_ie() {
    // mold/test/tls-ie.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_irregular_start_addr() {
    // mold/test/tls-irregular-start-addr.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_large_alignment() {
    // mold/test/tls-large-alignment.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_large_static_image() {
    // mold/test/tls-large-static-image.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_ld_noplt() {
    // mold/test/tls-ld-noplt.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_ld() {
    // mold/test/tls-ld.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_le_error() {
    // mold/test/tls-le-error.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_le() {
    // mold/test/tls-le.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_nopic() {
    // mold/test/tls-nopic.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_pic() {
    // mold/test/tls-pic.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tls_small_alignment() {
    // mold/test/tls-small-alignment.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tlsdesc_dlopen() {
    // mold/test/tlsdesc-dlopen.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tlsdesc_import() {
    // mold/test/tlsdesc-import.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tlsdesc_initial_exec() {
    // mold/test/tlsdesc-initial-exec.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tlsdesc_local_dynamic() {
    // mold/test/tlsdesc-local-dynamic.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tlsdesc_static() {
    // mold/test/tlsdesc-static.sh
}

#[test]
#[ignore = "needs TLS"]
fn mold_tlsdesc() {
    // mold/test/tlsdesc.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_trace_symbol_symver() {
    // mold/test/trace-symbol-symver.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_trace_symbol() {
    // mold/test/trace-symbol.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_trace() {
    // mold/test/trace.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_undefined_glob_gc_sections() {
    // mold/test/undefined-glob-gc-sections.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_undefined_glob() {
    // mold/test/undefined-glob.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_undefined() {
    // mold/test/undefined.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_undefined2() {
    // mold/test/undefined2.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_unknown_section_type() {
    // mold/test/unknown-section-type.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_unresolved_symbols() {
    // mold/test/unresolved-symbols.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_unresolved_symbols2() {
    // mold/test/unresolved-symbols2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_use_android_relr_tags() {
    // mold/test/use-android-relr-tags.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_verbose() {
    // mold/test/verbose.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script_search_paths() {
    // mold/test/version-script-search-paths.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script() {
    // mold/test/version-script.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script10() {
    // mold/test/version-script10.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script11() {
    // mold/test/version-script11.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script12() {
    // mold/test/version-script12.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script13() {
    // mold/test/version-script13.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script14() {
    // mold/test/version-script14.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script15() {
    // mold/test/version-script15.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script16() {
    // mold/test/version-script16.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script17() {
    // mold/test/version-script17.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script18() {
    // mold/test/version-script18.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script19() {
    // mold/test/version-script19.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script2() {
    // mold/test/version-script2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script20() {
    // mold/test/version-script20.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script21() {
    // mold/test/version-script21.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script22() {
    // mold/test/version-script22.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script23() {
    // mold/test/version-script23.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script3() {
    // mold/test/version-script3.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script4() {
    // mold/test/version-script4.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script5() {
    // mold/test/version-script5.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script6() {
    // mold/test/version-script6.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script7() {
    // mold/test/version-script7.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script8() {
    // mold/test/version-script8.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_version_script9() {
    // mold/test/version-script9.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_version() {
    // mold/test/version.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_versioned_undef() {
    // mold/test/versioned-undef.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_visibility() {
    // mold/test/visibility.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_warn_common() {
    // mold/test/warn-common.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_warn_once() {
    // mold/test/warn-once.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_warn_symbol_type() {
    // mold/test/warn-symbol-type.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_warn_unresolved_symbols() {
    // mold/test/warn-unresolved-symbols.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_weak_export_dso() {
    // mold/test/weak-export-dso.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_weak_export_dso2() {
    // mold/test/weak-export-dso2.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_weak_export_exe() {
    // mold/test/weak-export-exe.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_weak_undef_dso() {
    // mold/test/weak-undef-dso.sh
}

#[test]
#[ignore = "concept covered by tests/symbols.rs::weak_undefined_is_zero; mold original needs dynamic linking"]
fn mold_weak_undef() {
    // mold/test/weak-undef.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_weak_undef2() {
    // mold/test/weak-undef2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_weak_undef4() {
    // mold/test/weak-undef4.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_weak_undef5() {
    // mold/test/weak-undef5.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_whole_archive() {
    // mold/test/whole-archive.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_wrap_lto() {
    // mold/test/wrap-lto.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_wrap() {
    // mold/test/wrap.sh
}

#[test]
#[ignore = "needs unsupported feature"]
fn mold_z_cet_report() {
    // mold/test/z-cet-report.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_z_defs() {
    // mold/test/z-defs.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_z_dynamic_undefined_weak_exe() {
    // mold/test/z-dynamic-undefined-weak-exe.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_z_dynamic_undefined_weak() {
    // mold/test/z-dynamic-undefined-weak.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_z_dynamic_undefined_weak2() {
    // mold/test/z-dynamic-undefined-weak2.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_max_page_size() {
    // mold/test/z-max-page-size.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_nodefaultlib() {
    // mold/test/z-nodefaultlib.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_z_nodump() {
    // mold/test/z-nodump.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_now() {
    // mold/test/z-now.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_origin() {
    // mold/test/z-origin.sh
}

#[test]
#[ignore = "needs dynamic linking"]
fn mold_z_pack_relative_relocs() {
    // mold/test/z-pack-relative-relocs.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_rodynamic() {
    // mold/test/z-rodynamic.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_sectionheader() {
    // mold/test/z-sectionheader.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_separate_code() {
    // mold/test/z-separate-code.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_stack_size() {
    // mold/test/z-stack-size.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_z_start_stop_visibility() {
    // mold/test/z-start-stop-visibility.sh
}

#[test]
#[ignore = "needs libc / C runtime"]
fn mold_zero_to_bss() {
    // mold/test/zero-to-bss.sh
}
