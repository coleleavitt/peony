use peony_layout::{Layout, OutputSection, SecSource, SectionContribution};
use peony_object::elf;

use crate::{EmitError, Result};

pub(crate) fn synthetic_section_is_written(section: &OutputSection, layout: &Layout) -> bool {
    match section.source {
        SecSource::Input | SecSource::Bss => false,
        SecSource::RelaEmit(index) => layout.emit_relocs.get(index).is_some(),
        _ => true,
    }
}

pub(crate) fn checked_len(count: usize, size: u64) -> Result<u64> {
    let count = u64::try_from(count).map_err(|_| EmitError::TooLarge { size: u64::MAX })?;
    count
        .checked_mul(size)
        .ok_or(EmitError::WriteRangeOverflow {
            label: "emit table".to_string(),
            start: 0,
            len: size,
        })
}

pub(crate) fn section_header_span_len(layout: &Layout) -> Result<u64> {
    let shdr_count = layout
        .output_sections
        .iter()
        .map(|section| u64::from(section.shndx))
        .max()
        .map_or(1, |max_index| max_index.saturating_add(1));
    shdr_count
        .checked_mul(elf::SHDR_SIZE)
        .ok_or(EmitError::WriteRangeOverflow {
            label: "section headers".to_string(),
            start: layout.shoff,
            len: elf::SHDR_SIZE,
        })
}

pub(crate) fn contribution_end(section_offset: u64, contribution: &SectionContribution) -> u64 {
    section_offset
        .saturating_add(contribution.offset)
        .saturating_add(contribution.size)
}

pub(crate) const fn range_fits(start: u64, len: u64, buf_len: u64) -> bool {
    match start.checked_add(len) {
        Some(end) => end <= buf_len,
        None => false,
    }
}
