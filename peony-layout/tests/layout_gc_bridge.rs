#[path = "common/layout.rs"]
mod layout_common;

use layout_common::{object, section, symbol, symbol_table_for};
use peony_layout::{GcRootReason, gc_graph_rooted, gc_sections};
use peony_object::{Binding, InputArena, InputReloc, SectionKind, SymbolIndex, elf};

const R_X86_64_PC32: u32 = 2;

#[test]
fn gc_marks_reachable_sections_and_rejects_unreachable_ones() {
    let mut arena = InputArena::new();
    let sections = vec![
        section(
            &mut arena,
            1,
            b".text.start",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xe8, 0, 0, 0, 0],
            vec![InputReloc {
                offset: 1,
                r_type: R_X86_64_PC32,
                symbol: SymbolIndex(1),
                addend: -4,
            }],
        ),
        section(
            &mut arena,
            2,
            b".text.helper",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xc3],
            vec![InputReloc {
                offset: 0,
                r_type: R_X86_64_PC32,
                symbol: SymbolIndex(2),
                addend: -4,
            }],
        ),
        section(
            &mut arena,
            4,
            b".text.leaf",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xc3],
            Vec::new(),
        ),
        section(
            &mut arena,
            3,
            b".text.dead",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xc3],
            Vec::new(),
        ),
    ];
    let symbols = vec![
        symbol(
            0,
            b"_start",
            Binding::Global,
            1,
            elf::STT_FUNC,
            elf::STV_DEFAULT,
        ),
        symbol(
            1,
            b"helper",
            Binding::Local,
            2,
            elf::STT_FUNC,
            elf::STV_DEFAULT,
        ),
        symbol(
            2,
            b"leaf",
            Binding::Local,
            4,
            elf::STT_FUNC,
            elf::STV_DEFAULT,
        ),
        symbol(
            3,
            b"dead",
            Binding::Local,
            3,
            elf::STT_FUNC,
            elf::STV_DEFAULT,
        ),
    ];
    let objects = vec![object("gc-bridge.o", sections, symbols)];
    let table = symbol_table_for(&objects);

    let live = gc_sections(&objects, &table, "_start");

    assert!(live.contains(&(0, 1)));
    assert!(live.contains(&(0, 2)));
    assert!(live.contains(&(0, 4)));
    assert!(!live.contains(&(0, 3)));
}

#[test]
fn gc_graph_exposes_entry_root_and_two_hop_relocation_edges() {
    let mut arena = InputArena::new();
    let sections = vec![
        section(
            &mut arena,
            1,
            b".text.start",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xe8, 0, 0, 0, 0],
            vec![InputReloc {
                offset: 1,
                r_type: R_X86_64_PC32,
                symbol: SymbolIndex(1),
                addend: -4,
            }],
        ),
        section(
            &mut arena,
            2,
            b".text.helper",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xe8, 0, 0, 0, 0],
            vec![InputReloc {
                offset: 1,
                r_type: R_X86_64_PC32,
                symbol: SymbolIndex(2),
                addend: -4,
            }],
        ),
        section(
            &mut arena,
            3,
            b".text.leaf",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xc3],
            Vec::new(),
        ),
    ];
    let symbols = vec![
        symbol(
            0,
            b"_start",
            Binding::Global,
            1,
            elf::STT_FUNC,
            elf::STV_DEFAULT,
        ),
        symbol(
            1,
            b"helper",
            Binding::Local,
            2,
            elf::STT_FUNC,
            elf::STV_DEFAULT,
        ),
        symbol(
            2,
            b"leaf",
            Binding::Local,
            3,
            elf::STT_FUNC,
            elf::STV_DEFAULT,
        ),
    ];
    let objects = vec![object("gc-graph.o", sections, symbols)];
    let table = symbol_table_for(&objects);

    let graph = gc_graph_rooted(&objects, &table, "_start", false);

    assert!(
        graph
            .roots
            .iter()
            .any(|root| { root.section == (0, 1) && root.reason == GcRootReason::Entry })
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| { edge.from == (0, 1) && edge.to == (0, 2) })
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| { edge.from == (0, 2) && edge.to == (0, 3) })
    );
}

#[test]
fn gc_keeps_gnu_retain_alloc_section_as_root() {
    let mut arena = InputArena::new();
    let sections = vec![section(
        &mut arena,
        7,
        b".text.retained",
        SectionKind::Text,
        elf::SHF_ALLOC | elf::SHF_EXECINSTR | elf::SHF_GNU_RETAIN,
        &[0xc3],
        Vec::new(),
    )];
    let symbols = vec![symbol(
        0,
        b"retained",
        Binding::Local,
        7,
        elf::STT_FUNC,
        elf::STV_DEFAULT,
    )];
    let objects = vec![object("gc-retain.o", sections, symbols)];
    let table = symbol_table_for(&objects);

    let live = gc_sections(&objects, &table, "_missing_entry");
    let graph = gc_graph_rooted(&objects, &table, "_missing_entry", false);

    assert!(live.contains(&(0, 7)));
    assert_eq!(live.len(), 1);
    assert!(
        graph
            .roots
            .iter()
            .any(|root| { root.section == (0, 7) && root.reason == GcRootReason::RetainFlag })
    );
}
