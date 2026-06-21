use peony_object::{
    DataSrc,
    IndexLookup,
    InputObject,
    InputReloc,
    InputSection,
    Name,
    SectionData,
    SectionIndex,
    SectionKind,
    SymbolIndex,
    elf,
};
use peony_reloc::r_x86_64;

use super::{WorkItem, WorkRange, WorkRangeError, validate_work_item_ranges};

fn test_section(len: usize, relocs: Vec<InputReloc>) -> InputSection {
    InputSection {
        index: SectionIndex(1),
        name: Name::from_slice(b".text"),
        kind: SectionKind::Text,
        sh_type: elf::SHT_PROGBITS,
        data: SectionData {
            src: DataSrc::Mmap(0),
            off: 0,
            len: u32::try_from(len).expect("test section length fits in u32"),
        },
        align: 1,
        size: u64::try_from(len).expect("test section length fits in u64"),
        flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
        relocs,
    }
}

fn test_object(section: InputSection) -> InputObject {
    let mut section_map = IndexLookup::default();
    section_map.insert(section.index.0, 0);
    InputObject {
        path: "test.o".to_string(),
        sections: vec![section],
        symbols: Vec::new(),
        section_map,
        symbol_map: IndexLookup::default(),
        comdat_groups: Vec::new(),
    }
}

fn reloc(offset: u64, r_type: u32) -> InputReloc {
    InputReloc {
        offset,
        r_type,
        symbol: SymbolIndex(0),
        addend: 0,
    }
}

fn work_item<'a>(obj: &'a InputObject, file_off: usize) -> WorkItem<'a> {
    let isec = &obj.sections[0];
    WorkItem {
        file_off,
        file_len: isec.data.len(),
        section_va: 0x401000 + u64::try_from(file_off).expect("test offset fits in u64"),
        obj,
        isec,
        obj_id: 0,
        input_section_index: isec.index.0,
        reloc_count: isec.relocs.len(),
    }
}

#[test]
fn validate_work_ranges_accepts_adjacent_and_unordered_ranges() {
    let mut ranges = [
        WorkRange {
            item_index: 2,
            start: 12,
            end: 16,
        },
        WorkRange {
            item_index: 0,
            start: 0,
            end: 8,
        },
        WorkRange {
            item_index: 1,
            start: 8,
            end: 12,
        },
    ];

    let accepted = super::validate_work_ranges(&mut ranges).expect("adjacent ranges are disjoint");

    assert_eq!(
        accepted
            .range_for_item(1)
            .map(|range| (range.start(), range.end())),
        Some((8, 12))
    );
}

#[test]
fn validate_work_ranges_rejects_overlap() {
    let mut ranges = [
        WorkRange {
            item_index: 0,
            start: 4,
            end: 12,
        },
        WorkRange {
            item_index: 1,
            start: 11,
            end: 14,
        },
    ];

    assert_eq!(
        super::validate_work_ranges(&mut ranges),
        Err(WorkRangeError::Overlap {
            first_index: 0,
            first_start: 4,
            first_end: 12,
            second_index: 1,
            second_start: 11,
            second_end: 14,
        })
    );
}

#[test]
fn work_range_from_parts_rejects_overflow() {
    assert_eq!(
        WorkRange::from_parts(3, usize::MAX - 1, 8, usize::MAX),
        Err(WorkRangeError::Overflow {
            item_index: 3,
            file_off: usize::MAX - 1,
            file_len: 8,
        })
    );
}

#[test]
fn work_range_from_parts_rejects_out_of_bounds() {
    assert_eq!(
        WorkRange::from_parts(4, 12, 8, 16),
        Err(WorkRangeError::OutOfBounds {
            item_index: 4,
            file_off: 12,
            file_len: 8,
            buf_len: 16,
        })
    );
}

#[test]
fn work_range_from_parts_ignores_empty_ranges() {
    assert_eq!(WorkRange::from_parts(5, usize::MAX, 0, 0), Ok(None));
}

#[test]
fn validate_work_item_ranges_witness_contains_section_copy_when_range_is_accepted() {
    let obj = test_object(test_section(8, vec![reloc(4, r_x86_64::PC32)]));
    let item = work_item(&obj, 0x20);

    let accepted = validate_work_item_ranges(&[item], 0x40).expect("work item range is valid");
    let range = accepted
        .range_for_item(0)
        .expect("non-empty work item has accepted range");

    assert_eq!(range.item_index(), 0);
    assert_eq!((range.start(), range.end(), range.len()), (0x20, 0x28, 8));
    assert!(range.contains(item.file_off, item.file_off + item.file_len));
}

#[test]
fn validate_work_item_ranges_rejects_section_copy_outside_accepted_range() {
    let obj = test_object(test_section(8, Vec::new()));
    let mut item = work_item(&obj, 0x20);
    item.file_len = 4;

    assert_eq!(
        validate_work_item_ranges(&[item], 0x40),
        Err(WorkRangeError::OutOfBounds {
            item_index: 0,
            file_off: 0x20,
            file_len: 8,
            buf_len: 4,
        })
    );
}

#[test]
fn accepted_work_ranges_use_same_item_index_witness_for_serial_and_batched_traversal() {
    let objects = [
        test_object(test_section(4, Vec::new())),
        test_object(test_section(4, Vec::new())),
        test_object(test_section(4, Vec::new())),
    ];
    let items = [
        work_item(&objects[0], 0x20),
        work_item(&objects[1], 0x28),
        work_item(&objects[2], 0x24),
    ];
    let accepted = validate_work_item_ranges(&items, 0x40).expect("work item ranges are valid");

    let serial: Vec<_> = (0..items.len())
        .filter_map(|item_index| accepted.range_for_item(item_index))
        .map(|range| (range.item_index(), range.start(), range.end()))
        .collect();
    let batched: Vec<_> = (0..items.len())
        .collect::<Vec<_>>()
        .chunks(2)
        .flat_map(|batch| batch.iter().copied())
        .filter_map(|item_index| accepted.range_for_item(item_index))
        .map(|range| (range.item_index(), range.start(), range.end()))
        .collect();

    assert_eq!(serial, batched);
}

#[test]
fn validate_work_item_ranges_rejects_relocation_footprint_past_section_copy() {
    let obj = test_object(test_section(8, vec![reloc(6, r_x86_64::PC32)]));
    let item = work_item(&obj, 0x20);

    assert_eq!(
        validate_work_item_ranges(&[item], 0x40),
        Err(WorkRangeError::RelocationOutOfBounds {
            item_index: 0,
            reloc_index: 0,
            r_type: r_x86_64::PC32,
            offset: 6,
            width: 4,
            section_len: 8,
        })
    );
}

#[test]
fn validate_work_item_ranges_rejects_tls_prefix_footprint_before_section_copy() {
    let obj = test_object(test_section(16, vec![reloc(3, r_x86_64::TLSGD)]));
    let item = work_item(&obj, 0x20);

    assert_eq!(
        validate_work_item_ranges(&[item], 0x40),
        Err(WorkRangeError::RelocationBeforeSection {
            item_index: 0,
            reloc_index: 0,
            r_type: r_x86_64::TLSGD,
            offset: 3,
            prefix_len: 4,
        })
    );
}

#[test]
fn validate_work_item_ranges_rejects_overlap_before_dispatch_witness_exists() {
    let first = test_object(test_section(8, Vec::new()));
    let second = test_object(test_section(8, Vec::new()));
    let items = [work_item(&first, 0x20), work_item(&second, 0x24)];

    assert_eq!(
        validate_work_item_ranges(&items, 0x40),
        Err(WorkRangeError::Overlap {
            first_index: 0,
            first_start: 0x20,
            first_end: 0x28,
            second_index: 1,
            second_start: 0x24,
            second_end: 0x2c,
        })
    );
}
