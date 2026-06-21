use peony_object::{
    Binding,
    IndexLookup,
    InputArena,
    InputObject,
    InputReloc,
    InputSection,
    InputSymbol,
    Name,
    SectionData,
    SectionIndex,
    SectionKind,
    SymbolIndex,
    elf,
};

const SHT_LLVM_ADDRSIG: u32 = 0x6fff_4c03;

pub(crate) fn section(
    arena: &mut InputArena,
    index: usize,
    name: &[u8],
    kind: SectionKind,
    flags: u64,
    data: &[u8],
    relocs: Vec<InputReloc>,
) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(name),
        kind,
        sh_type: if kind == SectionKind::Bss {
            elf::SHT_NOBITS
        } else {
            elf::SHT_PROGBITS
        },
        data: if data.is_empty() {
            SectionData::EMPTY
        } else {
            arena.intern_bytes(data)
        },
        align: 1,
        size: data.len().max(1) as u64,
        flags,
        relocs,
    }
}

pub(crate) fn symbol(
    index: usize,
    name: &[u8],
    binding: Binding,
    section: usize,
    st_type: u8,
    visibility: u8,
) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(index),
        name: Name::from_slice(name),
        binding,
        is_undefined: false,
        is_common: false,
        is_ifunc: false,
        st_type,
        visibility,
        section: Some(SectionIndex(section)),
        value: 0,
        size: 1,
    }
}

pub(crate) fn object_with_addrsig(
    arena: &mut InputArena,
    path: &str,
    mut sections: Vec<InputSection>,
    symbols: Vec<InputSymbol>,
    significant: &[u64],
) -> InputObject {
    let next_index = sections.iter().map(|sec| sec.index.0).max().unwrap_or(0) + 1;
    sections.push(addrsig_section(arena, next_index, significant));
    object(path, sections, symbols)
}

pub(crate) struct RawAddrsigObject<'a> {
    pub(crate) path: &'a str,
    pub(crate) sections: Vec<InputSection>,
    pub(crate) symbols: Vec<InputSymbol>,
    pub(crate) data: &'a [u8],
}

pub(crate) fn object_with_raw_addrsig(
    arena: &mut InputArena,
    mut input: RawAddrsigObject<'_>,
) -> InputObject {
    let next_index = input
        .sections
        .iter()
        .map(|sec| sec.index.0)
        .max()
        .unwrap_or(0)
        + 1;
    input
        .sections
        .push(raw_addrsig_section(arena, next_index, input.data));
    object(input.path, input.sections, input.symbols)
}

pub(crate) fn object_without_addrsig(
    path: &str,
    sections: Vec<InputSection>,
    symbols: Vec<InputSymbol>,
) -> InputObject {
    object(path, sections, symbols)
}

fn object(path: &str, sections: Vec<InputSection>, symbols: Vec<InputSymbol>) -> InputObject {
    let mut section_map = IndexLookup::default();
    for (pos, sec) in sections.iter().enumerate() {
        section_map.insert(sec.index.0, pos);
    }
    let mut symbol_map = IndexLookup::default();
    for (pos, sym) in symbols.iter().enumerate() {
        symbol_map.insert(sym.index.0, pos);
    }
    InputObject {
        path: path.to_string(),
        sections,
        symbols,
        section_map,
        symbol_map,
        comdat_groups: Vec::new(),
    }
}

fn raw_addrsig_section(arena: &mut InputArena, index: usize, data: &[u8]) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(b".llvm_addrsig"),
        kind: SectionKind::Other,
        sh_type: SHT_LLVM_ADDRSIG,
        data: arena.intern_bytes(data),
        align: 1,
        size: u64::try_from(data.len()).expect("test addrsig section size fits u64"),
        flags: 0,
        relocs: Vec::new(),
    }
}

fn addrsig_section(arena: &mut InputArena, index: usize, significant: &[u64]) -> InputSection {
    let mut data = Vec::new();
    for &sym_index in significant {
        assert!(sym_index < 0x80);
        data.push(sym_index as u8);
    }
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(b".llvm_addrsig"),
        kind: SectionKind::Other,
        sh_type: SHT_LLVM_ADDRSIG,
        data: arena.intern_bytes(&data),
        align: 1,
        size: data.len() as u64,
        flags: 0,
        relocs: Vec::new(),
    }
}
