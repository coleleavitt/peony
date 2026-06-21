use peony_layout::icf::compute_fold_map;
use peony_object::{
    Binding,
    InputArena,
    InputObject,
    InputReloc,
    InputSection,
    InputSymbol,
    SectionKind,
    SymbolIndex,
    elf,
};

use crate::icf_common::{object_with_addrsig, object_without_addrsig, section, symbol};

#[test]
fn icf_rejects_named_function_address_taint() {
    // Given: one candidate function has its address taken through a named relocation.
    let mut arena = InputArena::new();
    let address_reloc = InputReloc {
        offset: 0,
        r_type: elf::R_X86_64_64,
        symbol: SymbolIndex(0),
        addend: 0,
    };
    let left_sections = text_sections_with_optional_data_reloc(&mut arena, vec![address_reloc]);
    let right_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let objects = candidate_pair(
        &mut arena,
        CandidatePairInput {
            left_sections,
            right_sections,
            left_symbol: local_hidden_func(0, b"f"),
            right_symbol: local_hidden_func(0, b"g"),
        },
    );

    // When: ICF computes folds for otherwise byte-identical text sections.
    let folds = compute_fold_map(&arena, &objects);

    // Then: named address-taking taint prevents folding.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_addrsig_listed_symbol() {
    // Given: the compiler marks one candidate symbol address-significant.
    let mut arena = InputArena::new();
    let left_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let left = object_with_addrsig(
        &mut arena,
        "left.o",
        left_sections,
        vec![local_hidden_func(0, b"f")],
        &[0],
    );
    let right_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let right = object_with_addrsig(
        &mut arena,
        "right.o",
        right_sections,
        vec![local_hidden_func(0, b"g")],
        &[],
    );

    // When: ICF sees byte-identical candidate text sections.
    let folds = compute_fold_map(&arena, &[left, right]);

    // Then: `.llvm_addrsig` significance prevents folding.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_default_visible_global_export() {
    // Given: both candidate functions are externally visible definitions.
    let mut arena = InputArena::new();
    let left_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let right_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let objects = candidate_pair(
        &mut arena,
        CandidatePairInput {
            left_sections,
            right_sections,
            left_symbol: symbol(0, b"f", Binding::Global, 1, elf::STT_FUNC, elf::STV_DEFAULT),
            right_symbol: symbol(0, b"g", Binding::Global, 1, elf::STT_FUNC, elf::STV_DEFAULT),
        },
    );

    // When: ICF computes folds for byte-identical text sections.
    let folds = compute_fold_map(&arena, &objects);

    // Then: exported/default-visible identity prevents folding.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_weak_definition() {
    // Given: both candidate functions are weak but hidden definitions.
    let mut arena = InputArena::new();
    let left_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let right_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let objects = candidate_pair(
        &mut arena,
        CandidatePairInput {
            left_sections,
            right_sections,
            left_symbol: symbol(0, b"f", Binding::Weak, 1, elf::STT_FUNC, elf::STV_HIDDEN),
            right_symbol: symbol(0, b"g", Binding::Weak, 1, elf::STT_FUNC, elf::STV_HIDDEN),
        },
    );

    // When: ICF computes folds for byte-identical text sections.
    let folds = compute_fold_map(&arena, &objects);

    // Then: weak-definition semantics prevent folding.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_missing_addrsig_as_unknown_address_safety() {
    // Given: otherwise foldable objects omit `.llvm_addrsig`.
    let mut arena = InputArena::new();
    let left_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let left = object_without_addrsig("left.o", left_sections, vec![local_hidden_func(0, b"f")]);
    let right_sections = text_sections_with_optional_data_reloc(&mut arena, Vec::new());
    let right = object_without_addrsig("right.o", right_sections, vec![local_hidden_func(0, b"g")]);

    // When: ICF computes folds without compiler address-safety metadata.
    let folds = compute_fold_map(&arena, &[left, right]);

    // Then: unknown address significance prevents folding.
    assert!(folds.is_empty());
}

#[test]
fn icf_rejects_unresolved_relocation_target_as_unknown_fold_key() {
    // Given: candidate text sections contain an unresolved non-address relocation target.
    let mut arena = InputArena::new();
    let unknown_reloc = InputReloc {
        offset: 0,
        r_type: 2,
        symbol: SymbolIndex(99),
        addend: 0,
    };
    let left_sections = vec![text_section_with_reloc(
        &mut arena,
        vec![unknown_reloc.clone()],
    )];
    let right_sections = vec![text_section_with_reloc(&mut arena, vec![unknown_reloc])];
    let objects = candidate_pair(
        &mut arena,
        CandidatePairInput {
            left_sections,
            right_sections,
            left_symbol: local_hidden_func(0, b"f"),
            right_symbol: local_hidden_func(0, b"g"),
        },
    );

    // When: ICF cannot resolve the fold-key relocation target name.
    let folds = compute_fold_map(&arena, &objects);

    // Then: unknown fold-key eligibility prevents folding.
    assert!(folds.is_empty());
}

struct CandidatePairInput {
    left_sections: Vec<InputSection>,
    right_sections: Vec<InputSection>,
    left_symbol: InputSymbol,
    right_symbol: InputSymbol,
}

fn candidate_pair(arena: &mut InputArena, input: CandidatePairInput) -> Vec<InputObject> {
    vec![
        object_with_addrsig(
            arena,
            "left.o",
            input.left_sections,
            vec![input.left_symbol],
            &[],
        ),
        object_with_addrsig(
            arena,
            "right.o",
            input.right_sections,
            vec![input.right_symbol],
            &[],
        ),
    ]
}

fn text_sections_with_optional_data_reloc(
    arena: &mut InputArena,
    data_relocs: Vec<InputReloc>,
) -> Vec<InputSection> {
    let mut sections = vec![text_section_with_reloc(arena, Vec::new())];
    if !data_relocs.is_empty() {
        sections.push(section(
            arena,
            2,
            b".data.ptr",
            SectionKind::Data,
            elf::SHF_ALLOC | elf::SHF_WRITE,
            &[0; 8],
            data_relocs,
        ));
    }
    sections
}

fn text_section_with_reloc(arena: &mut InputArena, relocs: Vec<InputReloc>) -> InputSection {
    section(
        arena,
        1,
        b".text.f",
        SectionKind::Text,
        elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        &[0xc3, 0x90],
        relocs,
    )
}

fn local_hidden_func(index: usize, name: &[u8]) -> InputSymbol {
    symbol(
        index,
        name,
        Binding::Local,
        1,
        elf::STT_FUNC,
        elf::STV_HIDDEN,
    )
}
