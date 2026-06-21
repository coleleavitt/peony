use crate::{
    RelocationByteError,
    RelocationByteInputs,
    RelocationByteWidthKind,
    X86_64RelocationExpression,
    model_x86_64_relocation_bytes,
    x86_64_reloc,
};

const fn reloc_inputs(offset: usize) -> RelocationByteInputs {
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

#[test]
fn models_pc64_little_endian_bytes_when_expression_is_supported() {
    let mut inputs = reloc_inputs(2);
    inputs.s = 0x401020;
    inputs.a = -8;
    inputs.p = 0x400100;
    let original = [0xaau8; 12];

    let patch = model_x86_64_relocation_bytes(x86_64_reloc::PC64, &inputs, &original)
        .expect("PC64 bytes model");

    assert_eq!(patch.expression, X86_64RelocationExpression::Pc64);
    assert_eq!(patch.output_offset, 2);
    assert_eq!(patch.width, 8);
    assert_eq!(&patch.produced_bytes[0..2], &[0xaa; 2]);
    assert_eq!(
        i64::from_le_bytes(patch.produced_bytes[2..10].try_into().unwrap()),
        0xf18
    );
    assert_eq!(&patch.produced_bytes[10..12], &[0xaa; 2]);
}

#[test]
fn rejects_unsigned_32_bit_overflow_when_value_would_truncate() {
    let mut inputs = reloc_inputs(0);
    inputs.s = 0x1_0000_0000;
    let original = [0u8; 8];

    let err = model_x86_64_relocation_bytes(x86_64_reloc::R32, &inputs, &original)
        .expect_err("R32 value outside zero/sign-extended 32-bit range rejects");

    assert_eq!(
        err,
        RelocationByteError::Overflow {
            relocation_type: x86_64_reloc::R32,
            offset: 0,
            value: 0x1_0000_0000,
            width: 4,
            kind: RelocationByteWidthKind::UnsignedOrSignExtended,
        }
    );
}

#[test]
fn models_tlsgd_local_exec_rewrite_bytes_when_prologue_is_canonical() {
    let mut inputs = reloc_inputs(4);
    inputs.a = -4;
    inputs.p = 4;
    inputs.tls_size = 0x140;
    let original = [
        0x66, 0x48, 0x8d, 0x3d, 0, 0, 0, 0, 0x66, 0x66, 0x48, 0xe8, 0, 0, 0, 0,
    ];

    let patch = model_x86_64_relocation_bytes(x86_64_reloc::TLSGD, &inputs, &original)
        .expect("TLSGD local-exec bytes model");

    assert_eq!(patch.expression, X86_64RelocationExpression::TlsGdLocalExec);
    assert_eq!(patch.output_offset, 0);
    assert_eq!(patch.width, 16);
    assert_eq!(
        &patch.produced_bytes[0..12],
        &[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0x48, 0x8d, 0x80]
    );
    assert_eq!(
        i32::from_le_bytes(patch.produced_bytes[12..16].try_into().unwrap()),
        -0x140
    );
}
