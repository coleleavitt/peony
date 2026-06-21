use std::collections::BTreeSet;

use peony_object::{InputArena, InputObject};

use crate::SectionRefWitness;

const SHT_LLVM_ADDRSIG: u32 = 0x6fff_4c03;
const R_X86_64_NONE: u32 = 0;
const R_X86_64_PC32: u32 = 2;
const R_X86_64_PLT32: u32 = 4;

#[derive(Default)]
pub(super) struct AddressTaint {
    pub(super) by_name: BTreeSet<Vec<u8>>,
    pub(super) by_section: BTreeSet<SectionRefWitness>,
    pub(super) objects_with_addrsig: BTreeSet<usize>,
    pub(super) addrsig_symbols: BTreeSet<(usize, usize)>,
}

impl AddressTaint {
    pub(super) fn extract(arena: &InputArena, objects: &[InputObject]) -> Self {
        let mut taint = Self::default();
        for (object_id, object) in objects.iter().enumerate() {
            for section in &object.sections {
                for reloc in &section.relocs {
                    taint.record_relocation(object, object_id, reloc);
                }
            }
            taint.record_addrsig(arena, object, object_id);
        }
        taint
    }

    fn record_relocation(
        &mut self,
        object: &InputObject,
        object_id: usize,
        reloc: &peony_object::InputReloc,
    ) {
        if !reloc_takes_address(reloc.r_type) {
            return;
        }
        let Some(symbol) = object.symbol_by_index(reloc.symbol.0) else {
            return;
        };
        if symbol.name.is_empty() {
            if let Some(section) = symbol.section {
                self.by_section
                    .insert(SectionRefWitness::new(object_id, section.0));
            }
            return;
        }
        self.by_name.insert(symbol.name.as_bytes().to_vec());
    }

    fn record_addrsig(&mut self, arena: &InputArena, object: &InputObject, object_id: usize) {
        let Some(section) = object
            .sections
            .iter()
            .find(|section| section.sh_type == SHT_LLVM_ADDRSIG)
        else {
            return;
        };
        let data = arena.bytes(section.data);
        let mut pos = 0usize;
        let mut addrsig_symbols = Vec::new();
        while pos < data.len() {
            let Some(symbol_index) = read_uleb128(data, &mut pos) else {
                return;
            };
            let Ok(raw_index) = usize::try_from(symbol_index) else {
                return;
            };
            let Some(symbol_pos) = object.symbol_pos(raw_index) else {
                return;
            };
            addrsig_symbols.push((object_id, symbol_pos));
        }
        self.objects_with_addrsig.insert(object_id);
        self.addrsig_symbols.extend(addrsig_symbols);
    }
}

const fn reloc_takes_address(relocation_type: u32) -> bool {
    !matches!(
        relocation_type,
        R_X86_64_NONE | R_X86_64_PC32 | R_X86_64_PLT32
    )
}

fn read_uleb128(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}
