use helpers::{
    checked_len,
    contribution_end,
    range_fits,
    section_header_span_len,
    synthetic_section_is_written,
};
use peony_layout::{Layout, OutputSection, SecSource};
use peony_object::{InputObject, elf};

use crate::input_work::{collect_input_work_items, usize_to_u64, validate_work_item_ranges};
use crate::{EmitError, Result, SectionWriteFilter};

mod helpers;
mod model;
pub use model::{EmitWriteRange, EmitWriteReport};

pub(crate) struct EmitReportInput<'a> {
    pub(crate) layout: &'a Layout,
    pub(crate) objects: &'a [InputObject],
    pub(crate) filter: SectionWriteFilter<'a>,
    pub(crate) buf_len: usize,
}

pub(crate) fn report_for_filter(input: EmitReportInput<'_>) -> Result<EmitWriteReport> {
    let buf_len =
        u64::try_from(input.buf_len).map_err(|_| EmitError::TooLarge { size: u64::MAX })?;
    let mut builder = ReportBuilder {
        report: EmitWriteReport::default(),
        layout: input.layout,
        objects: input.objects,
        filter: input.filter,
        buf_len,
    };
    builder.record_non_minimal_headers()?;
    builder.record_section_data(input.buf_len)?;
    builder.record_non_minimal_tls_got()?;
    builder.record_minimal_eh_frame_hdr()?;
    builder.record_non_minimal_section_headers()?;
    builder.record_minimal_build_id_descriptor()?;
    Ok(builder.report)
}

struct ReportBuilder<'a> {
    report: EmitWriteReport,
    layout: &'a Layout,
    objects: &'a [InputObject],
    filter: SectionWriteFilter<'a>,
    buf_len: u64,
}

impl ReportBuilder<'_> {
    fn record_non_minimal_headers(&mut self) -> Result<()> {
        if self.filter.is_minimal() {
            return Ok(());
        }
        self.push("elf header", 0, elf::EHDR_SIZE)?;
        let phdr_len = checked_len(self.layout.segments.len(), elf::PHDR_SIZE)?;
        self.push("program headers", self.layout.phoff, phdr_len)
    }
    fn record_section_data(&mut self, buf_len: usize) -> Result<()> {
        for section in &self.layout.output_sections {
            match section.source {
                SecSource::Input => {
                    if self.filter.zeroes_gaps(&section.name) {
                        self.record_input_gaps(section)?;
                    }
                }
                SecSource::Bss => {}
                _ if !self.filter.is_minimal()
                    && synthetic_section_is_written(section, self.layout) =>
                {
                    self.push(
                        format!("synthetic {}", section.name),
                        section.sh_offset,
                        section.sh_size,
                    )?;
                }
                _ => {}
            }
        }
        self.record_input_work(buf_len)
    }
    fn record_input_work(&mut self, buf_len: usize) -> Result<()> {
        let items = collect_input_work_items(self.layout, self.objects, self.filter);
        let accepted = validate_work_item_ranges(&items, buf_len)?;
        for (item_index, item) in items.iter().enumerate() {
            let Some(range) = accepted.range_for_item(item_index) else {
                continue;
            };
            let section_name = String::from_utf8_lossy(item.isec.name.as_bytes());
            self.push(
                format!(
                    "input {}:{} {}",
                    item.obj.path, item.input_section_index, section_name
                ),
                usize_to_u64(range.start()),
                usize_to_u64(range.len()),
            )?;
        }
        Ok(())
    }
    fn record_non_minimal_tls_got(&mut self) -> Result<()> {
        if self.filter.is_minimal() || self.layout.tls_got_writes.is_empty() {
            return Ok(());
        }
        let Some(got) = self
            .layout
            .output_sections
            .iter()
            .find(|section| section.source == SecSource::Got)
        else {
            return Ok(());
        };
        let Some(got_hi) = got.sh_addr.checked_add(got.sh_size) else {
            return Ok(());
        };
        for (slot_index, &(va, _value)) in self.layout.tls_got_writes.iter().enumerate() {
            let Some(write_end) = va.checked_add(8) else {
                continue;
            };
            if va < got.sh_addr || write_end > got_hi {
                continue;
            }
            let Some(file_offset) = got.sh_offset.checked_add(va - got.sh_addr) else {
                continue;
            };
            if file_offset
                .checked_add(8)
                .is_some_and(|end| end <= self.buf_len)
            {
                self.push(format!("tls got slot {slot_index}"), file_offset, 8)?;
            }
        }
        Ok(())
    }
    fn record_minimal_eh_frame_hdr(&mut self) -> Result<()> {
        if !self.filter.is_minimal() {
            return Ok(());
        }
        let Some(hdr) = self
            .layout
            .output_sections
            .iter()
            .find(|section| section.source == SecSource::EhFrameHdr)
        else {
            return Ok(());
        };
        let Some(eh) = self
            .layout
            .output_sections
            .iter()
            .find(|section| section.name == ".eh_frame")
        else {
            return Ok(());
        };
        if !range_fits(eh.sh_offset, eh.sh_size, self.buf_len) {
            return Ok(());
        }
        let planned_len = u64::try_from(self.layout.dyn_blobs.eh_frame_hdr.len())
            .unwrap_or(u64::MAX)
            .min(hdr.sh_size);
        self.push(".eh_frame_hdr rewrite", hdr.sh_offset, planned_len)
    }
    fn record_non_minimal_section_headers(&mut self) -> Result<()> {
        if self.filter.is_minimal() {
            return Ok(());
        }
        let len = section_header_span_len(self.layout)?;
        self.push("section headers", self.layout.shoff, len)
    }
    fn record_minimal_build_id_descriptor(&mut self) -> Result<()> {
        if !self.filter.is_minimal() {
            return Ok(());
        }
        let Some(section) = self
            .layout
            .output_sections
            .iter()
            .find(|section| section.source == SecSource::NoteBuildId)
        else {
            return Ok(());
        };
        let Some(descriptor_offset) = section.sh_offset.checked_add(16) else {
            return self.overflow("build-id descriptor", section.sh_offset, 16);
        };
        if descriptor_offset
            .checked_add(16)
            .is_some_and(|end| end <= self.buf_len)
        {
            self.push("build-id descriptor", descriptor_offset, 16)?;
        }
        Ok(())
    }
    fn record_input_gaps(&mut self, section: &OutputSection) -> Result<()> {
        let section_end = section.sh_offset.saturating_add(section.sh_size);
        let mut cursor = section.sh_offset;
        for contribution in &section.contributions {
            let start = section.sh_offset.saturating_add(contribution.offset);
            if start > cursor {
                self.push(
                    format!("gap {}", section.name),
                    cursor,
                    start.saturating_sub(cursor),
                )?;
            }
            cursor = cursor.max(contribution_end(section.sh_offset, contribution));
        }
        if section_end > cursor {
            self.push(
                format!("gap {}", section.name),
                cursor,
                section_end.saturating_sub(cursor),
            )?;
        }
        Ok(())
    }

    fn push<L>(&mut self, label: L, start: u64, len: u64) -> Result<()>
    where
        L: Into<String>,
    {
        let label = label.into();
        let Some(end) = start.checked_add(len) else {
            return self.overflow(label, start, len);
        };
        if end > self.buf_len {
            return Err(EmitError::WriteRangeOutOfBounds {
                label,
                start,
                len,
                buf_len: self.buf_len,
            });
        }
        self.report.push(EmitWriteRange::new(label, start, len));
        Ok(())
    }

    fn overflow<L>(&self, label: L, start: u64, len: u64) -> Result<()>
    where
        L: Into<String>,
    {
        Err(EmitError::WriteRangeOverflow {
            label: label.into(),
            start,
            len,
        })
    }
}
