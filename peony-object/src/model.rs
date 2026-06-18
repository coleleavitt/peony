use object::{SectionIndex, SymbolIndex};
use rustc_hash::FxHashMap;

use crate::{Name, SectionData};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
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

#[derive(Debug, Clone)]
pub struct InputSection {
    pub index: SectionIndex,
    pub name: Name,
    pub kind: SectionKind,
    pub sh_type: u32,
    pub data: SectionData,
    pub align: u64,
    pub size: u64,
    pub flags: u64,
    pub relocs: Vec<InputReloc>,
}

#[derive(Debug, Clone)]
pub struct InputReloc {
    pub offset: u64,
    pub r_type: u32,
    pub symbol: SymbolIndex,
    pub addend: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Local,
    Global,
    Weak,
}

#[derive(Debug, Clone)]
pub struct InputSymbol {
    pub index: SymbolIndex,
    pub name: Name,
    pub binding: Binding,
    pub is_undefined: bool,
    pub is_common: bool,
    pub is_ifunc: bool,
    pub st_type: u8,
    pub visibility: u8,
    pub section: Option<SectionIndex>,
    pub value: u64,
    pub size: u64,
}

pub struct InputObject {
    pub path: String,
    pub sections: Vec<InputSection>,
    pub symbols: Vec<InputSymbol>,
    pub section_map: FxHashMap<usize, usize>,
    pub symbol_map: FxHashMap<usize, usize>,
    pub comdat_groups: Vec<ComdatGroup>,
}

#[derive(Debug, Clone)]
pub struct ComdatGroup {
    pub signature: Name,
    pub members: Vec<usize>,
}

impl std::fmt::Debug for InputObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputObject")
            .field("path", &self.path)
            .field("sections", &self.sections.len())
            .field("symbols", &self.symbols.len())
            .finish()
    }
}
