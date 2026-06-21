use super::write::{write_i32, write_raw, write_u8, write_u16, write_u32, write_u64};
use super::{
    RelocationByteError,
    RelocationByteInputs,
    X86_64RelocationExpression,
    dtp_offset,
    reinterpret_i64,
    tls,
    tp_offset,
};

pub(crate) fn apply_expression(
    buf: &mut [u8],
    relocation_type: u32,
    a: &RelocationByteInputs,
    expression: X86_64RelocationExpression,
) -> Result<(usize, u8), RelocationByteError> {
    use X86_64RelocationExpression::*;
    let s = reinterpret_i64(a.s);
    let p = reinterpret_i64(a.p);
    match expression {
        Abs64 => write_u64(buf, a.offset, s.wrapping_add(a.a), relocation_type),
        Pc64 => write_u64(
            buf,
            a.offset,
            s.wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
        GotOff64 => write_u64(
            buf,
            a.offset,
            s.wrapping_add(a.a)
                .wrapping_sub(reinterpret_i64(a.got_base)),
            relocation_type,
        ),
        Size64 => write_u64(
            buf,
            a.offset,
            reinterpret_i64(a.z).wrapping_add(a.a),
            relocation_type,
        ),
        Abs32 => write_u32(buf, a.offset, s.wrapping_add(a.a), relocation_type),
        Abs32Signed => write_i32(buf, a.offset, s.wrapping_add(a.a), relocation_type),
        Pc32 => write_i32(
            buf,
            a.offset,
            s.wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
        Size32 => write_u32(
            buf,
            a.offset,
            reinterpret_i64(a.z).wrapping_add(a.a),
            relocation_type,
        ),
        Plt32 => {
            let target = if a.l != 0 { reinterpret_i64(a.l) } else { s };
            write_i32(
                buf,
                a.offset,
                target.wrapping_add(a.a).wrapping_sub(p),
                relocation_type,
            )
        }
        GotPcRel | GotPcRelx | RexGotPcRelx => write_i32(
            buf,
            a.offset,
            reinterpret_i64(a.g).wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
        Got32 => write_u32(
            buf,
            a.offset,
            reinterpret_i64(a.g)
                .wrapping_sub(reinterpret_i64(a.got_base))
                .wrapping_add(a.a),
            relocation_type,
        ),
        GotPc32 => write_i32(
            buf,
            a.offset,
            reinterpret_i64(a.got_base)
                .wrapping_add(a.a)
                .wrapping_sub(p),
            relocation_type,
        ),
        TpOff32 => write_i32(buf, a.offset, tp_offset(a), relocation_type),
        TpOff64 => write_u64(buf, a.offset, tp_offset(a), relocation_type),
        Dtpoff32Shared => write_i32(buf, a.offset, dtp_offset(a), relocation_type),
        Dtpoff32LocalExec => write_i32(buf, a.offset, tp_offset(a), relocation_type),
        Dtpoff64Shared => write_u64(buf, a.offset, dtp_offset(a), relocation_type),
        Dtpoff64LocalExec => write_u64(buf, a.offset, tp_offset(a), relocation_type),
        TlsGdShared => write_i32(
            buf,
            a.offset,
            reinterpret_i64(a.tls_gd).wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
        TlsGdInitialExec => tls::rewrite_tlsgd_ie(buf, a, p, relocation_type),
        TlsGdLocalExec => tls::rewrite_tlsgd_le(buf, a, relocation_type),
        TlsLdShared => write_i32(
            buf,
            a.offset,
            reinterpret_i64(a.tls_ldm).wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
        TlsLdLocalExec => tls::rewrite_tlsld_le(buf, a, relocation_type),
        GotTpOffShared | GotTpOffExecutable => write_i32(
            buf,
            a.offset,
            reinterpret_i64(a.tls_ie).wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
        TlsDescGotPcShared => write_i32(
            buf,
            a.offset,
            reinterpret_i64(a.tls_desc)
                .wrapping_add(a.a)
                .wrapping_sub(p),
            relocation_type,
        ),
        TlsDescGotPcInitialExec => tls::rewrite_tlsdesc_ie(buf, a, p, relocation_type),
        TlsDescGotPcLocalExec => tls::rewrite_tlsdesc_le(buf, a, relocation_type),
        TlsDescCallShared | UnsupportedNoop => Ok((a.offset, 0)),
        TlsDescCallLocalExec => write_raw(buf, a.offset, &[0x66, 0x90], relocation_type),
        Abs16 => write_u16(buf, a.offset, s.wrapping_add(a.a), relocation_type),
        Pc16 => write_u16(
            buf,
            a.offset,
            s.wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
        Abs8 => write_u8(buf, a.offset, s.wrapping_add(a.a), relocation_type),
        Pc8 => write_u8(
            buf,
            a.offset,
            s.wrapping_add(a.a).wrapping_sub(p),
            relocation_type,
        ),
    }
}
