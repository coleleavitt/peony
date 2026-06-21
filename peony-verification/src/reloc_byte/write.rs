use super::{RelocationByteError, RelocationByteWidthKind, offset_u64};

pub(crate) fn write_u64(
    buf: &mut [u8],
    off: usize,
    value: i64,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let bits = u64::from_ne_bytes(value.to_ne_bytes());
    write_raw(buf, off, &bits.to_le_bytes(), relocation_type)
}

pub(crate) fn write_i32(
    buf: &mut [u8],
    off: usize,
    value: i64,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let bytes = i32_bytes(value, off, relocation_type)?;
    write_raw(buf, off, &bytes, relocation_type)
}

pub(crate) fn write_u32(
    buf: &mut [u8],
    off: usize,
    value: i64,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let bytes = if let Ok(v) = u32::try_from(value) {
        v.to_le_bytes()
    } else if let Ok(v) = i32::try_from(value) {
        v.to_le_bytes()
    } else {
        return Err(overflow(
            relocation_type,
            off,
            value,
            4,
            RelocationByteWidthKind::UnsignedOrSignExtended,
        ));
    };
    write_raw(buf, off, &bytes, relocation_type)
}

pub(crate) fn write_u16(
    buf: &mut [u8],
    off: usize,
    value: i64,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let bytes = if let Ok(v) = u16::try_from(value) {
        v.to_le_bytes()
    } else if let Ok(v) = i16::try_from(value) {
        v.to_le_bytes()
    } else {
        return Err(overflow(
            relocation_type,
            off,
            value,
            2,
            RelocationByteWidthKind::UnsignedOrSignExtended,
        ));
    };
    write_raw(buf, off, &bytes, relocation_type)
}

pub(crate) fn write_u8(
    buf: &mut [u8],
    off: usize,
    value: i64,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let byte = if let Ok(v) = u8::try_from(value) {
        v
    } else if let Ok(v) = i8::try_from(value) {
        v.to_ne_bytes()[0]
    } else {
        return Err(overflow(
            relocation_type,
            off,
            value,
            1,
            RelocationByteWidthKind::UnsignedOrSignExtended,
        ));
    };
    write_raw(buf, off, &[byte], relocation_type)
}

pub(crate) fn write_raw(
    buf: &mut [u8],
    off: usize,
    bytes: &[u8],
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let end = checked_end(off, bytes.len(), buf.len(), relocation_type)?;
    buf[off..end].copy_from_slice(bytes);
    Ok((off, u8::try_from(bytes.len()).unwrap_or(u8::MAX)))
}

pub(crate) fn i32_bytes(
    value: i64,
    off: usize,
    relocation_type: u32,
) -> Result<[u8; 4], RelocationByteError> {
    let Ok(v) = i32::try_from(value) else {
        return Err(overflow(
            relocation_type,
            off,
            value,
            4,
            RelocationByteWidthKind::Signed,
        ));
    };
    Ok(v.to_le_bytes())
}

pub(crate) fn checked_end(
    off: usize,
    width: usize,
    len: usize,
    relocation_type: u32,
) -> Result<usize, RelocationByteError> {
    let Some(end) = off.checked_add(width) else {
        return Err(buffer_too_short(relocation_type, off, width, len));
    };
    if end > len {
        return Err(buffer_too_short(relocation_type, off, width, len));
    }
    Ok(end)
}

fn overflow(
    relocation_type: u32,
    off: usize,
    value: i64,
    width: u8,
    kind: RelocationByteWidthKind,
) -> RelocationByteError {
    RelocationByteError::Overflow {
        relocation_type,
        offset: offset_u64(off),
        value,
        width,
        kind,
    }
}

fn buffer_too_short(
    relocation_type: u32,
    off: usize,
    width: usize,
    len: usize,
) -> RelocationByteError {
    RelocationByteError::BufferTooShort {
        relocation_type,
        offset: offset_u64(off),
        width,
        buffer_len: len,
    }
}
