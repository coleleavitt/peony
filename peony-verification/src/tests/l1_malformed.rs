use peony_object::elf;

use crate::{
    ContributionOwnerWitness,
    LayoutWindowWitness,
    LayoutWitness,
    RangeBounds,
    RangeOwnerWitness,
    RangeWitness,
    SectionRefWitness,
    WitnessError,
    check_layout_witness,
};

#[test]
fn layout_witness_rejects_overlapping_output_section_file_windows() {
    let witness = LayoutWitness {
        image_base: 0x400000,
        file_size: 0x200,
        output_sections: vec![
            output_section(".first", 0x100, 0x120),
            output_section(".second", 0x110, 0x130),
        ],
        segments: Vec::new(),
    };

    let err = check_layout_witness(&witness).expect_err("overlap must reject");

    assert_eq!(
        err,
        WitnessError::LayoutRangeOverlap {
            kind: "output section file",
            first: ".first".to_string(),
            second: ".second".to_string(),
            first_end: 0x120,
            second_start: 0x110,
        }
    );
}

#[test]
fn layout_witness_rejects_contribution_ranges_outside_output_section() {
    let witness = LayoutWitness {
        image_base: 0x400000,
        file_size: 0x200,
        output_sections: vec![LayoutWindowWitness {
            output_section_name: ".debug_info".to_string(),
            section_type: elf::SHT_PROGBITS,
            flags: 0,
            range: RangeWitness::new(
                RangeOwnerWitness::OutputSection {
                    name: ".debug_info".to_string(),
                },
                RangeBounds::new(0x100, 0x110).expect("file range is valid"),
                RangeBounds::new(0, 0).expect("VA range is valid"),
            ),
            alignment: 1,
            contributions: vec![ContributionOwnerWitness {
                section: SectionRefWitness::new(0, 5),
                output_offset: 8,
                size: 12,
                range: RangeBounds::new(8, 20).expect("contribution range is valid"),
            }],
        }],
        segments: Vec::new(),
    };

    let err = check_layout_witness(&witness).expect_err("escaped contribution must reject");

    assert_eq!(
        err,
        WitnessError::LayoutContributionOutOfBounds {
            output_section: ".debug_info".to_string(),
            section: SectionRefWitness::new(0, 5),
            contribution_end: 20,
            section_size: 16,
        }
    );
}

#[test]
fn layout_witness_rejects_malformed_output_section_alignment() {
    let witness = LayoutWitness {
        image_base: 0x400000,
        file_size: 0x200,
        output_sections: vec![LayoutWindowWitness {
            alignment: 3,
            ..output_section(".text", 0x100, 0x110)
        }],
        segments: Vec::new(),
    };

    let err = check_layout_witness(&witness).expect_err("invalid alignment must reject");

    assert_eq!(
        err,
        WitnessError::LayoutInvalidAlignment {
            owner: ".text".to_string(),
            alignment: 3,
        }
    );
}

fn output_section(name: &str, file_start: u64, file_end: u64) -> LayoutWindowWitness {
    LayoutWindowWitness {
        output_section_name: name.to_string(),
        section_type: elf::SHT_PROGBITS,
        flags: 0,
        range: RangeWitness::new(
            RangeOwnerWitness::OutputSection {
                name: name.to_string(),
            },
            RangeBounds::new(file_start, file_end).expect("file range is valid"),
            RangeBounds::new(0, 0).expect("VA range is valid"),
        ),
        alignment: 1,
        contributions: Vec::new(),
    }
}
