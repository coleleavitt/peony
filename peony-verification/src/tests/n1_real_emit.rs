use std::collections::HashSet;

use peony_emit::{EmitConfig, emit_full, emit_partial_objects_with_report};

use super::n1_real_emit_fixture::{color_witnesses_from_layouts, linked_fixture, temp_fixture_dir};
use crate::{
    IncrementalColorWitnessKind,
    IncrementalPreservationError,
    PartialEmitPreservationWitness,
    check_partial_emit_preservation,
    partial_emit_writes_from_report,
};

#[test]
fn n1_accepts_real_partial_emit_report_and_rejects_green_write_claim() {
    let dir = temp_fixture_dir("n1-real-emit");
    let output = dir.join("app");
    let full_output = dir.join("app.full");
    let previous = linked_fixture(42);
    let current = linked_fixture(77);
    let changed_objects = HashSet::from([1usize]);

    emit_full(
        &output,
        &previous.arena,
        &previous.objects,
        &previous.symbols,
        &previous.layout,
        &EmitConfig::default(),
    )
    .expect("previous full emit succeeds");
    let previous_bytes = std::fs::read(&output).expect("previous output is readable");

    let report = emit_partial_objects_with_report(
        &output,
        &current.arena,
        &current.objects,
        &current.symbols,
        &current.layout,
        &EmitConfig::default(),
        &changed_objects,
    )
    .expect("partial object emit succeeds")
    .expect("existing output is patched");
    assert!(
        report
            .ranges()
            .iter()
            .any(|range| range.label().starts_with("input compute.o")),
        "real report should include the changed object's accepted input write"
    );
    let partial_bytes = std::fs::read(&output).expect("partial output is readable");

    emit_full(
        &full_output,
        &current.arena,
        &current.objects,
        &current.symbols,
        &current.layout,
        &EmitConfig::default(),
    )
    .expect("current full emit succeeds");
    let full_bytes = std::fs::read(&full_output).expect("full output is readable");

    let sections =
        color_witnesses_from_layouts(&previous.layout, &current.layout, &changed_objects);
    assert_eq!(
        color_for(&sections, ".text"),
        Some(IncrementalColorWitnessKind::Red)
    );
    assert_eq!(
        color_for(&sections, ".rodata"),
        Some(IncrementalColorWitnessKind::Green)
    );
    let writes = partial_emit_writes_from_report(&report).expect("report ranges convert");
    let good = PartialEmitPreservationWitness {
        sections,
        writes,
        previous_bytes,
        partial_bytes,
        full_bytes,
    };
    check_partial_emit_preservation(&good).expect("real partial emit report should accept");

    let mut bad = good.clone();
    for section in &mut bad.sections {
        if section.section_name == ".text" {
            section.color = IncrementalColorWitnessKind::Green;
        }
    }
    let err = check_partial_emit_preservation(&bad)
        .expect_err("claiming the real emitted .text range is green must reject");
    assert!(matches!(
        err,
        IncrementalPreservationError::WriteTouchesGreen {
            write_label,
            green_section,
            ..
        } if write_label.starts_with("input compute.o") && green_section == ".text"
    ));

    std::fs::remove_dir_all(&dir).expect("temporary fixture directory is removed");
}

fn color_for(
    sections: &[crate::IncrementalColorWitness],
    name: &str,
) -> Option<IncrementalColorWitnessKind> {
    sections
        .iter()
        .find(|section| section.section_name == name)
        .map(|section| section.color)
}
