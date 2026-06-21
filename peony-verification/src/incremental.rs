use serde::{Deserialize, Serialize};

use crate::{IncrementalColorWitness, IncrementalColorWitnessKind, RangeBounds, WitnessError};

mod error;
pub use error::IncrementalPreservationError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialEmitWriteWitness {
    pub label: String,
    pub range: RangeBounds,
}

impl PartialEmitWriteWitness {
    pub fn from_start_len(label: String, start: u64, len: u64) -> Result<Self, WitnessError> {
        Ok(Self {
            label,
            range: RangeBounds::from_start_len(start, len)?,
        })
    }
}

pub fn partial_emit_writes_from_report(
    report: &peony_emit::EmitWriteReport,
) -> Result<Vec<PartialEmitWriteWitness>, WitnessError> {
    report
        .ranges()
        .iter()
        .map(|range| {
            PartialEmitWriteWitness::from_start_len(
                range.label().to_string(),
                range.file_offset(),
                range.len(),
            )
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialEmitPreservationWitness {
    pub sections: Vec<IncrementalColorWitness>,
    pub writes: Vec<PartialEmitWriteWitness>,
    pub previous_bytes: Vec<u8>,
    pub partial_bytes: Vec<u8>,
    pub full_bytes: Vec<u8>,
}

pub fn check_partial_emit_preservation(
    witness: &PartialEmitPreservationWitness,
) -> Result<(), IncrementalPreservationError> {
    let image_len = checked_image_len(witness)?;
    let image = RangeBounds::from_start_len(0, image_len)?;
    let mut red_ranges = Vec::new();
    let mut green_ranges = Vec::new();

    for section in &witness.sections {
        let range = RangeBounds::from_start_len(section.file_offset, section.size)?;
        if !image.contains(range) {
            return Err(IncrementalPreservationError::SectionOutOfBounds {
                section: section.section_name.clone(),
                section_start: range.start,
                section_end: range.end,
                image_len,
            });
        }
        match section.color {
            IncrementalColorWitnessKind::Red => {
                red_ranges.push(SectionRange::new(&section.section_name, range));
            }
            IncrementalColorWitnessKind::Green => {
                green_ranges.push(SectionRange::new(&section.section_name, range));
            }
        }
    }
    red_ranges.sort_by(|left, right| {
        left.range
            .start
            .cmp(&right.range.start)
            .then_with(|| left.range.end.cmp(&right.range.end))
            .then_with(|| left.section.cmp(right.section))
    });

    for write in &witness.writes {
        if !image.contains(write.range) {
            return Err(IncrementalPreservationError::WriteOutOfBounds {
                write_label: write.label.clone(),
                write_start: write.range.start,
                write_end: write.range.end,
                image_len,
            });
        }
        if let Some(green) = green_ranges
            .iter()
            .find(|green| ranges_overlap(write.range, green.range))
        {
            return Err(IncrementalPreservationError::WriteTouchesGreen {
                write_label: write.label.clone(),
                green_section: green.section.to_string(),
                write_start: write.range.start,
                write_end: write.range.end,
                green_start: green.range.start,
                green_end: green.range.end,
            });
        }
        if !range_covered_by_red(write.range, &red_ranges) {
            return Err(IncrementalPreservationError::WriteOutsideRed {
                write_label: write.label.clone(),
                write_start: write.range.start,
                write_end: write.range.end,
            });
        }
    }

    for green in &green_ranges {
        compare_range(
            green,
            ByteSlices::new(&witness.previous_bytes, &witness.partial_bytes),
            ByteComparison::GreenPreviousPartial,
        )?;
    }
    for red in &red_ranges {
        compare_range(
            red,
            ByteSlices::new(&witness.partial_bytes, &witness.full_bytes),
            ByteComparison::RedPartialFull,
        )?;
    }

    Ok(())
}

fn checked_image_len(
    witness: &PartialEmitPreservationWitness,
) -> Result<u64, IncrementalPreservationError> {
    let previous_len = u64::try_from(witness.previous_bytes.len()).map_err(|_| {
        IncrementalPreservationError::ImageLengthTooLarge {
            len: witness.previous_bytes.len(),
        }
    })?;
    let partial_len = u64::try_from(witness.partial_bytes.len()).map_err(|_| {
        IncrementalPreservationError::ImageLengthTooLarge {
            len: witness.partial_bytes.len(),
        }
    })?;
    let full_len = u64::try_from(witness.full_bytes.len()).map_err(|_| {
        IncrementalPreservationError::ImageLengthTooLarge {
            len: witness.full_bytes.len(),
        }
    })?;
    if previous_len == partial_len && partial_len == full_len {
        return Ok(previous_len);
    }
    Err(IncrementalPreservationError::ImageLengthMismatch {
        previous_len,
        partial_len,
        full_len,
    })
}

#[derive(Debug, Clone, Copy)]
struct SectionRange<'a> {
    section: &'a str,
    range: RangeBounds,
}

impl<'a> SectionRange<'a> {
    const fn new(section: &'a str, range: RangeBounds) -> Self {
        Self { section, range }
    }
}

fn range_covered_by_red(range: RangeBounds, red_ranges: &[SectionRange<'_>]) -> bool {
    if range.is_empty() {
        return true;
    }
    let mut cursor = range.start;
    for red in red_ranges {
        if red.range.end <= cursor {
            continue;
        }
        if red.range.start > cursor {
            return false;
        }
        cursor = cursor.max(red.range.end);
        if cursor >= range.end {
            return true;
        }
    }
    false
}

const fn ranges_overlap(left: RangeBounds, right: RangeBounds) -> bool {
    left.start < right.end && right.start < left.end
}

#[derive(Debug, Clone, Copy)]
enum ByteComparison {
    GreenPreviousPartial,
    RedPartialFull,
}

#[derive(Debug, Clone, Copy)]
struct ByteSlices<'a> {
    left: &'a [u8],
    right: &'a [u8],
}

impl<'a> ByteSlices<'a> {
    const fn new(left: &'a [u8], right: &'a [u8]) -> Self {
        Self { left, right }
    }
}

fn compare_range(
    section: &SectionRange<'_>,
    bytes: ByteSlices<'_>,
    comparison: ByteComparison,
) -> Result<(), IncrementalPreservationError> {
    let mut offset = section.range.start;
    while offset < section.range.end {
        let index = usize::try_from(offset)
            .map_err(|_| IncrementalPreservationError::ByteOffsetTooLarge { offset })?;
        if bytes.left[index] != bytes.right[index] {
            return match comparison {
                ByteComparison::GreenPreviousPartial => {
                    Err(IncrementalPreservationError::GreenByteChanged {
                        section: section.section.to_string(),
                        offset,
                        previous: bytes.left[index],
                        partial: bytes.right[index],
                    })
                }
                ByteComparison::RedPartialFull => {
                    Err(IncrementalPreservationError::RedByteMismatch {
                        section: section.section.to_string(),
                        offset,
                        partial: bytes.left[index],
                        full: bytes.right[index],
                    })
                }
            };
        }
        offset += 1;
    }
    Ok(())
}
