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
use peony_symbols::SymbolTable;

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

pub(crate) fn object(
    path: &str,
    sections: Vec<InputSection>,
    symbols: Vec<InputSymbol>,
) -> InputObject {
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

pub(crate) fn symbol_table_for(objects: &[InputObject]) -> SymbolTable {
    let mut table = SymbolTable::new();
    for obj in objects {
        let id = table.add_object(obj.path.clone());
        table.process_object(id, obj).expect("test symbols resolve");
    }
    table
}
