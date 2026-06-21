use peony_object::{Binding, InputArena, InputObject, InputSection, SectionKind, elf};

use super::model::{
    IcfFoldKeyWitness,
    IcfFoldPairWitness,
    IcfFoldWitness,
    IcfRelocationSummaryWitness,
    IcfSectionWitness,
};
use super::taint::AddressTaint;
use crate::SectionRefWitness;

pub fn extract_icf_fold_witnesses(
    arena: &InputArena,
    objects: &[InputObject],
) -> Vec<IcfFoldWitness> {
    let taint = AddressTaint::extract(arena, objects);
    fold_pairs(arena, objects)
        .into_iter()
        .filter_map(|pair| fold_witness(arena, objects, &taint, pair))
        .collect()
}

pub(super) fn fold_pairs(arena: &InputArena, objects: &[InputObject]) -> Vec<IcfFoldPairWitness> {
    let mut pairs: Vec<_> = peony_layout::icf::compute_fold_map(arena, objects)
        .into_iter()
        .map(|(duplicate, canonical)| IcfFoldPairWitness {
            duplicate: SectionRefWitness::new(duplicate.0, duplicate.1),
            canonical: SectionRefWitness::new(canonical.0, canonical.1),
        })
        .collect();
    pairs.sort_unstable();
    pairs
}

pub(super) fn fold_witness(
    arena: &InputArena,
    objects: &[InputObject],
    taint: &AddressTaint,
    pair: IcfFoldPairWitness,
) -> Option<IcfFoldWitness> {
    let duplicate_section = section_witness(objects, taint, pair.duplicate)?;
    let canonical_section = section_witness(objects, taint, pair.canonical)?;
    let duplicate_key = fold_key(arena, objects, pair.duplicate)?;
    let canonical_key = fold_key(arena, objects, pair.canonical)?;
    let duplicate_bytes = section_bytes(arena, objects, pair.duplicate)?;
    let canonical_bytes = section_bytes(arena, objects, pair.canonical)?;
    let flags_equal = duplicate_key.flags == canonical_key.flags;
    let len_equal = duplicate_key.len == canonical_key.len;
    let bytes_equal = duplicate_bytes == canonical_bytes;
    let relocation_summaries_equal =
        duplicate_key.relocation_summaries == canonical_key.relocation_summaries;
    let address_taint_known =
        duplicate_section.object_has_addrsig && canonical_section.object_has_addrsig;
    let address_safe = duplicate_section.address_safe && canonical_section.address_safe;
    Some(IcfFoldWitness {
        duplicate: pair.duplicate,
        canonical: pair.canonical,
        duplicate_key,
        canonical_key,
        duplicate_section,
        canonical_section,
        flags_equal,
        len_equal,
        bytes_equal,
        relocation_summaries_equal,
        address_taint_known,
        address_safe,
    })
}

fn section_witness(
    objects: &[InputObject],
    taint: &AddressTaint,
    section: SectionRefWitness,
) -> Option<IcfSectionWitness> {
    let object = objects.get(section.object_id)?;
    let input_section = object.section_by_index(section.section_index)?;
    let mut facts = SectionAddressFacts::default();
    for (symbol_pos, symbol) in object.symbols.iter().enumerate() {
        if symbol.section.map(|owner| owner.0) != Some(section.section_index) || symbol.is_undefined
        {
            continue;
        }
        facts.has_addrsig_symbol |= taint
            .addrsig_symbols
            .contains(&(section.object_id, symbol_pos));
        if symbol.name.is_empty() {
            continue;
        }
        let name = symbol.name.as_bytes();
        facts.has_abi_unique_symbol |=
            name.starts_with(b"_ZTV") || name.starts_with(b"_ZTI") || name.starts_with(b"_ZTS");
        facts.has_named_address_taken_symbol |= taint.by_name.contains(name);
        facts.has_weak_definition |= matches!(symbol.binding, Binding::Weak);
        facts.has_default_visible_non_local_definition |=
            !matches!(symbol.binding, Binding::Local) && symbol.visibility == elf::STV_DEFAULT;
    }
    let object_has_addrsig = taint.objects_with_addrsig.contains(&section.object_id);
    let section_address_taken = taint.by_section.contains(&section);
    let address_safe = object_has_addrsig
        && !section_address_taken
        && !facts.has_addrsig_symbol
        && !facts.has_named_address_taken_symbol
        && !facts.has_abi_unique_symbol
        && !facts.has_weak_definition
        && !facts.has_default_visible_non_local_definition;
    let is_text = input_section.kind == SectionKind::Text;
    let has_contents = !input_section.data.is_empty();
    let reloc_targets_resolved = relocation_summaries(object, input_section)
        .iter()
        .all(|reloc| reloc.target_name.is_some());
    let fold_eligible = is_text && has_contents && reloc_targets_resolved && address_safe;
    Some(IcfSectionWitness {
        section,
        is_text,
        has_contents,
        object_has_addrsig,
        section_address_taken,
        has_addrsig_symbol: facts.has_addrsig_symbol,
        has_named_address_taken_symbol: facts.has_named_address_taken_symbol,
        has_abi_unique_symbol: facts.has_abi_unique_symbol,
        has_weak_definition: facts.has_weak_definition,
        has_default_visible_non_local_definition: facts.has_default_visible_non_local_definition,
        reloc_targets_resolved,
        address_safe,
        fold_eligible,
    })
}

fn fold_key(
    arena: &InputArena,
    objects: &[InputObject],
    section: SectionRefWitness,
) -> Option<IcfFoldKeyWitness> {
    let object = objects.get(section.object_id)?;
    let input_section = object.section_by_index(section.section_index)?;
    let bytes = arena.bytes(input_section.data);
    Some(IcfFoldKeyWitness {
        flags: input_section.flags,
        len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        content_digest: content_digest(bytes),
        relocation_summaries: relocation_summaries(object, input_section),
    })
}

fn section_bytes<'a>(
    arena: &'a InputArena,
    objects: &'a [InputObject],
    section: SectionRefWitness,
) -> Option<&'a [u8]> {
    let object = objects.get(section.object_id)?;
    let input_section = object.section_by_index(section.section_index)?;
    Some(arena.bytes(input_section.data))
}

fn relocation_summaries(
    object: &InputObject,
    section: &InputSection,
) -> Vec<IcfRelocationSummaryWitness> {
    section
        .relocs
        .iter()
        .map(|reloc| IcfRelocationSummaryWitness {
            offset: reloc.offset,
            relocation_type: reloc.r_type,
            addend: reloc.addend,
            target_name: object
                .symbol_by_index(reloc.symbol.0)
                .map(|symbol| symbol.name.as_bytes().to_vec()),
        })
        .collect()
}

fn content_digest(bytes: &[u8]) -> u128 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h1 = OFFSET;
    let mut h2 = 0x9e37_79b9_7f4a_7c15u64;
    for &byte in bytes {
        let value = u64::from(byte);
        h1 = (h1 ^ value).wrapping_mul(PRIME);
        h2 = (h2.wrapping_add(value)).wrapping_mul(PRIME) ^ (h2 >> 29);
    }
    (u128::from(h1) << 64) | u128::from(h2)
}

#[derive(Default)]
struct SectionAddressFacts {
    has_addrsig_symbol: bool,
    has_named_address_taken_symbol: bool,
    has_abi_unique_symbol: bool,
    has_weak_definition: bool,
    has_default_visible_non_local_definition: bool,
}
