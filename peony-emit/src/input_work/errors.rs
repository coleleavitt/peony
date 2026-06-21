use crate::EmitError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkRangeError {
    Overflow {
        item_index: usize,
        file_off: usize,
        file_len: usize,
    },
    OutOfBounds {
        item_index: usize,
        file_off: usize,
        file_len: usize,
        buf_len: usize,
    },
    Overlap {
        first_index: usize,
        first_start: usize,
        first_end: usize,
        second_index: usize,
        second_start: usize,
        second_end: usize,
    },
    RelocationBeforeSection {
        item_index: usize,
        reloc_index: usize,
        r_type: u32,
        offset: u64,
        prefix_len: usize,
    },
    RelocationOutOfBounds {
        item_index: usize,
        reloc_index: usize,
        r_type: u32,
        offset: u64,
        width: usize,
        section_len: usize,
    },
}

impl From<WorkRangeError> for EmitError {
    fn from(value: WorkRangeError) -> Self {
        match value {
            WorkRangeError::Overflow {
                item_index,
                file_off,
                file_len,
            } => EmitError::InputWriteRangeOverflow {
                item_index,
                file_off,
                file_len,
            },
            WorkRangeError::OutOfBounds {
                item_index,
                file_off,
                file_len,
                buf_len,
            } => EmitError::InputWriteRangeOutOfBounds {
                item_index,
                file_off,
                file_len,
                buf_len,
            },
            WorkRangeError::RelocationOutOfBounds {
                item_index,
                offset,
                width,
                section_len,
                ..
            } => EmitError::InputWriteRangeOutOfBounds {
                item_index,
                file_off: usize::try_from(offset).unwrap_or(usize::MAX),
                file_len: width,
                buf_len: section_len,
            },
            WorkRangeError::RelocationBeforeSection {
                item_index,
                offset,
                prefix_len,
                ..
            } => EmitError::InputWriteRangeOutOfBounds {
                item_index,
                file_off: usize::try_from(offset).unwrap_or(usize::MAX),
                file_len: prefix_len,
                buf_len: usize::try_from(offset).unwrap_or(usize::MAX),
            },
            WorkRangeError::Overlap {
                first_index,
                first_start,
                first_end,
                second_index,
                second_start,
                second_end,
            } => EmitError::InputWriteRangeOverlap {
                first_index,
                first_start,
                first_end,
                second_index,
                second_start,
                second_end,
            },
        }
    }
}
