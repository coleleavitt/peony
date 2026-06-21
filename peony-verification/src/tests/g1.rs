use peony_object::{
    Binding,
    IndexLookup,
    InputArena,
    InputObject,
    InputReloc,
    InputSection,
    InputSymbol,
    Name,
    SectionIndex,
    SectionKind,
    SymbolIndex,
    elf,
};

use crate::{
    GcRootReasonWitness,
    SectionRefWitness,
    WitnessError,
    check_gc_witness,
    extract_gc_witness,
};

const R_X86_64_PC32: u32 = 2;

fn section(
    arena: &mut InputArena,
    index: usize,
    name: &[u8],
    flags: u64,
    relocs: Vec<InputReloc>,
) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(name),
        kind: SectionKind::Text,
        sh_type: elf::SHT_PROGBITS,
        data: arena.intern_bytes(&[0xc3]),
        align: 1,
        size: 1,
        flags,
        relocs,
    }
}

fn symbol(
    index: usize,
    name: &[u8],
    binding: Binding,
    section: usize,
    visibility: u8,
) -> InputSymbol {
    InputSymbol {
        index: SymbolIndex(index),
        name: Name::from_slice(name),
        binding,
        is_undefined: false,
        is_common: false,
        is_ifunc: false,
        st_type: elf::STT_FUNC,
        visibility,
        section: Some(SectionIndex(section)),
        value: 0,
        size: 1,
    }
}

fn object(path: &str, sections: Vec<InputSection>, symbols: Vec<InputSymbol>) -> InputObject {
    let mut section_map = IndexLookup::default();
    for (pos, section) in sections.iter().enumerate() {
        section_map.insert(section.index.0, pos);
    }
    let mut symbol_map = IndexLookup::default();
    for (pos, symbol) in symbols.iter().enumerate() {
        symbol_map.insert(symbol.index.0, pos);
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

fn symbol_table_for(objects: &[InputObject]) -> peony_symbols::SymbolTable {
    let mut table = peony_symbols::SymbolTable::new();
    for object in objects {
        let id = table.add_object(object.path.clone());
        table
            .process_object(id, object)
            .expect("test symbols resolve");
    }
    table
}

fn two_hop_fixture() -> (Vec<InputObject>, peony_symbols::SymbolTable) {
    let mut arena = InputArena::new();
    let sections = vec![
        section(
            &mut arena,
            1,
            b".text.start",
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            vec![InputReloc {
                offset: 0,
                r_type: R_X86_64_PC32,
                symbol: SymbolIndex(1),
                addend: -4,
            }],
        ),
        section(
            &mut arena,
            2,
            b".text.mid",
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            vec![InputReloc {
                offset: 0,
                r_type: R_X86_64_PC32,
                symbol: SymbolIndex(2),
                addend: -4,
            }],
        ),
        section(
            &mut arena,
            3,
            b".text.leaf",
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            Vec::new(),
        ),
        section(
            &mut arena,
            4,
            b".text.dead",
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            Vec::new(),
        ),
    ];
    let symbols = vec![
        symbol(0, b"_start", Binding::Global, 1, elf::STV_DEFAULT),
        symbol(1, b"mid", Binding::Local, 2, elf::STV_DEFAULT),
        symbol(2, b"leaf", Binding::Local, 3, elf::STV_DEFAULT),
        symbol(3, b"dead", Binding::Local, 4, elf::STV_DEFAULT),
    ];
    let objects = vec![object("g1-two-hop.o", sections, symbols)];
    let table = symbol_table_for(&objects);
    (objects, table)
}

#[test]
fn gc_witness_accepts_two_hop_relocation_chain_and_rejects_unreachable_section() {
    let (objects, table) = two_hop_fixture();

    let witness = extract_gc_witness(&objects, &table, "_start", false);
    let reachable = check_gc_witness(&witness).expect("G1 witness matches Rust GC");

    assert_eq!(
        reachable,
        vec![
            SectionRefWitness::new(0, 1),
            SectionRefWitness::new(0, 2),
            SectionRefWitness::new(0, 3),
        ]
    );
    assert!(!witness.rust_live.contains(&SectionRefWitness::new(0, 4)));
}

#[test]
fn gc_witness_records_all_exposed_root_classes() {
    let mut arena = InputArena::new();
    let sections = vec![
        section(&mut arena, 1, b".text.export", elf::SHF_ALLOC, Vec::new()),
        section(&mut arena, 2, b".eh_frame", elf::SHF_ALLOC, Vec::new()),
        section(
            &mut arena,
            3,
            b".gcc_except_table",
            elf::SHF_ALLOC,
            Vec::new(),
        ),
        section(&mut arena, 4, b".init_array", elf::SHF_ALLOC, Vec::new()),
        section(&mut arena, 5, b".fini_array", elf::SHF_ALLOC, Vec::new()),
        section(&mut arena, 6, b".preinit_array", elf::SHF_ALLOC, Vec::new()),
        section(
            &mut arena,
            7,
            b".text.retained",
            elf::SHF_ALLOC | elf::SHF_GNU_RETAIN,
            Vec::new(),
        ),
    ];
    let symbols = vec![symbol(0, b"exported", Binding::Global, 1, elf::STV_DEFAULT)];
    let objects = vec![object("g1-roots.o", sections, symbols)];
    let table = symbol_table_for(&objects);

    let witness = extract_gc_witness(&objects, &table, "_missing", true);

    assert!(has_root(&witness, 1, GcRootReasonWitness::Export));
    assert!(has_root(&witness, 2, GcRootReasonWitness::EhFrame));
    assert!(has_root(&witness, 3, GcRootReasonWitness::GccExceptTable));
    assert!(has_root(&witness, 4, GcRootReasonWitness::InitFini));
    assert!(has_root(&witness, 5, GcRootReasonWitness::InitFini));
    assert!(has_root(&witness, 6, GcRootReasonWitness::InitFini));
    assert!(has_root(&witness, 7, GcRootReasonWitness::RetainFlag));
    check_gc_witness(&witness).expect("root-only G1 witness matches Rust GC");
}

#[test]
fn gc_witness_rejects_when_extracted_edge_is_missing() {
    let (objects, table) = two_hop_fixture();
    let mut witness = extract_gc_witness(&objects, &table, "_start", false);
    witness
        .edges
        .retain(|edge| edge.from != SectionRefWitness::new(0, 2));

    let err = check_gc_witness(&witness).expect_err("missing edge must reject");

    assert_eq!(
        err,
        WitnessError::GcReachabilityMismatch {
            model_only: Vec::new(),
            rust_only: vec![SectionRefWitness::new(0, 3)],
        }
    );
}

#[test]
fn gc_witness_rejects_when_extracted_root_is_missing() {
    let mut arena = InputArena::new();
    let sections = vec![section(
        &mut arena,
        7,
        b".text.retained",
        elf::SHF_ALLOC | elf::SHF_GNU_RETAIN,
        Vec::new(),
    )];
    let objects = vec![object("g1-retain.o", sections, Vec::new())];
    let table = symbol_table_for(&objects);
    let mut witness = extract_gc_witness(&objects, &table, "_missing", false);
    witness.roots.clear();

    let err = check_gc_witness(&witness).expect_err("missing root must reject");

    assert_eq!(
        err,
        WitnessError::GcReachabilityMismatch {
            model_only: Vec::new(),
            rust_only: vec![SectionRefWitness::new(0, 7)],
        }
    );
}

fn has_root(
    witness: &crate::GcReachabilityWitness,
    section_index: usize,
    reason: GcRootReasonWitness,
) -> bool {
    witness
        .roots
        .iter()
        .any(|root| root.root == SectionRefWitness::new(0, section_index) && root.reason == reason)
}
