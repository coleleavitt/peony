mod apply;
mod tls;
mod types;
mod write;

pub use types::{
    RelocationByteError,
    RelocationByteInputs,
    RelocationBytePatch,
    RelocationByteWidthKind,
    X86_64RelocationExpression,
};

pub mod x86_64_reloc {
    pub const R64: u32 = 1;
    pub const PC32: u32 = 2;
    pub const GOT32: u32 = 3;
    pub const PLT32: u32 = 4;
    pub const GOTPCREL: u32 = 9;
    pub const R32: u32 = 10;
    pub const R32S: u32 = 11;
    pub const R16: u32 = 12;
    pub const PC16: u32 = 13;
    pub const R8: u32 = 14;
    pub const PC8: u32 = 15;
    pub const DTPOFF64: u32 = 17;
    pub const TPOFF64: u32 = 18;
    pub const TLSGD: u32 = 19;
    pub const TLSLD: u32 = 20;
    pub const DTPOFF32: u32 = 21;
    pub const GOTTPOFF: u32 = 22;
    pub const TPOFF32: u32 = 23;
    pub const PC64: u32 = 24;
    pub const GOTOFF64: u32 = 25;
    pub const GOTPC32: u32 = 26;
    pub const SIZE32: u32 = 32;
    pub const SIZE64: u32 = 33;
    pub const GOTPC32_TLSDESC: u32 = 34;
    pub const TLSDESC_CALL: u32 = 35;
    pub const GOTPCRELX: u32 = 41;
    pub const REX_GOTPCRELX: u32 = 42;
}

pub fn model_x86_64_relocation_bytes(
    relocation_type: u32,
    inputs: &RelocationByteInputs,
    original_bytes: &[u8],
) -> Result<RelocationBytePatch, RelocationByteError> {
    let expression = x86_64_relocation_expression(relocation_type, inputs);
    let mut produced_bytes = original_bytes.to_vec();
    let (write_offset, width) =
        apply::apply_expression(&mut produced_bytes, relocation_type, inputs, expression)?;
    Ok(RelocationBytePatch {
        expression,
        output_offset: offset_u64(write_offset),
        width,
        original_bytes: original_bytes.to_vec(),
        produced_bytes,
    })
}

pub const fn x86_64_relocation_expression(
    relocation_type: u32,
    inputs: &RelocationByteInputs,
) -> X86_64RelocationExpression {
    use x86_64_reloc::*;
    match relocation_type {
        R64 => X86_64RelocationExpression::Abs64,
        PC64 => X86_64RelocationExpression::Pc64,
        GOTOFF64 => X86_64RelocationExpression::GotOff64,
        SIZE64 => X86_64RelocationExpression::Size64,
        R32 => X86_64RelocationExpression::Abs32,
        R32S => X86_64RelocationExpression::Abs32Signed,
        PC32 => X86_64RelocationExpression::Pc32,
        SIZE32 => X86_64RelocationExpression::Size32,
        PLT32 => X86_64RelocationExpression::Plt32,
        GOTPCREL => X86_64RelocationExpression::GotPcRel,
        GOTPCRELX => X86_64RelocationExpression::GotPcRelx,
        REX_GOTPCRELX => X86_64RelocationExpression::RexGotPcRelx,
        GOT32 => X86_64RelocationExpression::Got32,
        GOTPC32 => X86_64RelocationExpression::GotPc32,
        TPOFF32 => X86_64RelocationExpression::TpOff32,
        TPOFF64 => X86_64RelocationExpression::TpOff64,
        DTPOFF32 if inputs.shared => X86_64RelocationExpression::Dtpoff32Shared,
        DTPOFF32 => X86_64RelocationExpression::Dtpoff32LocalExec,
        DTPOFF64 if inputs.shared => X86_64RelocationExpression::Dtpoff64Shared,
        DTPOFF64 => X86_64RelocationExpression::Dtpoff64LocalExec,
        TLSGD if inputs.shared => X86_64RelocationExpression::TlsGdShared,
        TLSGD if inputs.tls_imported => X86_64RelocationExpression::TlsGdInitialExec,
        TLSGD => X86_64RelocationExpression::TlsGdLocalExec,
        TLSLD if inputs.shared => X86_64RelocationExpression::TlsLdShared,
        TLSLD => X86_64RelocationExpression::TlsLdLocalExec,
        GOTTPOFF if inputs.shared => X86_64RelocationExpression::GotTpOffShared,
        GOTTPOFF => X86_64RelocationExpression::GotTpOffExecutable,
        GOTPC32_TLSDESC if inputs.shared => X86_64RelocationExpression::TlsDescGotPcShared,
        GOTPC32_TLSDESC if inputs.tls_imported => {
            X86_64RelocationExpression::TlsDescGotPcInitialExec
        }
        GOTPC32_TLSDESC => X86_64RelocationExpression::TlsDescGotPcLocalExec,
        TLSDESC_CALL if inputs.shared => X86_64RelocationExpression::TlsDescCallShared,
        TLSDESC_CALL => X86_64RelocationExpression::TlsDescCallLocalExec,
        R16 => X86_64RelocationExpression::Abs16,
        PC16 => X86_64RelocationExpression::Pc16,
        R8 => X86_64RelocationExpression::Abs8,
        PC8 => X86_64RelocationExpression::Pc8,
        _ => X86_64RelocationExpression::UnsupportedNoop,
    }
}

pub(crate) fn reinterpret_i64(value: u64) -> i64 {
    i64::from_ne_bytes(value.to_ne_bytes())
}

pub(crate) fn offset_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub(crate) fn dtp_offset(a: &RelocationByteInputs) -> i64 {
    reinterpret_i64(a.tls).wrapping_add(a.a)
}

pub(crate) fn tp_offset(a: &RelocationByteInputs) -> i64 {
    dtp_offset(a).wrapping_sub(reinterpret_i64(a.tls_size))
}
