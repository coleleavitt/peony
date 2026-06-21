use crate::{
    EmitWorkRangeWitness,
    RangeBounds,
    RangeOwnerWitness,
    RangeWitness,
    SectionRefWitness,
    WitnessError,
};

fn accepted_range() -> RangeWitness {
    RangeWitness::new(
        RangeOwnerWitness::InputSection(SectionRefWitness::new(0, 1)),
        RangeBounds::new(0x40, 0x50).expect("accepted file range is valid"),
        RangeBounds::new(0x401040, 0x401050).expect("accepted VA range is valid"),
    )
}

#[test]
fn emit_work_range_witness_accepts_copy_and_relocation_footprints_inside_range() {
    let witness = EmitWorkRangeWitness::new(
        accepted_range(),
        RangeBounds::new(0x40, 0x50).expect("section copy range is valid"),
        vec![
            RangeBounds::new(0x44, 0x48).expect("PC32 footprint is valid"),
            RangeBounds::new(0x40, 0x50).expect("TLSGD relaxed footprint is valid"),
        ],
    )
    .expect("all E1 ranges are contained");

    assert_eq!(witness.relocation_footprints.len(), 2);
}

#[test]
fn emit_work_range_witness_rejects_section_copy_outside_accepted_range() {
    let err = EmitWorkRangeWitness::new(
        accepted_range(),
        RangeBounds::new(0x3f, 0x50).expect("section copy range is valid"),
        Vec::new(),
    )
    .expect_err("section copy must stay inside accepted work range");

    assert_eq!(
        err,
        WitnessError::RangeNotContained {
            kind: "section copy",
            range_start: 0x3f,
            range_end: 0x50,
            accepted_start: 0x40,
            accepted_end: 0x50,
        }
    );
}

#[test]
fn emit_work_range_witness_rejects_relocation_footprint_outside_accepted_range() {
    let err = EmitWorkRangeWitness::new(
        accepted_range(),
        RangeBounds::new(0x40, 0x50).expect("section copy range is valid"),
        vec![RangeBounds::new(0x4e, 0x52).expect("relocation footprint range is valid")],
    )
    .expect_err("relocation footprint must stay inside accepted work range");

    assert_eq!(
        err,
        WitnessError::RangeNotContained {
            kind: "relocation footprint",
            range_start: 0x4e,
            range_end: 0x52,
            accepted_start: 0x40,
            accepted_end: 0x50,
        }
    );
}
