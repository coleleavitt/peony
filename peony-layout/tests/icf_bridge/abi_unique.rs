use peony_layout::icf::compute_fold_map;
use peony_object::{Binding, InputArena, InputObject, InputSection, InputSymbol, SectionKind, elf};

use crate::icf_common::{object_with_addrsig, section, symbol};

#[test]
fn icf_rejects_abi_unique_vtable_name() {
    // Given: one candidate defines a C++ ABI vtable name.
    let mut arena = InputArena::new();
    let objects = abi_unique_candidate_pair(&mut arena, b"_ZTV1A");

    // When: ICF computes folds for byte-identical local hidden text sections.
    let folds = compute_fold_map(&arena, &objects);

    // Then: the `_ZTV` address-identity source prevents folding.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_abi_unique_typeinfo_name() {
    // Given: one candidate defines a C++ ABI typeinfo object name.
    let mut arena = InputArena::new();
    let objects = abi_unique_candidate_pair(&mut arena, b"_ZTI1A");

    // When: ICF computes folds for byte-identical local hidden text sections.
    let folds = compute_fold_map(&arena, &objects);

    // Then: the isolated `_ZTI` address-identity source prevents folding.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_abi_unique_typeinfo_name_string() {
    // Given: one candidate defines a C++ ABI typeinfo-name string.
    let mut arena = InputArena::new();
    let objects = abi_unique_candidate_pair(&mut arena, b"_ZTS1A");

    // When: ICF computes folds for byte-identical local hidden text sections.
    let folds = compute_fold_map(&arena, &objects);

    // Then: the `_ZTS` address-identity source prevents folding.
    assert!(folds.is_empty());
}

fn abi_unique_candidate_pair(arena: &mut InputArena, abi_name: &[u8]) -> Vec<InputObject> {
    let left_sections = vec![text_section(arena, b".text.f")];
    let right_sections = vec![text_section(arena, b".text.g")];
    vec![
        object_with_addrsig(
            arena,
            "left.o",
            left_sections,
            vec![local_hidden_func(abi_name)],
            &[],
        ),
        object_with_addrsig(
            arena,
            "right.o",
            right_sections,
            vec![local_hidden_func(b"g")],
            &[],
        ),
    ]
}

fn text_section(arena: &mut InputArena, name: &[u8]) -> InputSection {
    section(
        arena,
        1,
        name,
        SectionKind::Text,
        elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        &[0xc3, 0x90],
        Vec::new(),
    )
}

fn local_hidden_func(name: &[u8]) -> InputSymbol {
    symbol(0, name, Binding::Local, 1, elf::STT_FUNC, elf::STV_HIDDEN)
}
