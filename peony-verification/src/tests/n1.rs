use peony_cache::{
    CachedLinkState,
    Fingerprint,
    PatchSectionRecord,
    RelocReverseIndex,
    SectionRecord,
    plan_partial_relink,
};

use crate::{
    IncrementalColorWitness,
    IncrementalColorWitnessKind,
    IncrementalPreservationError,
    PartialEmitPreservationWitness,
    PartialEmitWriteWitness,
    WitnessError,
    check_partial_emit_preservation,
    extract_incremental_color_witnesses,
};

fn red_section(name: &str, file_offset: u64, size: u64) -> IncrementalColorWitness {
    IncrementalColorWitness {
        section_name: name.to_string(),
        file_offset,
        virtual_address: 0x400000 + file_offset,
        size,
        capacity: size,
        color: IncrementalColorWitnessKind::Red,
    }
}

fn green_section(name: &str, file_offset: u64, size: u64) -> IncrementalColorWitness {
    IncrementalColorWitness {
        section_name: name.to_string(),
        file_offset,
        virtual_address: 0x400000 + file_offset,
        size,
        capacity: size,
        color: IncrementalColorWitnessKind::Green,
    }
}

fn write(label: &str, start: u64, len: u64) -> PartialEmitWriteWitness {
    PartialEmitWriteWitness::from_start_len(label.to_string(), start, len)
        .expect("test write range is valid")
}

fn base_witness() -> PartialEmitPreservationWitness {
    PartialEmitPreservationWitness {
        sections: vec![red_section(".text", 2, 2), green_section(".rodata", 4, 2)],
        writes: vec![write("input .text", 2, 2)],
        previous_bytes: vec![0, 0, 1, 1, 7, 7],
        partial_bytes: vec![0, 0, 9, 9, 7, 7],
        full_bytes: vec![0, 0, 9, 9, 7, 7],
    }
}

#[test]
fn accepts_partial_emit_when_writes_are_red_and_green_bytes_are_untouched() {
    let witness = base_witness();

    check_partial_emit_preservation(&witness).expect("N1 witness should accept");
}

#[test]
fn rejects_write_set_touching_green_bytes() {
    let mut witness = base_witness();
    witness.writes.push(write("stale write", 4, 1));

    let err = check_partial_emit_preservation(&witness)
        .expect_err("write touching green bytes must reject");

    assert!(matches!(
        err,
        IncrementalPreservationError::WriteTouchesGreen {
            write_label,
            green_section,
            ..
        } if write_label == "stale write" && green_section == ".rodata"
    ));
}

#[test]
fn rejects_green_byte_that_changed_without_a_write_range() {
    let mut witness = base_witness();
    witness.partial_bytes[4] = 6;

    let err = check_partial_emit_preservation(&witness)
        .expect_err("green byte drift must reject even when write set omits it");

    assert!(matches!(
        err,
        IncrementalPreservationError::GreenByteChanged {
            section,
            offset: 4,
            previous: 7,
            partial: 6,
        } if section == ".rodata"
    ));
}

#[test]
fn rejects_red_byte_that_does_not_match_full_link() {
    let mut witness = base_witness();
    witness.partial_bytes[3] = 8;

    let err = check_partial_emit_preservation(&witness)
        .expect_err("red bytes must be compared against the full-link image");

    assert!(matches!(
        err,
        IncrementalPreservationError::RedByteMismatch {
            section,
            offset: 3,
            partial: 8,
            full: 9,
        } if section == ".text"
    ));
}

#[test]
fn rejects_write_range_not_covered_by_red_bytes() {
    let mut witness = base_witness();
    witness.writes = vec![write("header write", 0, 1)];

    let err = check_partial_emit_preservation(&witness)
        .expect_err("writes outside the red model set must reject");

    assert!(matches!(
        err,
        IncrementalPreservationError::WriteOutsideRed {
            write_label,
            write_start: 0,
            write_end: 1,
        } if write_label == "header write"
    ));
}

#[test]
fn rejects_out_of_bounds_and_overflowing_write_ranges() {
    let mut witness = base_witness();
    witness.writes = vec![write("oob", 5, 2)];

    let err = check_partial_emit_preservation(&witness)
        .expect_err("write range past the output image must reject");

    assert!(matches!(
        err,
        IncrementalPreservationError::WriteOutOfBounds {
            write_label,
            write_start: 5,
            write_end: 7,
            image_len: 6,
        } if write_label == "oob"
    ));

    let overflow = PartialEmitWriteWitness::from_start_len("overflow".to_string(), u64::MAX, 1)
        .expect_err("overflowing half-open range must reject");
    assert_eq!(
        overflow,
        WitnessError::RangeEndOverflow {
            start: u64::MAX,
            len: 1,
        }
    );
}

#[test]
fn rejects_green_claim_for_moved_symbol_dependent_section() {
    let mut data_prev = previous_section(".data");
    data_prev.file_offset = 6;
    data_prev.virtual_address = 0x400006;
    data_prev.size = 2;
    data_prev.capacity = 2;
    let previous = vec![previous_section(".text"), data_prev];
    let cached = CachedLinkState {
        changed_inputs: vec!["changed.o".to_string()],
        sections: previous.clone(),
        symbols: Vec::new(),
        front_end: None,
    };
    let mut data_current = current_section(".data");
    data_current.file_offset = 6;
    data_current.virtual_address = 0x400006;
    data_current.size = 2;
    let current = vec![current_section(".text"), data_current];
    let reverse_index = RelocReverseIndex::new(3, 1);
    reverse_index.insert(2, 0);
    let plan = plan_partial_relink(&cached, &current, &[2], &reverse_index, &[".data"])
        .expect("moved symbol dependent should be red");
    let sections = extract_incremental_color_witnesses(&plan, &previous, &current)
        .expect("incremental color extraction should succeed");
    assert_eq!(
        sections
            .iter()
            .find(|section| section.section_name == ".data")
            .map(|section| section.color),
        Some(IncrementalColorWitnessKind::Red)
    );

    let good = PartialEmitPreservationWitness {
        sections: sections.clone(),
        writes: vec![write("relocated .data", 6, 2)],
        previous_bytes: vec![0, 0, 1, 1, 7, 7, 1, 1],
        partial_bytes: vec![0, 0, 1, 1, 7, 7, 9, 9],
        full_bytes: vec![0, 0, 1, 1, 7, 7, 9, 9],
    };
    check_partial_emit_preservation(&good).expect("red dependent write should accept");

    let mut bad_sections = sections;
    for section in &mut bad_sections {
        if section.section_name == ".data" {
            section.color = IncrementalColorWitnessKind::Green;
        }
    }
    let bad = PartialEmitPreservationWitness {
        sections: bad_sections,
        ..good
    };

    let err = check_partial_emit_preservation(&bad)
        .expect_err("dependent section cannot be claimed green while written");

    assert!(matches!(
        err,
        IncrementalPreservationError::WriteTouchesGreen {
            write_label,
            green_section,
            ..
        } if write_label == "relocated .data" && green_section == ".data"
    ));
}

fn previous_section(name: &str) -> SectionRecord {
    SectionRecord {
        name: name.to_string(),
        fingerprint: Fingerprint::of_bytes(name.as_bytes()),
        file_offset: 2,
        size: 2,
        capacity: 2,
        virtual_address: 0x400002,
    }
}

fn current_section(name: &str) -> PatchSectionRecord {
    PatchSectionRecord {
        name: name.to_string(),
        file_offset: 2,
        size: 2,
        virtual_address: 0x400002,
        input_changed: false,
    }
}
