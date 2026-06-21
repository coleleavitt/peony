use std::collections::HashSet;
use std::path::PathBuf;

use peony_cache::{
    CachedLinkState,
    Fingerprint,
    PatchSectionRecord,
    RelocReverseIndex,
    SectionRecord,
    plan_partial_relink,
};
use peony_layout::{
    Layout,
    LayoutConfig,
    SecSource,
    SectionFilter,
    TlsGotInfo,
    compute_layout,
    finalize_symbols,
};
use peony_object::{
    Binding,
    InputArena,
    InputObject,
    InputSection,
    Name,
    SectionIndex,
    SectionKind,
    elf,
};
use peony_symbols::SymbolTable;

use super::{object, symbol, symbol_table_for, text_section};
use crate::{IncrementalColorWitness, extract_incremental_color_witnesses};

pub(crate) struct LinkedFixture {
    pub(crate) arena: InputArena,
    pub(crate) objects: Vec<InputObject>,
    pub(crate) symbols: SymbolTable,
    pub(crate) layout: Layout,
}

pub(crate) fn linked_fixture(immediate: u8) -> LinkedFixture {
    let mut arena = InputArena::new();
    let objects = vec![
        object(
            "start.o",
            vec![text_section(&mut arena, 1, &[0xc3])],
            vec![symbol(b"_start", Binding::Global, 1)],
        ),
        object(
            "compute.o",
            vec![text_section(
                &mut arena,
                1,
                &[0xb8, immediate, 0x00, 0x00, 0x00, 0xc3],
            )],
            vec![symbol(b"compute", Binding::Global, 1)],
        ),
        object(
            "const.o",
            vec![rodata_section(&mut arena, 1, &[7, 7, 7, 7])],
            Vec::new(),
        ),
    ];
    let mut symbols = symbol_table_for(&objects);
    let layout = compute_layout(
        &arena,
        &objects,
        &symbols,
        &[],
        &[],
        SectionFilter::All,
        None,
        &LayoutConfig::default(),
        &TlsGotInfo::default(),
    )
    .expect("fixture layout computes");
    finalize_symbols(&mut symbols, &layout);
    LinkedFixture {
        arena,
        objects,
        symbols,
        layout,
    }
}

pub(crate) fn color_witnesses_from_layouts(
    previous: &Layout,
    current: &Layout,
    changed_objects: &HashSet<usize>,
) -> Vec<IncrementalColorWitness> {
    let previous_sections = section_records(previous);
    let current_sections = patch_section_records(current, changed_objects);
    let cached = CachedLinkState {
        changed_inputs: vec!["compute.o".to_string()],
        sections: previous_sections.clone(),
        symbols: Vec::new(),
        front_end: None,
    };
    let plan = plan_partial_relink(
        &cached,
        &current_sections,
        &[],
        &RelocReverseIndex::new(0, 0),
        &[],
    )
    .expect("fixture partial plan accepts");
    extract_incremental_color_witnesses(&plan, &previous_sections, &current_sections)
        .expect("fixture color witnesses extract")
}

pub(crate) fn temp_fixture_dir(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "peony-{label}-{}-{}",
        std::process::id(),
        timestamp_nanos()
    ));
    std::fs::create_dir(&path).expect("temporary fixture directory is created");
    path
}

fn rodata_section(arena: &mut InputArena, index: usize, data: &[u8]) -> InputSection {
    InputSection {
        index: SectionIndex(index),
        name: Name::from_slice(b".rodata"),
        kind: SectionKind::ReadOnly,
        sh_type: elf::SHT_PROGBITS,
        data: arena.intern_bytes(data),
        align: 1,
        size: u64::try_from(data.len()).expect("section size fits in u64"),
        flags: elf::SHF_ALLOC,
        relocs: Vec::new(),
    }
}

fn section_records(layout: &Layout) -> Vec<SectionRecord> {
    layout
        .output_sections
        .iter()
        .filter(|section| file_backed_section(section.sh_type, section.sh_size))
        .map(|section| SectionRecord {
            name: section.name.clone(),
            fingerprint: Fingerprint::of_bytes(section.name.as_bytes()),
            file_offset: section.sh_offset,
            size: section.sh_size,
            capacity: section.sh_size,
            virtual_address: section.sh_addr,
        })
        .collect()
}

fn patch_section_records(
    layout: &Layout,
    changed_objects: &HashSet<usize>,
) -> Vec<PatchSectionRecord> {
    layout
        .output_sections
        .iter()
        .filter(|section| file_backed_section(section.sh_type, section.sh_size))
        .map(|section| PatchSectionRecord {
            name: section.name.clone(),
            file_offset: section.sh_offset,
            size: section.sh_size,
            virtual_address: section.sh_addr,
            input_changed: section.source == SecSource::Input
                && section
                    .contributions
                    .iter()
                    .any(|contribution| changed_objects.contains(&contribution.object_id)),
        })
        .collect()
}

fn file_backed_section(sh_type: u32, size: u64) -> bool {
    sh_type != elf::SHT_NOBITS && size != 0
}

fn timestamp_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after epoch")
        .as_nanos()
}
