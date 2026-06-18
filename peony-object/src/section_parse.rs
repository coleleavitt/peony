use object::read::elf::{ElfFile64, SectionHeader};
use object::{Endianness, Object, ObjectSection, ObjectSymbol, SymbolIndex};

use crate::{ComdatGroup, InputReloc, Name, SectionKind};

pub(super) fn parse_comdat_groups(elf: &ElfFile64<Endianness>) -> Vec<ComdatGroup> {
    const SHT_GROUP: u32 = 17;
    const GRP_COMDAT: u32 = 0x1;
    let endian = elf.endian();
    let mut groups = Vec::new();
    for section in elf.sections() {
        let hdr = section.elf_section_header();
        if hdr.sh_type(endian) != SHT_GROUP {
            continue;
        }
        let Ok(data) = section.data() else {
            continue;
        };
        if data.len() < 4 {
            continue;
        }
        let flags = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if flags & GRP_COMDAT == 0 {
            continue;
        }
        let members = data[4..]
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize)
            .collect();
        let sig_idx = hdr.sh_info(endian) as usize;
        let signature = elf
            .symbol_by_index(SymbolIndex(sig_idx))
            .ok()
            .and_then(|s| s.name_bytes().ok())
            .map(Name::from_slice)
            .unwrap_or_else(Name::empty);
        groups.push(ComdatGroup { signature, members });
    }
    groups
}

pub(super) fn classify_section(name: &[u8], flags: u64) -> SectionKind {
    const SHF_MERGE: u64 = 0x10;
    const SHF_STRINGS: u64 = 0x20;
    const SHF_TLS: u64 = 0x400;

    if flags & SHF_TLS != 0 {
        return if name.starts_with(b".tbss") {
            SectionKind::Tbss
        } else {
            SectionKind::Tdata
        };
    }

    if flags & SHF_MERGE != 0 {
        return if flags & SHF_STRINGS != 0 {
            SectionKind::MergeString
        } else {
            SectionKind::MergeConst
        };
    }

    if name.starts_with(b".text") {
        SectionKind::Text
    } else if name.starts_with(b".rodata") {
        SectionKind::ReadOnly
    } else if name.starts_with(b".data") {
        SectionKind::Data
    } else if name.starts_with(b".bss") {
        SectionKind::Bss
    } else if is_debug_section_name(name) {
        SectionKind::Debug
    } else if name == b".eh_frame" {
        SectionKind::EhFrame
    } else if name.starts_with(b".init_array") || name.starts_with(b".fini_array") {
        SectionKind::InitArray
    } else {
        SectionKind::Other
    }
}

pub(super) fn is_debug_section_name(name: &[u8]) -> bool {
    name == b".debug"
        || name.starts_with(b".debug_")
        || name.starts_with(b".zdebug_")
        || name.starts_with(b".gnu.debuglto_")
}

pub(super) fn normalize_debug_section_name(name: &[u8]) -> Name {
    if let Some(rest) = name.strip_prefix(b".zdebug_") {
        let mut normalized = b".debug_".to_vec();
        normalized.extend_from_slice(rest);
        Name::from(normalized)
    } else {
        Name::from_slice(name)
    }
}

pub(super) fn collect_relocs(
    section: &object::read::elf::ElfSection64<Endianness>,
) -> Vec<InputReloc> {
    let relocs = section.relocations();
    let mut out = Vec::with_capacity(relocs.size_hint().0);
    for (offset, reloc) in relocs {
        let symbol = match reloc.target() {
            object::RelocationTarget::Symbol(s) => s,
            _ => continue,
        };
        let r_type = match reloc.flags() {
            object::RelocationFlags::Elf { r_type } => r_type,
            _ => continue,
        };
        out.push(InputReloc {
            offset,
            r_type,
            symbol,
            addend: reloc.addend(),
        });
    }
    out
}
