use peony_layout::{Layout, SecSource};
use peony_object::{InputObject, InputSection};
use peony_prof::TraceField;

use crate::SectionWriteFilter;

#[derive(Clone, Copy)]
pub(crate) struct WorkItem<'a> {
    pub(crate) file_off: usize,
    pub(crate) file_len: usize,
    pub(crate) section_va: u64,
    pub(crate) obj: &'a InputObject,
    pub(crate) isec: &'a InputSection,
    pub(crate) obj_id: usize,
    pub(crate) input_section_index: usize,
    pub(crate) reloc_count: usize,
}

pub(crate) struct WorkSummary {
    sections: u64,
    bytes: u64,
    relocs: u64,
    file_start: u64,
    file_len: u64,
    va_start: u64,
    va_len: u64,
}

impl WorkSummary {
    pub(crate) fn from_items(items: &[WorkItem<'_>]) -> Option<Self> {
        let first = items.first()?;
        let mut file_start = usize_to_u64(first.file_off);
        let mut file_end = usize_to_u64(first.file_off.saturating_add(first.file_len));
        let mut va_start = first.section_va;
        let mut va_end = first
            .section_va
            .saturating_add(usize_to_u64(first.file_len));
        let mut bytes = 0u64;
        let mut relocs = 0u64;
        for item in items {
            let item_file_start = usize_to_u64(item.file_off);
            let item_file_end = usize_to_u64(item.file_off.saturating_add(item.file_len));
            let item_va_end = item.section_va.saturating_add(usize_to_u64(item.file_len));
            file_start = file_start.min(item_file_start);
            file_end = file_end.max(item_file_end);
            va_start = va_start.min(item.section_va);
            va_end = va_end.max(item_va_end);
            bytes = bytes.saturating_add(usize_to_u64(item.file_len));
            relocs = relocs.saturating_add(usize_to_u64(item.reloc_count));
        }
        Some(Self {
            sections: usize_to_u64(items.len()),
            bytes,
            relocs,
            file_start,
            file_len: file_end.saturating_sub(file_start),
            va_start,
            va_len: va_end.saturating_sub(va_start),
        })
    }

    pub(crate) fn trace_fields(&self) -> [TraceField; 5] {
        [
            TraceField::count("sections", self.sections),
            TraceField::bytes("bytes", self.bytes),
            TraceField::count("relocs", self.relocs),
            TraceField::byte_range("file", self.file_start, self.file_len),
            TraceField::addr_range("va", self.va_start, self.va_len),
        ]
    }
}

pub(crate) fn collect_input_work_items<'a>(
    layout: &Layout,
    objects: &'a [InputObject],
    filter: SectionWriteFilter<'_>,
) -> Vec<WorkItem<'a>> {
    layout
        .output_sections
        .iter()
        .filter(|sec| sec.source == SecSource::Input && filter.collects_output_section(&sec.name))
        .flat_map(|sec| {
            sec.contributions
                .iter()
                .filter(|c| filter.writes_contribution(c.object_id))
                .filter_map(move |c| {
                    let obj = objects.get(c.object_id)?;
                    let isec = obj.sections.get(c.section_pos)?;
                    let file_off = usize::try_from(sec.sh_offset.saturating_add(c.offset)).ok()?;
                    Some(WorkItem {
                        file_off,
                        file_len: isec.data.len(),
                        section_va: sec.sh_addr.saturating_add(c.offset),
                        obj,
                        isec,
                        obj_id: c.object_id,
                        input_section_index: c.section_index,
                        reloc_count: isec.relocs.len(),
                    })
                })
        })
        .collect()
}

pub(crate) fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
