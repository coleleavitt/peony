mod fixtures;

use fixtures::{fold_objects_with_raw_addrsig, fold_objects_with_text_relocs, safe_fold_objects};
use peony_object::{InputArena, InputReloc, SymbolIndex, elf};

use crate::{
    IcfWitnessError,
    SectionRefWitness,
    check_icf_fold_witnesses,
    extract_icf_fold_witnesses,
};

#[test]
fn icf_fold_witness_accepts_addrsig_backed_local_sections_when_fold_is_address_safe() {
    // Given: two byte-identical local text sections with explicit addrsig opt-in.
    let mut arena = InputArena::new();
    let objects = safe_fold_objects(&mut arena, true, Vec::new());

    // When: I1 extracts and checks fold witnesses for the current ICF path.
    let witnesses = extract_icf_fold_witnesses(&arena, &objects);
    let checked = check_icf_fold_witnesses(&arena, &objects, &witnesses);

    // Then: the duplicate folds to the canonical and satisfies the address-safe premise.
    checked.expect("address-safe ICF witness checks");
    assert_eq!(witnesses.len(), 1);
    let witness = &witnesses[0];
    assert_eq!(witness.canonical, SectionRefWitness::new(0, 1));
    assert_eq!(witness.duplicate, SectionRefWitness::new(1, 1));
    assert!(witness.bytes_equal);
    assert!(witness.relocation_summaries_equal);
    assert!(witness.address_taint_known);
    assert!(witness.address_safe);
    assert!(witness.canonical_section.fold_eligible);
    assert!(witness.duplicate_section.fold_eligible);
}

#[test]
fn icf_fold_witness_rejects_named_address_taint_when_witness_claims_safe() {
    // Given: a witness from a safe fold and equivalent objects where `&f` is taken.
    let mut safe_arena = InputArena::new();
    let safe_objects = safe_fold_objects(&mut safe_arena, true, Vec::new());
    let witnesses = extract_icf_fold_witnesses(&safe_arena, &safe_objects);
    let mut tainted_arena = InputArena::new();
    let objects = safe_fold_objects(
        &mut tainted_arena,
        true,
        vec![InputReloc {
            offset: 0,
            r_type: elf::R_X86_64_64,
            symbol: SymbolIndex(0),
            addend: 0,
        }],
    );

    // When: the stale safe witness is checked against the tainted input.
    let err = check_icf_fold_witnesses(&tainted_arena, &objects, &witnesses)
        .expect_err("named address taint must reject I1");

    // Then: I1 rejects the bridge claim instead of assuming address_safe.
    assert!(matches!(
        err,
        IcfWitnessError::SectionPredicateMismatch { .. } | IcfWitnessError::AddressUnsafe { .. }
    ));
}

#[test]
fn icf_fold_witness_rejects_missing_addrsig_when_address_taint_is_unknown() {
    // Given: a witness from addrsig-backed objects and equivalent objects without addrsig.
    let mut safe_arena = InputArena::new();
    let safe_objects = safe_fold_objects(&mut safe_arena, true, Vec::new());
    let witnesses = extract_icf_fold_witnesses(&safe_arena, &safe_objects);
    let mut unknown_arena = InputArena::new();
    let objects = safe_fold_objects(&mut unknown_arena, false, Vec::new());

    // When: the stale safe witness is checked against unsupported address-taint input.
    let err = check_icf_fold_witnesses(&unknown_arena, &objects, &witnesses)
        .expect_err("missing addrsig must reject I1");

    // Then: unknown address significance is not promoted to address_safe.
    assert!(matches!(
        err,
        IcfWitnessError::SectionPredicateMismatch { .. } | IcfWitnessError::AddressUnsafe { .. }
    ));
}

#[test]
fn icf_fold_witness_rejects_malformed_addrsig_when_address_taint_is_unknown() {
    // Given: a witness from addrsig-backed objects and equivalent objects with invalid addrsig.
    let mut safe_arena = InputArena::new();
    let safe_objects = safe_fold_objects(&mut safe_arena, true, Vec::new());
    let witnesses = extract_icf_fold_witnesses(&safe_arena, &safe_objects);
    let mut malformed_arena = InputArena::new();
    let objects = fold_objects_with_raw_addrsig(&mut malformed_arena, &[0x80]);

    // When: the stale safe witness is checked against unsupported addrsig input.
    let err = check_icf_fold_witnesses(&malformed_arena, &objects, &witnesses)
        .expect_err("malformed addrsig must reject I1");

    // Then: parser-unknown address significance is not promoted to address_safe.
    assert!(matches!(
        err,
        IcfWitnessError::SectionPredicateMismatch { .. } | IcfWitnessError::AddressUnsafe { .. }
    ));
}

#[test]
fn icf_fold_witness_rejects_unresolved_relocation_target_when_fold_key_is_unknown() {
    // Given: a witness from resolved objects and equivalent objects with unknown relocs.
    let mut safe_arena = InputArena::new();
    let safe_objects = safe_fold_objects(&mut safe_arena, true, Vec::new());
    let witnesses = extract_icf_fold_witnesses(&safe_arena, &safe_objects);
    let mut unresolved_arena = InputArena::new();
    let unresolved_reloc = InputReloc {
        offset: 0,
        r_type: 2,
        symbol: SymbolIndex(99),
        addend: 0,
    };
    let objects = fold_objects_with_text_relocs(&mut unresolved_arena, unresolved_reloc);

    // When: the stale safe witness is checked against unresolved fold-key input.
    let err = check_icf_fold_witnesses(&unresolved_arena, &objects, &witnesses)
        .expect_err("unresolved fold key must reject I1");

    // Then: unresolved eligibility rejects the bridge claim before theorem use.
    assert!(matches!(
        err,
        IcfWitnessError::SectionPredicateMismatch { .. }
            | IcfWitnessError::FoldPredicateMismatch { .. }
    ));
}
