use peony_object::elf;

use super::common::{
    NamedRange,
    check_alignment_value,
    check_file_bounds,
    check_non_overlapping,
    segment_owner,
};
use crate::{LayoutSegmentWitness, LayoutWitness, WitnessError};

pub(super) fn check_segments(
    witness: &LayoutWitness,
) -> Result<Vec<&LayoutSegmentWitness>, WitnessError> {
    let mut load_file_ranges = Vec::new();
    let mut load_va_ranges = Vec::new();
    let mut load_segments = Vec::new();

    for segment in &witness.segments {
        let owner = segment_owner(segment);
        check_alignment_value(&owner, segment.alignment, true)?;
        check_file_bounds(&owner, segment.range.file, witness.file_size)?;
        if segment.range.file.len() > segment.range.va.len() {
            return Err(WitnessError::LayoutSegmentFileLargerThanMemory {
                segment: owner,
                file_size: segment.range.file.len(),
                memory_size: segment.range.va.len(),
            });
        }
        if segment.alignment > 1
            && segment.range.file.start % segment.alignment
                != segment.range.va.start % segment.alignment
        {
            return Err(WitnessError::LayoutPageCongruence {
                owner,
                file_start: segment.range.file.start,
                virtual_start: segment.range.va.start,
                alignment: segment.alignment,
            });
        }
        if segment.segment_type == elf::PT_LOAD {
            push_load_ranges(segment, &mut load_file_ranges, &mut load_va_ranges);
            load_segments.push(segment);
        }
    }

    check_non_overlapping("PT_LOAD file", &mut load_file_ranges)?;
    check_non_overlapping("PT_LOAD virtual", &mut load_va_ranges)?;
    Ok(load_segments)
}

fn push_load_ranges(
    segment: &LayoutSegmentWitness,
    file_ranges: &mut Vec<NamedRange>,
    va_ranges: &mut Vec<NamedRange>,
) {
    if !segment.range.file.is_empty() {
        file_ranges.push(NamedRange {
            owner: segment_owner(segment),
            range: segment.range.file,
        });
    }
    if !segment.range.va.is_empty() {
        va_ranges.push(NamedRange {
            owner: segment_owner(segment),
            range: segment.range.va,
        });
    }
}
