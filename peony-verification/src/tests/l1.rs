use peony_layout::{LayoutConfig, SectionFilter, TlsGotInfo, compute_layout};
use peony_object::{
    Binding,
    InputArena,
    InputSection,
    Name,
    SectionData,
    SectionIndex,
    SectionKind,
    elf,
};

use crate::{check_layout_witness, extract_layout_witness};

struct SectionSpec<'a> {
    index: usize,
    name: &'a [u8],
    kind: SectionKind,
    section_type: u32,
    flags: u64,
    align: u64,
    data: &'a [u8],
    size: u64,
}

fn input_section(arena: &mut InputArena, spec: SectionSpec<'_>) -> InputSection {
    InputSection {
        index: SectionIndex(spec.index),
        name: Name::from_slice(spec.name),
        kind: spec.kind,
        sh_type: spec.section_type,
        data: if spec.data.is_empty() {
            SectionData::EMPTY
        } else {
            arena.intern_bytes(spec.data)
        },
        align: spec.align,
        size: spec.size,
        flags: spec.flags,
        relocs: Vec::new(),
    }
}

#[test]
fn layout_witness_accepts_alloc_debug_bss_and_tls_windows() {
    let mut arena = InputArena::new();
    let sections = vec![
        input_section(
            &mut arena,
            SectionSpec {
                index: 1,
                name: b".text",
                kind: SectionKind::Text,
                section_type: elf::SHT_PROGBITS,
                flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
                align: 16,
                data: &[0xc3],
                size: 1,
            },
        ),
        input_section(
            &mut arena,
            SectionSpec {
                index: 2,
                name: b".rodata",
                kind: SectionKind::ReadOnly,
                section_type: elf::SHT_PROGBITS,
                flags: elf::SHF_ALLOC,
                align: 8,
                data: b"hello\0",
                size: 6,
            },
        ),
        input_section(
            &mut arena,
            SectionSpec {
                index: 3,
                name: b".data",
                kind: SectionKind::Data,
                section_type: elf::SHT_PROGBITS,
                flags: elf::SHF_ALLOC | elf::SHF_WRITE,
                align: 8,
                data: &[1, 2, 3, 4],
                size: 4,
            },
        ),
        input_section(
            &mut arena,
            SectionSpec {
                index: 4,
                name: b".bss",
                kind: SectionKind::Bss,
                section_type: elf::SHT_NOBITS,
                flags: elf::SHF_ALLOC | elf::SHF_WRITE,
                align: 16,
                data: &[],
                size: 32,
            },
        ),
        input_section(
            &mut arena,
            SectionSpec {
                index: 5,
                name: b".debug_info",
                kind: SectionKind::Debug,
                section_type: elf::SHT_PROGBITS,
                flags: 0,
                align: 1,
                data: &[0, 1, 2, 3],
                size: 4,
            },
        ),
        input_section(
            &mut arena,
            SectionSpec {
                index: 6,
                name: b".tdata",
                kind: SectionKind::Tdata,
                section_type: elf::SHT_PROGBITS,
                flags: elf::SHF_ALLOC | elf::SHF_WRITE | elf::SHF_TLS,
                align: 8,
                data: &[9, 10, 11, 12],
                size: 4,
            },
        ),
    ];
    let objects = vec![super::object(
        "layout-l1.o",
        sections,
        vec![super::symbol(b"_start", Binding::Global, 1)],
    )];
    let table = super::symbol_table_for(&objects);
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
    .expect("L1 layout fixture computes");

    let witness = extract_layout_witness(&layout).expect("layout witness extracts");

    check_layout_witness(&witness).expect("layout witness satisfies L1 invariants");
    assert!(
        witness
            .segments
            .iter()
            .any(|segment| segment.segment_type == elf::PT_TLS)
    );
    let debug = witness
        .output_sections
        .iter()
        .find(|section| section.output_section_name == ".debug_info")
        .expect("debug section is witnessed");
    assert_eq!(debug.range.va.start, debug.range.va.end);
    let bss = witness
        .output_sections
        .iter()
        .find(|section| section.output_section_name == ".bss")
        .expect("bss section is witnessed");
    assert_eq!(bss.range.file.start, bss.range.file.end);
    assert_eq!(bss.range.va.end - bss.range.va.start, 32);
}
