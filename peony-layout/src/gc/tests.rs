use peony_object::{
    Binding,
    IndexLookup,
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

use super::gc_sections;

fn section(index: usize, name: &[u8], relocs: Vec<InputReloc>) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(name),
        kind: SectionKind::Text,
        sh_type: elf::SHT_PROGBITS,
        data: SectionData::EMPTY,
        align: 1,
        size: 1,
        flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        relocs,
    }
}

fn symbol(index: usize, name: &[u8], binding: Binding, section: usize) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(index),
        name: Name::from_slice(name),
        binding,
        is_undefined: false,
        is_common: false,
        is_ifunc: false,
        st_type: elf::STT_FUNC,
        visibility: elf::STV_DEFAULT,
        section: Some(SectionIndex(section)),
        value: 0,
        size: 1,
    }
}

fn object(sections: Vec<InputSection>, symbols: Vec<InputSymbol>) -> InputObject {
    let mut section_map = IndexLookup::default();
    for (pos, sec) in sections.iter().enumerate() {
        section_map.insert(sec.index.0, pos);
    }
    let mut symbol_map = IndexLookup::default();
    for (pos, sym) in symbols.iter().enumerate() {
        symbol_map.insert(sym.index.0, pos);
    }
    InputObject {
        path: "gc-test.o".to_string(),
        sections,
        symbols,
        section_map,
        symbol_map,
        comdat_groups: Vec::new(),
    }
}

#[test]
fn follows_relocations_from_entry_section() {
    let sections = vec![
        section(
            1,
            b".text.start",
            vec![InputReloc {
                offset: 0,
                r_type: peony_object::elf::R_X86_64_64,
                symbol: SymbolIndex(1),
                addend: 0,
            }],
        ),
        section(2, b".text.helper", Vec::new()),
    ];
    let symbols = vec![
        symbol(0, b"_start", Binding::Global, 1),
        symbol(1, b"helper", Binding::Local, 2),
    ];
    let obj = object(sections, symbols);
    let mut table = SymbolTable::new();
    let obj_id = table.add_object(obj.path.clone());
    table
        .process_object(obj_id, &obj)
        .expect("test object resolves");

    let live = gc_sections(&[obj], &table, "_start");

    assert!(live.contains(&(0, 1)));
    assert!(live.contains(&(0, 2)));
    assert_eq!(live.len(), 2);
}

#[test]
fn follows_sparse_relocation_symbol_indices() {
    let sections = vec![
        section(
            1,
            b".text.start",
            vec![InputReloc {
                offset: 0,
                r_type: peony_object::elf::R_X86_64_64,
                symbol: SymbolIndex(5000),
                addend: 0,
            }],
        ),
        section(2, b".text.helper", Vec::new()),
    ];
    let symbols = vec![
        symbol(0, b"_start", Binding::Global, 1),
        symbol(5000, b"helper", Binding::Local, 2),
    ];
    let obj = object(sections, symbols);
    let mut table = SymbolTable::new();
    let obj_id = table.add_object(obj.path.clone());
    table
        .process_object(obj_id, &obj)
        .expect("test object resolves");

    let out = super::gc_sections_rooted_with_stats(&[obj], &table, "_start", false);

    assert!(out.live.contains(&(0, 1)));
    assert!(out.live.contains(&(0, 2)));
    assert_eq!(out.live.len(), 2);
    assert_eq!(out.stats.sparse_target_objects, 1);
}
