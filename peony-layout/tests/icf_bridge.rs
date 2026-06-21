#[path = "icf_bridge/abi_unique.rs"]
mod abi_unique;
#[path = "icf_bridge/address_safety.rs"]
mod address_safety;
#[path = "common/icf.rs"]
mod icf_common;
#[path = "icf_bridge/malformed_addrsig.rs"]
mod malformed_addrsig;

use icf_common::{object_with_addrsig, section, symbol};
use peony_layout::icf::compute_fold_map;
use peony_object::{Binding, InputArena, InputReloc, SectionKind, SymbolIndex, elf};

#[test]
fn icf_folds_only_addrsig_backed_address_safe_sections() {
    let mut arena = InputArena::new();
    let left_sections = vec![section(
        &mut arena,
        1,
        b".text.f",
        SectionKind::Text,
        elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        &[0xc3, 0x90],
        Vec::new(),
    )];
    let left_symbols = vec![symbol(
        0,
        b"f",
        Binding::Local,
        1,
        elf::STT_FUNC,
        elf::STV_HIDDEN,
    )];
    let left = object_with_addrsig(&mut arena, "left.o", left_sections, left_symbols, &[]);

    let right_sections = vec![section(
        &mut arena,
        1,
        b".text.g",
        SectionKind::Text,
        elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        &[0xc3, 0x90],
        Vec::new(),
    )];
    let right_symbols = vec![symbol(
        0,
        b"g",
        Binding::Local,
        1,
        elf::STT_FUNC,
        elf::STV_HIDDEN,
    )];
    let right = object_with_addrsig(&mut arena, "right.o", right_sections, right_symbols, &[]);

    let folds = compute_fold_map(&arena, &[left, right]);

    assert_eq!(folds.get(&(1, 1)), Some(&(0, 1)));
    assert_eq!(folds.len(), 1);
}

#[test]
fn icf_rejects_section_relative_address_taint() {
    let mut arena = InputArena::new();
    let tainted_sections = vec![
        section(
            &mut arena,
            1,
            b".text.f",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xc3, 0x90],
            Vec::new(),
        ),
        section(
            &mut arena,
            2,
            b".data.ptr",
            SectionKind::Data,
            elf::SHF_ALLOC | elf::SHF_WRITE,
            &[0; 8],
            vec![InputReloc {
                offset: 0,
                r_type: elf::R_X86_64_64,
                symbol: SymbolIndex(1),
                addend: 0,
            }],
        ),
    ];
    let tainted_symbols = vec![
        symbol(0, b"f", Binding::Local, 1, elf::STT_FUNC, elf::STV_HIDDEN),
        symbol(
            1,
            b"",
            Binding::Local,
            1,
            elf::STT_SECTION,
            elf::STV_DEFAULT,
        ),
    ];
    let tainted = object_with_addrsig(
        &mut arena,
        "tainted.o",
        tainted_sections,
        tainted_symbols,
        &[],
    );

    let candidate_sections = vec![section(
        &mut arena,
        1,
        b".text.g",
        SectionKind::Text,
        elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        &[0xc3, 0x90],
        Vec::new(),
    )];
    let candidate_symbols = vec![symbol(
        0,
        b"g",
        Binding::Local,
        1,
        elf::STT_FUNC,
        elf::STV_HIDDEN,
    )];
    let candidate = object_with_addrsig(
        &mut arena,
        "candidate.o",
        candidate_sections,
        candidate_symbols,
        &[],
    );

    let folds = compute_fold_map(&arena, &[tainted, candidate]);

    assert!(
        folds.is_empty(),
        "section-relative address-taking reloc must make the target section ineligible"
    );
}
