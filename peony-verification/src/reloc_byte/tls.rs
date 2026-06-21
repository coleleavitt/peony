use super::write::{checked_end, i32_bytes};
use super::{RelocationByteError, RelocationByteInputs, reinterpret_i64, tp_offset};

pub(crate) fn rewrite_tlsgd_ie(
    buf: &mut [u8],
    a: &RelocationByteInputs,
    p: i64,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let start = a.offset.wrapping_sub(4);
    let end = checked_end(start, 16, buf.len(), relocation_type)?;
    if buf[start..start + 3] == [0x66, 0x48, 0x8d] {
        let disp = reinterpret_i64(a.tls_ie)
            .wrapping_add(a.a)
            .wrapping_sub(p)
            .wrapping_sub(8);
        let bytes = i32_bytes(disp, a.offset, relocation_type)?;
        buf[start..end].copy_from_slice(&[
            0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0x48, 0x03, 0x05, 0, 0, 0, 0,
        ]);
        buf[start + 12..start + 16].copy_from_slice(&bytes);
    }
    Ok((start, 16))
}

pub(crate) fn rewrite_tlsgd_le(
    buf: &mut [u8],
    a: &RelocationByteInputs,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let start = a.offset.wrapping_sub(4);
    let end = checked_end(start, 16, buf.len(), relocation_type)?;
    if buf[start..start + 3] == [0x66, 0x48, 0x8d] {
        let bytes = i32_bytes(tp_offset(a).wrapping_add(4), a.offset, relocation_type)?;
        buf[start..end].copy_from_slice(&[
            0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0x48, 0x8d, 0x80, 0, 0, 0, 0,
        ]);
        buf[start + 12..start + 16].copy_from_slice(&bytes);
    }
    Ok((start, 16))
}

pub(crate) fn rewrite_tlsld_le(
    buf: &mut [u8],
    a: &RelocationByteInputs,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let start = a.offset.wrapping_sub(3);
    let end = checked_end(start, 12, buf.len(), relocation_type)?;
    buf[start..end].copy_from_slice(&[0x66, 0x66, 0x66, 0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    Ok((start, 12))
}

pub(crate) fn rewrite_tlsdesc_ie(
    buf: &mut [u8],
    a: &RelocationByteInputs,
    p: i64,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let start = a.offset.wrapping_sub(3);
    checked_end(start, 7, buf.len(), relocation_type)?;
    if buf[start..start + 2] == [0x48, 0x8d] {
        let disp = reinterpret_i64(a.tls_ie).wrapping_add(a.a).wrapping_sub(p);
        let bytes = i32_bytes(disp, a.offset, relocation_type)?;
        buf[start + 1] = 0x8b;
        buf[a.offset..a.offset + 4].copy_from_slice(&bytes);
    }
    Ok((start, 7))
}

pub(crate) fn rewrite_tlsdesc_le(
    buf: &mut [u8],
    a: &RelocationByteInputs,
    relocation_type: u32,
) -> Result<(usize, u8), RelocationByteError> {
    let start = a.offset.wrapping_sub(3);
    let end = checked_end(start, 7, buf.len(), relocation_type)?;
    if buf[start..start + 2] == [0x48, 0x8d] {
        let bytes = i32_bytes(tp_offset(a).wrapping_add(4), a.offset, relocation_type)?;
        buf[start..end].copy_from_slice(&[0x48, 0xc7, 0xc0, 0, 0, 0, 0]);
        buf[a.offset..a.offset + 4].copy_from_slice(&bytes);
    }
    Ok((start, 7))
}
