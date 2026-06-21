use std::collections::BTreeSet;

use peony_object::{InputArena, InputObject};

use super::extract::{fold_pairs, fold_witness};
use super::model::{IcfFoldKeyWitness, IcfFoldWitness, IcfSectionWitness, IcfWitnessError};
use super::taint::AddressTaint;

pub fn check_icf_fold_witnesses(
    arena: &InputArena,
    objects: &[InputObject],
    witnesses: &[IcfFoldWitness],
) -> Result<(), IcfWitnessError> {
    let taint = AddressTaint::extract(arena, objects);
    for witness in witnesses {
        check_one_witness(arena, objects, &taint, witness)?;
    }
    let actual: BTreeSet<_> = fold_pairs(arena, objects).into_iter().collect();
    let supplied: BTreeSet<_> = witnesses.iter().map(IcfFoldWitness::pair).collect();
    if actual == supplied {
        return Ok(());
    }
    Err(IcfWitnessError::FoldSetMismatch {
        missing: actual.difference(&supplied).copied().collect(),
        unexpected: supplied.difference(&actual).copied().collect(),
    })
}

fn check_one_witness(
    arena: &InputArena,
    objects: &[InputObject],
    taint: &AddressTaint,
    witness: &IcfFoldWitness,
) -> Result<(), IcfWitnessError> {
    let computed = fold_witness(arena, objects, taint, witness.pair()).ok_or(
        IcfWitnessError::MissingSection {
            section: witness.duplicate,
        },
    )?;
    require_section(&witness.duplicate_section, &computed.duplicate_section)?;
    require_section(&witness.canonical_section, &computed.canonical_section)?;
    require_fold_bool(
        witness,
        "flags_equal",
        witness.flags_equal,
        computed.flags_equal,
    )?;
    require_fold_bool(witness, "len_equal", witness.len_equal, computed.len_equal)?;
    require_fold_bool(
        witness,
        "bytes_equal",
        witness.bytes_equal,
        computed.bytes_equal,
    )?;
    require_fold_bool(
        witness,
        "relocation_summaries_equal",
        witness.relocation_summaries_equal,
        computed.relocation_summaries_equal,
    )?;
    require_fold_bool(
        witness,
        "address_taint_known",
        witness.address_taint_known,
        computed.address_taint_known,
    )?;
    require_fold_bool(
        witness,
        "address_safe",
        witness.address_safe,
        computed.address_safe,
    )?;
    require_fold_key(
        witness,
        &computed,
        "duplicate_key",
        &witness.duplicate_key,
        &computed.duplicate_key,
    )?;
    require_fold_key(
        witness,
        &computed,
        "canonical_key",
        &witness.canonical_key,
        &computed.canonical_key,
    )?;
    if !computed.duplicate_section.fold_eligible || !computed.canonical_section.fold_eligible {
        return Err(IcfWitnessError::AddressUnsafe {
            duplicate: witness.duplicate,
            canonical: witness.canonical,
            duplicate_safe: computed.duplicate_section.address_safe,
            canonical_safe: computed.canonical_section.address_safe,
        });
    }
    Ok(())
}

fn require_section(
    witness: &IcfSectionWitness,
    computed: &IcfSectionWitness,
) -> Result<(), IcfWitnessError> {
    require_section_bool(witness, "is_text", witness.is_text, computed.is_text)?;
    require_section_bool(
        witness,
        "has_contents",
        witness.has_contents,
        computed.has_contents,
    )?;
    require_section_bool(
        witness,
        "object_has_addrsig",
        witness.object_has_addrsig,
        computed.object_has_addrsig,
    )?;
    require_section_bool(
        witness,
        "section_address_taken",
        witness.section_address_taken,
        computed.section_address_taken,
    )?;
    require_section_bool(
        witness,
        "has_addrsig_symbol",
        witness.has_addrsig_symbol,
        computed.has_addrsig_symbol,
    )?;
    require_section_bool(
        witness,
        "has_named_address_taken_symbol",
        witness.has_named_address_taken_symbol,
        computed.has_named_address_taken_symbol,
    )?;
    require_section_bool(
        witness,
        "has_abi_unique_symbol",
        witness.has_abi_unique_symbol,
        computed.has_abi_unique_symbol,
    )?;
    require_section_bool(
        witness,
        "has_weak_definition",
        witness.has_weak_definition,
        computed.has_weak_definition,
    )?;
    require_section_bool(
        witness,
        "has_default_visible_non_local_definition",
        witness.has_default_visible_non_local_definition,
        computed.has_default_visible_non_local_definition,
    )?;
    require_section_bool(
        witness,
        "reloc_targets_resolved",
        witness.reloc_targets_resolved,
        computed.reloc_targets_resolved,
    )?;
    require_section_bool(
        witness,
        "address_safe",
        witness.address_safe,
        computed.address_safe,
    )?;
    require_section_bool(
        witness,
        "fold_eligible",
        witness.fold_eligible,
        computed.fold_eligible,
    )
}

fn require_section_bool(
    witness: &IcfSectionWitness,
    field: &'static str,
    witness_value: bool,
    computed_value: bool,
) -> Result<(), IcfWitnessError> {
    if witness_value == computed_value {
        return Ok(());
    }
    Err(IcfWitnessError::SectionPredicateMismatch {
        section: witness.section,
        field,
        witness: witness_value,
        computed: computed_value,
    })
}

fn require_fold_bool(
    witness: &IcfFoldWitness,
    field: &'static str,
    witness_value: bool,
    computed_value: bool,
) -> Result<(), IcfWitnessError> {
    if witness_value == computed_value {
        return Ok(());
    }
    Err(IcfWitnessError::FoldPredicateMismatch {
        duplicate: witness.duplicate,
        canonical: witness.canonical,
        field,
    })
}

fn require_fold_key(
    witness: &IcfFoldWitness,
    computed: &IcfFoldWitness,
    field: &'static str,
    witness_key: &IcfFoldKeyWitness,
    computed_key: &IcfFoldKeyWitness,
) -> Result<(), IcfWitnessError> {
    if witness_key == computed_key {
        return Ok(());
    }
    Err(IcfWitnessError::FoldPredicateMismatch {
        duplicate: witness.duplicate,
        canonical: computed.canonical,
        field,
    })
}
