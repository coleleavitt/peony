use crate::{LayoutSegmentWitness, RangeBounds, SectionRefWitness, WitnessError};

pub(super) struct NamedRange {
    pub(super) owner: String,
    pub(super) range: RangeBounds,
}

pub(super) fn check_alignment_value(
    owner: &str,
    alignment: u64,
    allow_zero: bool,
) -> Result<(), WitnessError> {
    if allow_zero && alignment == 0 {
        return Ok(());
    }
    if alignment != 0 && alignment.is_power_of_two() {
        return Ok(());
    }
    Err(WitnessError::LayoutInvalidAlignment {
        owner: owner.to_string(),
        alignment,
    })
}

pub(super) fn check_aligned_start(
    owner: &str,
    start: u64,
    alignment: u64,
) -> Result<(), WitnessError> {
    if alignment <= 1 || start.is_multiple_of(alignment) {
        return Ok(());
    }
    Err(WitnessError::LayoutMisaligned {
        owner: owner.to_string(),
        start,
        alignment,
    })
}

pub(super) fn check_file_bounds(
    owner: &str,
    range: RangeBounds,
    file_size: u64,
) -> Result<(), WitnessError> {
    if range.end <= file_size {
        return Ok(());
    }
    Err(WitnessError::LayoutFileOutOfBounds {
        owner: owner.to_string(),
        range_end: range.end,
        file_size,
    })
}

pub(super) fn check_non_overlapping(
    kind: &'static str,
    ranges: &mut [NamedRange],
) -> Result<(), WitnessError> {
    ranges.sort_by_key(|named| (named.range.start, named.range.end));
    for pair in ranges.windows(2) {
        let first = &pair[0];
        let second = &pair[1];
        if first.range.end > second.range.start {
            return Err(WitnessError::LayoutRangeOverlap {
                kind,
                first: first.owner.clone(),
                second: second.owner.clone(),
                first_end: first.range.end,
                second_start: second.range.start,
            });
        }
    }
    Ok(())
}

pub(super) fn segment_owner(segment: &LayoutSegmentWitness) -> String {
    format!(
        "program header {} type {:#x}",
        segment.index, segment.segment_type
    )
}

pub(super) fn section_ref_owner(section: SectionRefWitness) -> String {
    format!(
        "object {} section {}",
        section.object_id, section.section_index
    )
}
