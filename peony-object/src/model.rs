use object::{SectionIndex, SymbolIndex};
use rustc_hash::FxHashMap;

use crate::{Name, SectionData};

const MISSING_INDEX: usize = usize::MAX;

#[derive(Debug, Clone)]
pub enum IndexLookup {
    Dense(Vec<usize>),
    Sparse(FxHashMap<usize, usize>),
}

impl Default for IndexLookup {
    fn default() -> Self {
        Self::Sparse(FxHashMap::default())
    }
}

impl IndexLookup {
    pub fn with_dense_len(len: usize) -> Self {
        Self::Dense(vec![MISSING_INDEX; len])
    }

    pub fn reserve(&mut self, additional: usize) {
        if let Self::Sparse(map) = self {
            map.reserve(additional);
        }
    }

    pub fn insert(&mut self, raw_index: usize, pos: usize) {
        match self {
            Self::Dense(dense) if raw_index < dense.len() => {
                dense[raw_index] = pos;
            }
            Self::Dense(dense) => {
                let mut sparse = FxHashMap::default();
                sparse.reserve(dense.len().saturating_add(1));
                for (index, &entry) in dense.iter().enumerate() {
                    if entry != MISSING_INDEX {
                        sparse.insert(index, entry);
                    }
                }
                sparse.insert(raw_index, pos);
                *self = Self::Sparse(sparse);
            }
            Self::Sparse(map) => {
                map.insert(raw_index, pos);
            }
        }
    }

    pub fn get(&self, raw_index: &usize) -> Option<&usize> {
        match self {
            Self::Dense(dense) => dense.get(*raw_index).filter(|&&pos| pos != MISSING_INDEX),
            Self::Sparse(map) => map.get(raw_index),
        }
    }

    #[inline]
    pub fn position(&self, raw_index: usize) -> Option<usize> {
        self.get(&raw_index).copied()
    }
}

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
    pub section_map: IndexLookup,
    pub symbol_map: IndexLookup,
    pub comdat_groups: Vec<ComdatGroup>,
}

impl InputObject {
    #[inline]
    pub fn section_pos(&self, raw_index: usize) -> Option<usize> {
        self.section_map.position(raw_index)
    }

    #[inline]
    pub fn section_by_index(&self, raw_index: usize) -> Option<&InputSection> {
        self.sections.get(self.section_pos(raw_index)?)
    }

    #[inline]
    pub fn symbol_pos(&self, raw_index: usize) -> Option<usize> {
        self.symbol_map.position(raw_index)
    }

    #[inline]
    pub fn symbol_by_index(&self, raw_index: usize) -> Option<&InputSymbol> {
        self.symbols.get(self.symbol_pos(raw_index)?)
    }
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
