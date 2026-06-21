use std::collections::BTreeSet;

use peony_object::elf;

use super::common::{
    NamedRange,
    check_aligned_start,
    check_alignment_value,
    check_file_bounds,
    check_non_overlapping,
    section_ref_owner,
};
use crate::{
    ContributionOwnerWitness,
    LayoutSegmentWitness,
    LayoutWindowWitness,
    LayoutWitness,
    RangeBounds,
    SectionRefWitness,
    WitnessError,
};

pub(super) fn check_output_sections(
    witness: &LayoutWitness,
    load_segments: &[&LayoutSegmentWitness],
) -> Result<(), WitnessError> {
    let mut file_ranges = Vec::new();
    let mut va_ranges = Vec::new();
    let mut contribution_owners = BTreeSet::new();

    for section in &witness.output_sections {
        check_section_window(witness, load_segments, section)?;
        check_contributions(section, &mut contribution_owners)?;
        push_section_ranges(section, &mut file_ranges, &mut va_ranges);
    }

    check_non_overlapping("output section file", &mut file_ranges)?;
    check_non_overlapping("output section virtual", &mut va_ranges)
}

fn check_section_window(
    witness: &LayoutWitness,
    load_segments: &[&LayoutSegmentWitness],
    section: &LayoutWindowWitness,
) -> Result<(), WitnessError> {
    let owner = section.output_section_name.clone();
    check_alignment_value(&owner, section.alignment, false)?;
    check_file_bounds(&owner, section.range.file, witness.file_size)?;
    if !section.range.file.is_empty() {
        check_aligned_start(&owner, section.range.file.start, section.alignment)?;
    }

    if section.flags & elf::SHF_ALLOC == 0 {
        return check_non_alloc_virtual_range(section, owner);
    }
    check_alloc_section_window(witness, load_segments, section, owner)
}

fn check_non_alloc_virtual_range(
    section: &LayoutWindowWitness,
    owner: String,
) -> Result<(), WitnessError> {
    if section.range.va.is_empty() {
        return Ok(());
    }
    Err(WitnessError::LayoutNonAllocVirtualAddress {
        owner,
        virtual_end: section.range.va.end,
    })
}

fn check_alloc_section_window(
    witness: &LayoutWitness,
    load_segments: &[&LayoutSegmentWitness],
    section: &LayoutWindowWitness,
    owner: String,
) -> Result<(), WitnessError> {
    check_aligned_start(&owner, section.range.va.start, section.alignment)?;
    if section.range.va.start < witness.image_base
        || section.range.va.start - witness.image_base != section.range.file.start
    {
        return Err(WitnessError::LayoutPageCongruence {
            owner,
            file_start: section.range.file.start,
            virtual_start: section.range.va.start,
            alignment: section.alignment,
        });
    }
    if section_is_in_load_segments(section, load_segments) {
        return Ok(());
    }
    Err(WitnessError::LayoutSectionOutsideLoadSegment {
        section: section.output_section_name.clone(),
    })
}

fn section_is_in_load_segments(
    section: &LayoutWindowWitness,
    load_segments: &[&LayoutSegmentWitness],
) -> bool {
    load_segments
        .iter()
        .any(|segment| segment.range.va.contains(section.range.va))
        && (section.range.file.is_empty()
            || load_segments
                .iter()
                .any(|segment| segment.range.file.contains(section.range.file)))
}

fn check_contributions(
    section: &LayoutWindowWitness,
    contribution_owners: &mut BTreeSet<SectionRefWitness>,
) -> Result<(), WitnessError> {
    let section_size = section.range.file.len().max(section.range.va.len());
    let mut prior: Option<&ContributionOwnerWitness> = None;
    for contribution in &section.contributions {
        check_contribution_range(section, contribution, section_size)?;
        check_contribution_order(prior, contribution)?;
        if !contribution_owners.insert(contribution.section) {
            return Err(WitnessError::LayoutDuplicateContributionOwner {
                section: contribution.section,
            });
        }
        prior = Some(contribution);
    }
    Ok(())
}

fn check_contribution_range(
    section: &LayoutWindowWitness,
    contribution: &ContributionOwnerWitness,
    section_size: u64,
) -> Result<(), WitnessError> {
    let computed = RangeBounds::from_start_len(contribution.output_offset, contribution.size)?;
    if computed != contribution.range {
        return Err(WitnessError::LayoutContributionRangeMismatch {
            output_section: section.output_section_name.clone(),
            section: contribution.section,
            declared_start: contribution.range.start,
            declared_end: contribution.range.end,
            computed_start: computed.start,
            computed_end: computed.end,
        });
    }
    if contribution.range.end <= section_size {
        return Ok(());
    }
    Err(WitnessError::LayoutContributionOutOfBounds {
        output_section: section.output_section_name.clone(),
        section: contribution.section,
        contribution_end: contribution.range.end,
        section_size,
    })
}

fn check_contribution_order(
    prior: Option<&ContributionOwnerWitness>,
    contribution: &ContributionOwnerWitness,
) -> Result<(), WitnessError> {
    let Some(previous) = prior else {
        return Ok(());
    };
    if previous.range.end <= contribution.range.start {
        return Ok(());
    }
    Err(WitnessError::LayoutRangeOverlap {
        kind: "contribution",
        first: section_ref_owner(previous.section),
        second: section_ref_owner(contribution.section),
        first_end: previous.range.end,
        second_start: contribution.range.start,
    })
}

fn push_section_ranges(
    section: &LayoutWindowWitness,
    file_ranges: &mut Vec<NamedRange>,
    va_ranges: &mut Vec<NamedRange>,
) {
    if !section.range.file.is_empty() {
        file_ranges.push(NamedRange {
            owner: section.output_section_name.clone(),
            range: section.range.file,
        });
    }
    if section.flags & elf::SHF_ALLOC != 0 && !section.range.va.is_empty() {
        va_ranges.push(NamedRange {
            owner: section.output_section_name.clone(),
            range: section.range.va,
        });
    }
}
