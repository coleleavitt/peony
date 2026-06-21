use peony_layout::icf::compute_fold_map;
use peony_object::{Binding, InputArena, InputObject, InputSection, InputSymbol, SectionKind, elf};

use crate::icf_common::{RawAddrsigObject, object_with_raw_addrsig, section, symbol};

#[test]
fn icf_rejects_truncated_addrsig_as_unknown_address_safety() {
    // Given: otherwise foldable objects carry a truncated ULEB128 addrsig entry.
    let mut arena = InputArena::new();
    let objects = raw_addrsig_candidate_pair(&mut arena, &[0x80]);

    // When: ICF computes folds from unsupported addrsig parser input.
    let folds = compute_fold_map(&arena, &objects);

    // Then: malformed addrsig does not become address-safe metadata.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_overwide_addrsig_as_unknown_address_safety() {
    // Given: otherwise foldable objects carry an overwide unterminated ULEB128.
    let mut arena = InputArena::new();
    let objects = raw_addrsig_candidate_pair(&mut arena, &[0x80; 10]);

    // When: ICF computes folds from unsupported addrsig parser input.
    let folds = compute_fold_map(&arena, &objects);

    // Then: malformed addrsig does not become address-safe metadata.
    assert!(folds.is_empty());
}

fn raw_addrsig_candidate_pair(arena: &mut InputArena, data: &[u8]) -> Vec<InputObject> {
    let left_sections = vec![text_section(arena, b".text.f")];
    let right_sections = vec![text_section(arena, b".text.g")];
    vec![
        object_with_raw_addrsig(
            arena,
            RawAddrsigObject {
                path: "left.o",
                sections: left_sections,
                symbols: vec![local_hidden_func(b"f")],
                data,
            },
        ),
        object_with_raw_addrsig(
            arena,
            RawAddrsigObject {
                path: "right.o",
                sections: right_sections,
                symbols: vec![local_hidden_func(b"g")],
                data,
            },
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
