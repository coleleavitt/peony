use peony_object::InputSection;

use super::WorkItem;
use super::errors::WorkRangeError;
use super::footprints::relocation_footprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcceptedWorkItemRange {
    item_index: usize,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcceptedWorkRanges {
    item_ranges: Vec<Option<AcceptedWorkItemRange>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkRange {
    pub(crate) item_index: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl AcceptedWorkItemRange {
    #[cfg(test)]
    pub(crate) const fn item_index(self) -> usize {
        self.item_index
    }

    pub(crate) const fn start(self) -> usize {
        self.start
    }

    #[cfg(test)]
    pub(crate) const fn end(self) -> usize {
        self.end
    }

    pub(crate) const fn len(self) -> usize {
        self.end - self.start
    }

    pub(crate) const fn contains(self, start: usize, end: usize) -> bool {
        self.start <= start && end <= self.end
    }
}

impl AcceptedWorkRanges {
    pub(crate) fn range_for_item(&self, item_index: usize) -> Option<AcceptedWorkItemRange> {
        self.item_ranges.get(item_index).copied().flatten()
    }
}

impl WorkRange {
    pub(crate) fn from_parts(
        item_index: usize,
        file_off: usize,
        file_len: usize,
        buf_len: usize,
    ) -> Result<Option<Self>, WorkRangeError> {
        if file_len == 0 {
            return Ok(None);
        }
        let Some(end) = file_off.checked_add(file_len) else {
            return Err(WorkRangeError::Overflow {
                item_index,
                file_off,
                file_len,
            });
        };
        if end > buf_len {
            return Err(WorkRangeError::OutOfBounds {
                item_index,
                file_off,
                file_len,
                buf_len,
            });
        }
        Ok(Some(Self {
            item_index,
            start: file_off,
            end,
        }))
    }

    const fn accepted(self) -> AcceptedWorkItemRange {
        AcceptedWorkItemRange {
            item_index: self.item_index,
            start: self.start,
            end: self.end,
        }
    }
}

pub(crate) fn validate_work_item_ranges(
    items: &[WorkItem<'_>],
    buf_len: usize,
) -> Result<AcceptedWorkRanges, WorkRangeError> {
    let mut item_ranges = vec![None; items.len()];
    let mut ranges = Vec::with_capacity(items.len());
    for (item_index, item) in items.iter().enumerate() {
        if let Some(range) =
            WorkRange::from_parts(item_index, item.file_off, item.file_len, buf_len)?
        {
            validate_item_containment(item_index, item.isec, range)?;
            item_ranges[item_index] = Some(range.accepted());
            ranges.push(range);
        }
    }
    validate_work_ranges(&mut ranges)?;
    Ok(AcceptedWorkRanges { item_ranges })
}

pub(crate) fn validate_work_ranges(
    ranges: &mut [WorkRange],
) -> Result<AcceptedWorkRanges, WorkRangeError> {
    ranges.sort_unstable_by_key(|range| (range.start, range.end, range.item_index));
    let mut previous: Option<WorkRange> = None;
    for current in ranges.iter().copied() {
        if let Some(first) = previous
            && current.start < first.end
        {
            return Err(WorkRangeError::Overlap {
                first_index: first.item_index,
                first_start: first.start,
                first_end: first.end,
                second_index: current.item_index,
                second_start: current.start,
                second_end: current.end,
            });
        }
        previous = Some(current);
    }
    let range_count = ranges
        .iter()
        .map(|range| range.item_index)
        .max()
        .map_or(0, |max_index| max_index.saturating_add(1));
    let mut item_ranges = vec![None; range_count];
    for range in ranges {
        item_ranges[range.item_index] = Some(range.accepted());
    }
    Ok(AcceptedWorkRanges { item_ranges })
}

fn validate_item_containment(
    item_index: usize,
    isec: &InputSection,
    accepted: WorkRange,
) -> Result<(), WorkRangeError> {
    let accepted_len = accepted.end - accepted.start;
    if isec.data.len() > accepted_len {
        return Err(WorkRangeError::OutOfBounds {
            item_index,
            file_off: accepted.start,
            file_len: isec.data.len(),
            buf_len: accepted_len,
        });
    }
    for (reloc_index, reloc) in isec.relocs.iter().enumerate() {
        let Some(footprint) = relocation_footprint(item_index, reloc_index, reloc)? else {
            continue;
        };
        let Some(footprint_end) = footprint.start.checked_add(footprint.len) else {
            return Err(WorkRangeError::RelocationOutOfBounds {
                item_index,
                reloc_index,
                r_type: reloc.r_type,
                offset: reloc.offset,
                width: footprint.len,
                section_len: isec.data.len(),
            });
        };
        if footprint_end > isec.data.len() {
            return Err(WorkRangeError::RelocationOutOfBounds {
                item_index,
                reloc_index,
                r_type: reloc.r_type,
                offset: reloc.offset,
                width: footprint.len,
                section_len: isec.data.len(),
            });
        }
    }
    Ok(())
}
