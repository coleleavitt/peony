use memmap2::Mmap;
use rustc_hash::FxHashSet;

use crate::{InputObject, Name};

#[derive(Default)]
pub struct InputArena {
    pub(crate) mmaps: Vec<Mmap>,
    pub(crate) owned: Vec<Vec<u8>>,
    names: FxHashSet<Name>,
}

impl InputArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_mmap(&mut self, m: Mmap) -> u32 {
        let id = self.mmaps.len() as u32;
        self.mmaps.push(m);
        id
    }

    pub fn push_owned(&mut self, v: Vec<u8>) -> u32 {
        let id = self.owned.len() as u32;
        self.owned.push(v);
        id
    }

    pub fn merge_parsed_owned(&mut self, obj: &mut InputObject, local_owned: Vec<Vec<u8>>) {
        let base = self.owned.len() as u32;
        if !local_owned.is_empty() {
            for sec in &mut obj.sections {
                if let DataSrc::Owned(i) = sec.data.src {
                    sec.data.src = DataSrc::Owned(base + i);
                }
            }
            self.owned.extend(local_owned);
        }
    }

    pub fn intern_name(&mut self, bytes: &[u8]) -> Name {
        if let Some(name) = self.names.get(bytes) {
            return name.clone();
        }
        let name = Name::from_slice(bytes);
        self.names.insert(name.clone());
        name
    }

    pub fn intern_object_names(&mut self, obj: &mut InputObject) {
        for section in &mut obj.sections {
            section.name = self.intern_name(section.name.as_bytes());
        }
        for symbol in &mut obj.symbols {
            if symbol.name.is_empty() || symbol.st_type == crate::elf::STT_SECTION {
                symbol.name = self.intern_name(symbol.name.as_bytes());
            }
        }
        for group in &mut obj.comdat_groups {
            group.signature = self.intern_name(group.signature.as_bytes());
        }
    }

    pub fn interned_name_count(&self) -> usize {
        self.names.len()
    }

    pub fn interned_name_bytes(&self) -> u64 {
        self.names.iter().map(|name| name.len() as u64).sum()
    }

    pub fn intern_bytes(&mut self, bytes: &[u8]) -> SectionData {
        assert!(
            bytes.len() <= u32::MAX as usize,
            "section too large for u32 len"
        );
        let len = bytes.len() as u32;
        let id = self.push_owned(bytes.to_vec());
        SectionData {
            src: DataSrc::Owned(id),
            off: 0,
            len,
        }
    }

    #[inline]
    pub fn mmap_bytes(&self, file_id: u32) -> &[u8] {
        &self.mmaps[file_id as usize]
    }

    #[inline]
    pub fn owned_bytes(&self, owned_id: u32) -> &[u8] {
        &self.owned[owned_id as usize]
    }

    #[inline]
    pub fn bytes(&self, d: SectionData) -> &[u8] {
        let base: &[u8] = match d.src {
            DataSrc::Mmap(i) => &self.mmaps[i as usize],
            DataSrc::Owned(i) => &self.owned[i as usize],
        };
        &base[d.off as usize..d.off as usize + d.len as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_name_reuses_backing_storage() {
        let mut arena = InputArena::new();
        let first = arena.intern_name(b"same");
        let second = arena.intern_name(b"same");
        let third = arena.intern_name(b"different");

        assert!(first.ptr_eq(&second));
        assert!(!first.ptr_eq(&third));
        assert_eq!(arena.interned_name_count(), 2);
        assert_eq!(arena.interned_name_bytes(), 13);
    }

    #[test]
    fn merge_parsed_owned_skips_interning_when_there_is_no_owned_data() {
        let mut arena = InputArena::new();
        let name = Name::from_slice(b".text.fast");
        let mut obj = InputObject {
            path: "obj.o".to_string(),
            sections: vec![crate::InputSection {
                index: object::SectionIndex(1),
                name: name.clone(),
                kind: crate::SectionKind::Text,
                sh_type: crate::elf::SHT_PROGBITS,
                data: SectionData::EMPTY,
                align: 1,
                size: 1,
                flags: crate::elf::SHF_ALLOC | crate::elf::SHF_EXECINSTR,
                relocs: Vec::new(),
            }],
            symbols: Vec::new(),
            section_map: crate::IndexLookup::default(),
            symbol_map: crate::IndexLookup::default(),
            comdat_groups: Vec::new(),
        };

        arena.merge_parsed_owned(&mut obj, Vec::new());

        assert_eq!(arena.interned_name_count(), 0);
        assert!(obj.sections[0].name.ptr_eq(&name));
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SectionData {
    pub src: DataSrc,
    pub off: u32,
    pub len: u32,
}

impl SectionData {
    pub const EMPTY: SectionData = SectionData {
        src: DataSrc::Mmap(0),
        off: 0,
        len: 0,
    };

    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DataSrc {
    Mmap(u32),
    Owned(u32),
}
