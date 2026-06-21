use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::RelocationByteInputs;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyRelocAddressWitness {
    pub relocation_type: u32,
    pub addend: i64,
    pub place: ApplyRelocPlaceWitness,
    pub target: ApplyRelocTargetWitness,
    pub got_base: u64,
    pub shared: bool,
    pub tls: ApplyRelocTlsWitness,
    pub inputs: RelocationByteInputs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyRelocPlaceWitness {
    pub section_va: u64,
    pub reloc_offset: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyRelocTargetWitness {
    LocalSection {
        section_address: Option<u64>,
        symbol_value: u64,
        size: u64,
    },
    LocalAbsolute {
        symbol_value: u64,
        size: u64,
    },
    GlobalDefined {
        virtual_address: u64,
        got_address: u64,
        plt_address: u64,
        size: u64,
    },
    WeakUndefined {
        got_address: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyRelocTlsWitness {
    Absent {
        tls_size: u64,
    },
    SectionRelative {
        section_tls_offset: Option<u64>,
        symbol_value: u64,
        tls_size: u64,
        tls_gd: u64,
        tls_ie: u64,
        tls_desc: u64,
        tls_ldm: u64,
    },
    Imported {
        tls_size: u64,
        tls_gd: u64,
        tls_ie: u64,
        tls_desc: u64,
        tls_ldm: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApplyRelocAddressError {
    #[error("apply_reloc address witness is missing {field}")]
    MissingAddress { field: &'static str },
    #[error("apply_reloc address witness {field} overflows: {base:#x} + {addend:#x}")]
    AddressOverflow {
        field: &'static str,
        base: u64,
        addend: u64,
    },
    #[error("apply_reloc relocation offset {offset:#x} does not fit usize")]
    OffsetOutOfRange { offset: u64 },
    #[error("apply_reloc byte input {field} mismatch: expected {expected:#x}, got {actual:#x}")]
    InputMismatch {
        field: &'static str,
        expected: i128,
        actual: i128,
    },
}
