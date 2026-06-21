use crate::{
    ApplyRelocAddressError,
    ApplyRelocAddressWitness,
    ApplyRelocPlaceWitness,
    ApplyRelocTargetWitness,
    ApplyRelocTlsWitness,
    RelocationByteInputs,
    X86_64RelocationExpression,
    check_apply_reloc_address_witness,
    x86_64_reloc,
    x86_64_relocation_expression,
};

const fn base_inputs(offset: usize) -> RelocationByteInputs {
    RelocationByteInputs {
        s: 0,
        a: 0,
        p: 0,
        g: 0,
        l: 0,
        z: 0,
        got_base: 0,
        tls: 0,
        tls_size: 0,
        offset,
        shared: false,
        tls_gd: 0,
        tls_ie: 0,
        tls_desc: 0,
        tls_ldm: 0,
        tls_imported: false,
    }
}

fn witness(
    relocation_type: u32,
    target: ApplyRelocTargetWitness,
    tls: ApplyRelocTlsWitness,
    inputs: RelocationByteInputs,
) -> ApplyRelocAddressWitness {
    ApplyRelocAddressWitness {
        relocation_type,
        addend: inputs.a,
        place: ApplyRelocPlaceWitness {
            section_va: 0x401000,
            reloc_offset: 4,
        },
        target,
        got_base: inputs.got_base,
        shared: inputs.shared,
        tls,
        inputs,
    }
}

fn assert_checked_expression(
    witness: ApplyRelocAddressWitness,
    expression: X86_64RelocationExpression,
) -> RelocationByteInputs {
    let accepted =
        check_apply_reloc_address_witness(&witness).expect("R1b address witness is accepted");
    assert_eq!(
        x86_64_relocation_expression(witness.relocation_type, &accepted),
        expression
    );
    accepted
}

#[test]
fn accepts_local_apply_reloc_address_witness_when_inputs_match_section_layout() {
    // Given: a local section symbol and byte inputs with S/A/P/Z from that layout.
    let mut inputs = base_inputs(4);
    inputs.s = 0x500020;
    inputs.a = 7;
    inputs.p = 0x401004;
    inputs.z = 3;
    inputs.got_base = 0x600000;
    let witness = witness(
        x86_64_reloc::R64,
        ApplyRelocTargetWitness::LocalSection {
            section_address: Some(0x500000),
            symbol_value: 0x20,
            size: 3,
        },
        ApplyRelocTlsWitness::Absent { tls_size: 0 },
        inputs,
    );

    // When: the R1b witness is checked before feeding R1a's byte model.
    let accepted = assert_checked_expression(witness, X86_64RelocationExpression::Abs64);

    // Then: the accepted byte inputs are exactly the supplied relocation inputs.
    assert_eq!(accepted, inputs);
}

#[test]
fn accepts_global_plt_got_and_weak_undefined_address_witnesses() {
    // Given: global and weak-undefined symbol resolution facts with GOT and PLT slots.
    let mut global_inputs = base_inputs(4);
    global_inputs.s = 0x402000;
    global_inputs.a = -4;
    global_inputs.p = 0x401004;
    global_inputs.g = 0x603000;
    global_inputs.l = 0x404000;
    global_inputs.z = 8;
    global_inputs.got_base = 0x603000;
    let global = witness(
        x86_64_reloc::PLT32,
        ApplyRelocTargetWitness::GlobalDefined {
            virtual_address: 0x402000,
            got_address: 0x603000,
            plt_address: 0x404000,
            size: 8,
        },
        ApplyRelocTlsWitness::Absent { tls_size: 0 },
        global_inputs,
    );
    let mut weak_inputs = base_inputs(4);
    weak_inputs.a = -4;
    weak_inputs.p = 0x401004;
    weak_inputs.g = 0x603008;
    weak_inputs.got_base = 0x603000;
    let weak = witness(
        x86_64_reloc::GOTPCREL,
        ApplyRelocTargetWitness::WeakUndefined {
            got_address: 0x603008,
        },
        ApplyRelocTlsWitness::Absent { tls_size: 0 },
        weak_inputs,
    );

    // When: each witness is checked against the address-resolution fields.
    let global_accepted = assert_checked_expression(global, X86_64RelocationExpression::Plt32);
    let weak_accepted = assert_checked_expression(weak, X86_64RelocationExpression::GotPcRel);

    // Then: both accepted witnesses retain the GOT/PLT values passed to the byte model.
    assert_eq!(global_accepted.g, 0x603000);
    assert_eq!(global_accepted.l, 0x404000);
    assert_eq!(weak_accepted.s, 0);
    assert_eq!(weak_accepted.g, 0x603008);
}

#[test]
fn accepts_tls_apply_reloc_address_witness_when_inputs_match_tls_layout() {
    // Given: a static TLS symbol with section-relative TLS offset and TLS GOT state.
    let mut inputs = base_inputs(4);
    inputs.s = 0x405002;
    inputs.a = -4;
    inputs.p = 0x401004;
    inputs.z = 4;
    inputs.got_base = 0x603000;
    inputs.tls = 0x22;
    inputs.tls_size = 0x100;
    inputs.tls_ie = 0x603040;
    let witness = witness(
        x86_64_reloc::TPOFF32,
        ApplyRelocTargetWitness::LocalSection {
            section_address: Some(0x405000),
            symbol_value: 2,
            size: 4,
        },
        ApplyRelocTlsWitness::SectionRelative {
            section_tls_offset: Some(0x20),
            symbol_value: 2,
            tls_size: 0x100,
            tls_gd: 0,
            tls_ie: 0x603040,
            tls_desc: 0,
            tls_ldm: 0,
        },
        inputs,
    );

    // When: the TLS witness is accepted for the R1a byte model.
    let accepted = assert_checked_expression(witness, X86_64RelocationExpression::TpOff32);

    // Then: TLS input T is the section TLS offset plus symbol value.
    assert_eq!(accepted.tls, 0x22);
    assert_eq!(accepted.tls_size, 0x100);
    assert_eq!(accepted.tls_ie, 0x603040);
}

#[test]
fn rejects_apply_reloc_address_witness_when_layout_or_inputs_are_missing() {
    // Given: malformed R1b witnesses for a missing target layout and mismatched S.
    let mut missing_inputs = base_inputs(4);
    missing_inputs.p = 0x401004;
    let missing_layout = witness(
        x86_64_reloc::R64,
        ApplyRelocTargetWitness::LocalSection {
            section_address: None,
            symbol_value: 0x20,
            size: 3,
        },
        ApplyRelocTlsWitness::Absent { tls_size: 0 },
        missing_inputs,
    );
    let mut mismatch_inputs = base_inputs(4);
    mismatch_inputs.s = 0xdead;
    mismatch_inputs.p = 0x401004;
    let mismatch = witness(
        x86_64_reloc::R64,
        ApplyRelocTargetWitness::GlobalDefined {
            virtual_address: 0x402000,
            got_address: 0,
            plt_address: 0,
            size: 0,
        },
        ApplyRelocTlsWitness::Absent { tls_size: 0 },
        mismatch_inputs,
    );

    // When: the malformed witnesses are checked.
    let missing_err = check_apply_reloc_address_witness(&missing_layout)
        .expect_err("missing target layout rejects the bridge");
    let mismatch_err = check_apply_reloc_address_witness(&mismatch)
        .expect_err("mismatched byte inputs reject the bridge");

    // Then: R1b declines the bridge instead of relying on stale byte-model inputs.
    assert_eq!(
        missing_err,
        ApplyRelocAddressError::MissingAddress {
            field: "target.section_address",
        }
    );
    assert_eq!(
        mismatch_err,
        ApplyRelocAddressError::InputMismatch {
            field: "s",
            expected: 0x402000,
            actual: 0xdead,
        }
    );
}
