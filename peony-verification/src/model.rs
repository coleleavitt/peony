use serde::{Deserialize, Serialize};

use crate::RangeWitness;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SectionRefWitness {
    pub object_id: usize,
    pub section_index: usize,
}

impl SectionRefWitness {
    pub const fn new(object_id: usize, section_index: usize) -> Self {
        Self {
            object_id,
            section_index,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionKindWitness {
    Text,
    ReadOnly,
    Data,
    Bss,
    Debug,
    EhFrame,
    MergeString,
    MergeConst,
    InitArray,
    Tdata,
    Tbss,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionWitness {
    pub owner: SectionRefWitness,
    pub name: Vec<u8>,
    pub kind: SectionKindWitness,
    pub flags: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolBindingWitness {
    Local,
    Global,
    Weak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolStateWitness {
    Undefined,
    Defined {
        object_id: u32,
        section_index: usize,
    },
    Absolute {
        object_id: u32,
    },
    Common {
        size: u64,
        align: u64,
    },
    Import {
        copy_reloc: bool,
        dynsym_index: u32,
        version: Option<Vec<u8>>,
        soname: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolWitness {
    pub name: Vec<u8>,
    pub symbol_id: Option<u32>,
    pub binding: SymbolBindingWitness,
    pub state: SymbolStateWitness,
    pub value: u64,
    pub size: u64,
    pub virtual_address: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolErrorWitness {
    DuplicateStrong {
        name: Vec<u8>,
        first: String,
        second: String,
    },
    Undefined {
        name: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolRefWitness {
    pub symbol_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelocationWriteWitness {
    pub output_offset: u64,
    pub width: u8,
    pub original_bytes: Vec<u8>,
    pub produced_bytes: Option<Vec<u8>>,
    pub relocation_type: u32,
    pub addend: i64,
    pub place: u64,
    pub symbol: Option<SymbolRefWitness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GcRootReasonWitness {
    Entry,
    RetainFlag,
    Export,
    EhFrame,
    GccExceptTable,
    InitFini,
    UserDefined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcRootWitness {
    pub root: SectionRefWitness,
    pub reason: GcRootReasonWitness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GcEdgeReasonWitness {
    Relocation,
    SectionGroup,
    EhFrame,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcEdgeWitness {
    pub from: SectionRefWitness,
    pub to: SectionRefWitness,
    pub reason: GcEdgeReasonWitness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcReachabilityWitness {
    pub roots: Vec<GcRootWitness>,
    pub edges: Vec<GcEdgeWitness>,
    pub rust_live: Vec<SectionRefWitness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionOwnerWitness {
    pub section: SectionRefWitness,
    pub output_offset: u64,
    pub size: u64,
    pub range: crate::RangeBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutWindowWitness {
    pub output_section_name: String,
    pub section_type: u32,
    pub flags: u64,
    pub range: RangeWitness,
    pub alignment: u64,
    pub contributions: Vec<ContributionOwnerWitness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutSegmentWitness {
    pub index: usize,
    pub segment_type: u32,
    pub flags: u32,
    pub range: RangeWitness,
    pub alignment: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutWitness {
    pub image_base: u64,
    pub file_size: u64,
    pub output_sections: Vec<LayoutWindowWitness>,
    pub segments: Vec<LayoutSegmentWitness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IncrementalColorWitnessKind {
    Red,
    Green,
}

impl From<peony_cache::SectionColor> for IncrementalColorWitnessKind {
    fn from(value: peony_cache::SectionColor) -> Self {
        match value {
            peony_cache::SectionColor::Red => Self::Red,
            peony_cache::SectionColor::Green => Self::Green,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncrementalColorWitness {
    pub section_name: String,
    pub file_offset: u64,
    pub virtual_address: u64,
    pub size: u64,
    pub capacity: u64,
    pub color: IncrementalColorWitnessKind,
}
