use peony_object::InputReloc;
use peony_reloc::r_x86_64;

use super::errors::WorkRangeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RelocationFootprint {
    pub(crate) start: usize,
    pub(crate) len: usize,
}

pub(crate) fn relocation_footprint(
    item_index: usize,
    reloc_index: usize,
    reloc: &InputReloc,
) -> Result<Option<RelocationFootprint>, WorkRangeError> {
    use r_x86_64::*;
    let (prefix_len, len) = match reloc.r_type {
        NONE => return Ok(None),
        R64 | PC64 | GOTOFF64 | SIZE64 | TPOFF64 | DTPOFF64 => (0, 8),
        R32 | R32S | PC32 | SIZE32 | PLT32 | GOTPCREL | GOTPCRELX | REX_GOTPCRELX | GOT32
        | GOTPC32 | TPOFF32 | DTPOFF32 | GOTTPOFF => (0, 4),
        R16 | PC16 => (0, 2),
        R8 | PC8 => (0, 1),
        TLSGD => (4, 16),
        TLSLD => (3, 12),
        GOTPC32_TLSDESC => (3, 7),
        TLSDESC_CALL => (0, 2),
        _ => return Ok(None),
    };
    let offset = usize::try_from(reloc.offset).map_err(|_| WorkRangeError::OutOfBounds {
        item_index,
        file_off: usize::MAX,
        file_len: len,
        buf_len: usize::MAX,
    })?;
    let Some(start) = offset.checked_sub(prefix_len) else {
        return Err(WorkRangeError::RelocationBeforeSection {
            item_index,
            reloc_index,
            r_type: reloc.r_type,
            offset: reloc.offset,
            prefix_len,
        });
    };
    Ok(Some(RelocationFootprint { start, len }))
}
