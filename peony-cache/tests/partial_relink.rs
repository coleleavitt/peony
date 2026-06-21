use peony_cache::{
    CachedLinkState,
    Fingerprint,
    PartialRelinkFallback,
    PatchSectionRecord,
    RelocReverseIndex,
    SectionColor,
    SectionRecord,
    plan_partial_relink,
};

fn cached_state(sections: Vec<SectionRecord>) -> CachedLinkState {
    CachedLinkState {
        changed_inputs: vec!["changed.o".to_string()],
        sections,
        symbols: Vec::new(),
        front_end: None,
    }
}

fn previous_section(name: &str) -> SectionRecord {
    SectionRecord {
        name: name.to_string(),
        fingerprint: Fingerprint::of_bytes(name.as_bytes()),
        file_offset: 0x1000,
        size: 0x20,
        capacity: 0x20,
        virtual_address: 0x401000,
    }
}

fn current_section(name: &str) -> PatchSectionRecord {
    PatchSectionRecord {
        name: name.to_string(),
        file_offset: 0x1000,
        size: 0x20,
        virtual_address: 0x401000,
        input_changed: false,
    }
}

#[test]
fn accepts_only_capacity_stable_section_metadata() {
    let cached = cached_state(vec![previous_section(".text")]);
    let mut current = current_section(".text");
    current.input_changed = true;

    let plan = plan_partial_relink(&cached, &[current], &[], &RelocReverseIndex::new(0, 0), &[])
        .expect("stable metadata should be patchable");

    assert_eq!(plan.color(".text"), Some(SectionColor::Red));
    assert!(plan.is_red(".text"));
}

#[test]
fn rejects_missing_section_metadata() {
    let cached = cached_state(Vec::new());

    let reason = plan_partial_relink(
        &cached,
        &[current_section(".text")],
        &[],
        &RelocReverseIndex::new(0, 0),
        &[],
    )
    .expect_err("missing previous metadata must force full emit");

    assert_eq!(reason, PartialRelinkFallback::MissingSectionMetadata);
}

#[test]
fn rejects_missing_previous_section() {
    let cached = cached_state(vec![previous_section(".text")]);

    let reason = plan_partial_relink(
        &cached,
        &[current_section(".rodata")],
        &[],
        &RelocReverseIndex::new(0, 0),
        &[],
    )
    .expect_err("unknown section must force full emit");

    assert_eq!(
        reason,
        PartialRelinkFallback::MissingPreviousSection {
            section: ".rodata".to_string()
        }
    );
}

#[test]
fn rejects_file_offset_drift() {
    let cached = cached_state(vec![previous_section(".text")]);
    let mut current = current_section(".text");
    current.file_offset += 0x10;

    let reason = plan_partial_relink(&cached, &[current], &[], &RelocReverseIndex::new(0, 0), &[])
        .expect_err("file-offset drift must force full emit");

    assert_eq!(
        reason,
        PartialRelinkFallback::SectionFileOffsetChanged {
            section: ".text".to_string(),
            previous: 0x1000,
            current: 0x1010
        }
    );
}

#[test]
fn rejects_virtual_address_drift() {
    let cached = cached_state(vec![previous_section(".text")]);
    let mut current = current_section(".text");
    current.virtual_address += 0x10;

    let reason = plan_partial_relink(&cached, &[current], &[], &RelocReverseIndex::new(0, 0), &[])
        .expect_err("virtual-address drift must force full emit");

    assert_eq!(
        reason,
        PartialRelinkFallback::SectionVirtualAddressChanged {
            section: ".text".to_string(),
            previous: 0x401000,
            current: 0x401010
        }
    );
}

#[test]
fn rejects_growth_beyond_cached_capacity() {
    let cached = cached_state(vec![previous_section(".text")]);
    let mut current = current_section(".text");
    current.size += 1;

    let reason = plan_partial_relink(&cached, &[current], &[], &RelocReverseIndex::new(0, 0), &[])
        .expect_err("capacity overflow must force full emit");

    assert_eq!(
        reason,
        PartialRelinkFallback::SectionCapacityExceeded {
            section: ".text".to_string(),
            capacity: 0x20,
            size: 0x21
        }
    );
}

#[test]
fn rejects_size_change_even_when_capacity_would_fit() {
    let mut previous = previous_section(".text");
    previous.capacity = 0x40;
    let cached = cached_state(vec![previous]);
    let mut current = current_section(".text");
    current.size = 0x30;

    let reason = plan_partial_relink(&cached, &[current], &[], &RelocReverseIndex::new(0, 0), &[])
        .expect_err("size changes are not patchable until padding is implemented");

    assert_eq!(
        reason,
        PartialRelinkFallback::SectionSizeChanged {
            section: ".text".to_string(),
            previous: 0x20,
            current: 0x30
        }
    );
}

#[test]
fn marks_relocation_dependents_red_when_symbol_moved() {
    let mut data_prev = previous_section(".data");
    data_prev.file_offset = 0x2000;
    data_prev.virtual_address = 0x402000;
    let cached = cached_state(vec![previous_section(".text"), data_prev]);

    let mut data_current = current_section(".data");
    data_current.file_offset = 0x2000;
    data_current.virtual_address = 0x402000;
    let current = vec![current_section(".text"), data_current];

    let reverse_index = RelocReverseIndex::new(3, 1);
    reverse_index.insert(2, 0);
    let plan = plan_partial_relink(&cached, &current, &[2], &reverse_index, &[".data"])
        .expect("moved-symbol dependents should produce a conservative plan");

    assert_eq!(plan.color(".text"), Some(SectionColor::Green));
    assert_eq!(plan.color(".data"), Some(SectionColor::Red));
}
