use peony_cache::{
    CachedLinkState,
    Fingerprint,
    PatchSectionRecord,
    RelocReverseIndex,
    SectionColor,
    SectionRecord,
    plan_partial_relink,
};
use peony_layout::{LayoutConfig, SectionFilter, TlsGotInfo, compute_layout};
use peony_object::{
    Binding,
    IndexLookup,
    InputArena,
    InputObject,
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

use crate::{
    HalfOpenRangeWitness,
    RangeBounds,
    RangeOwnerWitness,
    RangeWitness,
    SectionKindWitness,
    SectionRefWitness,
    SymbolBindingWitness,
    SymbolStateWitness,
    WitnessError,
    extract_incremental_color_witnesses,
    extract_layout_window_witnesses,
    extract_section_witnesses,
    extract_symbol_witnesses,
};

mod e1;
mod i1;
mod l1;
mod l1_malformed;
mod n1;
mod n1_real_emit;
mod n1_real_emit_fixture;
mod r1b;
mod s1;

fn text_section(arena: &mut InputArena, index: usize, data: &[u8]) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(b".text"),
        kind: SectionKind::Text,
        sh_type: elf::SHT_PROGBITS,
        data: if data.is_empty() {
            SectionData::EMPTY
        } else {
            arena.intern_bytes(data)
        },
        align: 1,
        size: u64::try_from(data.len().max(1)).expect("section size fits in u64"),
        flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        relocs: Vec::new(),
    }
}

fn symbol(name: &[u8], binding: Binding, section: usize) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(0),
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

fn symbol_table_for(objects: &[InputObject]) -> SymbolTable {
    let mut table = SymbolTable::new();
    for obj in objects {
        let id = table.add_object(obj.path.clone());
        table.process_object(id, obj).expect("test symbols resolve");
    }
    table
}

#[test]
fn rejects_invalid_range_when_start_exceeds_end() {
    let err = HalfOpenRangeWitness::new(9, 4).expect_err("invalid range must reject");

    assert_eq!(err, WitnessError::RangeStartAfterEnd { start: 9, end: 4 });
}

#[test]
fn extracts_symbol_witnesses_in_name_order_when_table_iteration_is_unordered() {
    let mut arena = InputArena::new();
    let objects = vec![
        object(
            "first.o",
            vec![text_section(&mut arena, 1, &[0xc3])],
            vec![symbol(b"zeta", Binding::Global, 1)],
        ),
        object(
            "second.o",
            vec![text_section(&mut arena, 1, &[0xc3])],
            vec![symbol(b"alpha", Binding::Weak, 1)],
        ),
    ];
    let table = symbol_table_for(&objects);

    let witnesses = extract_symbol_witnesses(&table);

    assert_eq!(witnesses[0].name, b"alpha");
    assert_eq!(witnesses[0].binding, SymbolBindingWitness::Weak);
    assert!(matches!(
        witnesses[0].state,
        SymbolStateWitness::Defined { .. }
    ));
    assert_eq!(witnesses[1].name, b"zeta");
}

#[test]
fn extracts_section_and_layout_witnesses_from_multi_object_link() {
    let mut arena = InputArena::new();
    let objects = vec![
        object(
            "a.o",
            vec![text_section(&mut arena, 1, &[0xc3])],
            vec![symbol(b"_start", Binding::Global, 1)],
        ),
        object(
            "b.o",
            vec![text_section(&mut arena, 2, &[0x90, 0xc3])],
            vec![symbol(b"helper", Binding::Global, 2)],
        ),
    ];
    let table = symbol_table_for(&objects);
    let layout = compute_layout(
        &arena,
        &objects,
        &table,
        &[],
        &[],
        SectionFilter::All,
        None,
        &LayoutConfig::default(),
        &TlsGotInfo::default(),
    )
    .expect("multi-object layout computes");

    let sections = extract_section_witnesses(&objects);
    let windows = extract_layout_window_witnesses(&layout).expect("layout witnesses extract");

    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].kind, SectionKindWitness::Text);
    assert_eq!(sections[0].owner, SectionRefWitness::new(0, 1));
    assert!(
        windows
            .iter()
            .any(|window| window.output_section_name == ".text")
    );
}

#[test]
fn extracts_incremental_color_witnesses_in_section_name_order() {
    let previous = vec![
        SectionRecord {
            name: ".zdata".to_string(),
            fingerprint: Fingerprint::default(),
            file_offset: 0x300,
            size: 4,
            capacity: 8,
            virtual_address: 0x403000,
        },
        SectionRecord {
            name: ".text".to_string(),
            fingerprint: Fingerprint::default(),
            file_offset: 0x100,
            size: 1,
            capacity: 4,
            virtual_address: 0x401000,
        },
    ];
    let current = vec![
        PatchSectionRecord {
            name: ".zdata".to_string(),
            file_offset: 0x300,
            size: 4,
            virtual_address: 0x403000,
            input_changed: true,
        },
        PatchSectionRecord {
            name: ".text".to_string(),
            file_offset: 0x100,
            size: 1,
            virtual_address: 0x401000,
            input_changed: false,
        },
    ];
    let cached = CachedLinkState {
        changed_inputs: vec!["b.o".to_string()],
        sections: previous.clone(),
        symbols: Vec::new(),
        front_end: None,
    };
    let rev_index = RelocReverseIndex::new(0, 0);
    let plan = plan_partial_relink(&cached, &current, &[], &rev_index, &[])
        .expect("partial relink plan accepted");

    let witnesses = extract_incremental_color_witnesses(&plan, &previous, &current)
        .expect("incremental color witnesses extract");

    assert_eq!(witnesses[0].section_name, ".text");
    assert_eq!(witnesses[0].color, SectionColor::Green.into());
    assert_eq!(witnesses[1].section_name, ".zdata");
    assert_eq!(witnesses[1].color, SectionColor::Red.into());
}

#[test]
fn builds_standalone_emit_range_witness_when_emit_work_items_are_private() {
    let witness = RangeWitness::new(
        RangeOwnerWitness::OutputSection {
            name: ".text".to_string(),
        },
        RangeBounds::new(0x40, 0x44).expect("file range valid"),
        RangeBounds::new(0x401040, 0x401044).expect("va range valid"),
    );

    assert_eq!(
        witness.file,
        RangeBounds {
            start: 0x40,
            end: 0x44
        }
    );
}
