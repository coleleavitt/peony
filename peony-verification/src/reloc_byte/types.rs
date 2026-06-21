use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelocationByteInputs {
    pub s: u64,
    pub a: i64,
    pub p: u64,
    pub g: u64,
    pub l: u64,
    pub z: u64,
    pub got_base: u64,
    pub tls: u64,
    pub tls_size: u64,
    pub offset: usize,
    pub shared: bool,
    pub tls_gd: u64,
    pub tls_ie: u64,
    pub tls_desc: u64,
    pub tls_ldm: u64,
    pub tls_imported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelocationBytePatch {
    pub expression: X86_64RelocationExpression,
    pub output_offset: u64,
    pub width: u8,
    pub original_bytes: Vec<u8>,
    pub produced_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum X86_64RelocationExpression {
    Abs64,
    Pc64,
    GotOff64,
    Size64,
    Abs32,
    Abs32Signed,
    Pc32,
    Size32,
    Plt32,
    GotPcRel,
    GotPcRelx,
    RexGotPcRelx,
    Got32,
    GotPc32,
    TpOff32,
    TpOff64,
    Dtpoff32Shared,
    Dtpoff32LocalExec,
    Dtpoff64Shared,
    Dtpoff64LocalExec,
    TlsGdShared,
    TlsGdInitialExec,
    TlsGdLocalExec,
    TlsLdShared,
    TlsLdLocalExec,
    GotTpOffShared,
    GotTpOffExecutable,
    TlsDescGotPcShared,
    TlsDescGotPcInitialExec,
    TlsDescGotPcLocalExec,
    TlsDescCallShared,
    TlsDescCallLocalExec,
    Abs16,
    Pc16,
    Abs8,
    Pc8,
    UnsupportedNoop,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RelocationByteError {
    #[error(
        "relocation type {relocation_type} value {value} overflows {width}-byte {kind:?} field at offset {offset:#x}"
    )]
    Overflow {
        relocation_type: u32,
        offset: u64,
        value: i64,
        width: u8,
        kind: RelocationByteWidthKind,
    },
    #[error(
        "relocation type {relocation_type} writes {width} bytes at offset {offset:#x}, beyond buffer length {buffer_len}"
    )]
    BufferTooShort {
        relocation_type: u32,
        offset: u64,
        width: usize,
        buffer_len: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationByteWidthKind {
    Signed,
    UnsignedOrSignExtended,
}
