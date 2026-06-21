#[path = "common/layout.rs"]
mod layout_common;

use layout_common::{object, section, symbol, symbol_table_for};
use peony_layout::{LayoutConfig, SecSource, SectionFilter, TlsGotInfo, compute_layout};
use peony_object::{Binding, InputArena, SectionKind, elf};

#[test]
fn compute_layout_emits_page_congruent_non_overlapping_file_ranges() {
    let mut arena = InputArena::new();
    let sections = vec![section(
        &mut arena,
        1,
        b".text",
        SectionKind::Text,
        elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        &[0xc3],
        Vec::new(),
    )];
    let symbols = vec![symbol(
        0,
        b"_start",
        Binding::Global,
        1,
        elf::STT_FUNC,
        elf::STV_DEFAULT,
    )];
    let objects = vec![object("layout-bridge.o", sections, symbols)];
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
    .expect("layout should compute");

    for sec in &layout.output_sections {
        if sec.sh_flags & elf::SHF_ALLOC != 0 {
            assert_eq!(
                sec.sh_addr - layout.image_base,
                sec.sh_offset,
                "{} must preserve vaddr/file-offset congruence",
                sec.name
            );
        }
    }

    let mut ranges: Vec<(u64, u64, &str)> = layout
        .output_sections
        .iter()
        .filter(|sec| sec.source != SecSource::Bss && sec.sh_size != 0)
        .map(|sec| {
            (
                sec.sh_offset,
                sec.sh_offset + sec.sh_size,
                sec.name.as_str(),
            )
        })
        .collect();
    ranges.sort_unstable_by_key(|(start, end, _)| (*start, *end));

    for (start, end, name) in &ranges {
        assert!(*end <= layout.file_size, "{name} must fit in output file");
        assert!(*start <= *end, "{name} range must be well formed");
    }
    for pair in ranges.windows(2) {
        let (_, first_end, first_name) = pair[0];
        let (second_start, _, second_name) = pair[1];
        assert!(
            first_end <= second_start,
            "{first_name} and {second_name} file ranges must be disjoint"
        );
    }
}

#[test]
fn compute_layout_l1_fixture_exposes_debug_bss_data_and_tls_windows() {
    let mut arena = InputArena::new();
    let sections = vec![
        section(
            &mut arena,
            1,
            b".text",
            SectionKind::Text,
            elf::SHF_ALLOC | elf::SHF_EXECINSTR,
            &[0xc3],
            Vec::new(),
        ),
        section(
            &mut arena,
            2,
            b".rodata",
            SectionKind::ReadOnly,
            elf::SHF_ALLOC,
            b"hello\0",
            Vec::new(),
        ),
        section(
            &mut arena,
            3,
            b".data",
            SectionKind::Data,
            elf::SHF_ALLOC | elf::SHF_WRITE,
            &[1, 2, 3, 4],
            Vec::new(),
        ),
        section(
            &mut arena,
            4,
            b".bss",
            SectionKind::Bss,
            elf::SHF_ALLOC | elf::SHF_WRITE,
            &[],
            Vec::new(),
        ),
        section(
            &mut arena,
            5,
            b".debug_info",
            SectionKind::Debug,
            0,
            &[0, 1, 2, 3],
            Vec::new(),
        ),
        section(
            &mut arena,
            6,
            b".tdata",
            SectionKind::Tdata,
            elf::SHF_ALLOC | elf::SHF_WRITE | elf::SHF_TLS,
            &[9, 10, 11, 12],
            Vec::new(),
        ),
    ];
    let symbols = vec![symbol(
        0,
        b"_start",
        Binding::Global,
        1,
        elf::STT_FUNC,
        elf::STV_DEFAULT,
    )];
    let objects = vec![object("layout-l1.o", sections, symbols)];
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
    .expect("L1 fixture layout should compute");

    let debug = layout
        .output_sections
        .iter()
        .find(|section| section.name == ".debug_info")
        .expect("debug output section is present");
    assert_eq!(debug.sh_flags & elf::SHF_ALLOC, 0);
    assert_eq!(debug.sh_addr, 0);
    assert_eq!(debug.source, SecSource::Input);
    let bss = layout
        .output_sections
        .iter()
        .find(|section| section.name == ".bss")
        .expect("bss output section is present");
    assert_eq!(bss.source, SecSource::Bss);
    assert_eq!(bss.sh_type, elf::SHT_NOBITS);
    assert!(
        layout
            .segments
            .iter()
            .any(|segment| segment.p_type == elf::PT_TLS)
    );
    assert!(
        layout
            .output_sections
            .iter()
            .any(|section| { section.name == ".tdata" && section.sh_flags & elf::SHF_TLS != 0 })
    );
}
