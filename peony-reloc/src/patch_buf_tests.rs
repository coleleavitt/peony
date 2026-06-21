mod overflow;
mod scalar;
mod tls;

use peony_verification::{
    RelocationByteError,
    RelocationByteInputs,
    X86_64RelocationExpression,
    model_x86_64_relocation_bytes,
    x86_64_relocation_expression,
};

use crate::RelocError;
use crate::apply::{RelocAddrs, patch_buf};

const fn base_addrs(offset: usize) -> RelocAddrs {
    RelocAddrs {
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

fn addrs(offset: usize, edit: impl FnOnce(&mut RelocAddrs)) -> RelocAddrs {
    let mut addrs = base_addrs(offset);
    edit(&mut addrs);
    addrs
}

fn model_inputs(addrs: &RelocAddrs) -> RelocationByteInputs {
    RelocationByteInputs {
        s: addrs.s,
        a: addrs.a,
        p: addrs.p,
        g: addrs.g,
        l: addrs.l,
        z: addrs.z,
        got_base: addrs.got_base,
        tls: addrs.tls,
        tls_size: addrs.tls_size,
        offset: addrs.offset,
        shared: addrs.shared,
        tls_gd: addrs.tls_gd,
        tls_ie: addrs.tls_ie,
        tls_desc: addrs.tls_desc,
        tls_ldm: addrs.tls_ldm,
        tls_imported: addrs.tls_imported,
    }
}

fn assert_patch_matches_model(
    relocation_type: u32,
    addrs: RelocAddrs,
    original: Vec<u8>,
    expression: X86_64RelocationExpression,
) {
    let inputs = model_inputs(&addrs);
    assert_eq!(
        x86_64_relocation_expression(relocation_type, &inputs),
        expression
    );
    let expected = model_x86_64_relocation_bytes(relocation_type, &inputs, &original)
        .expect("relocation branch is modeled");
    let mut actual = original;

    patch_buf(&mut actual, relocation_type, &addrs, "bridge.o").unwrap();

    assert_eq!(actual, expected.produced_bytes);
}

fn assert_patch_overflow_matches_model(relocation_type: u32, addrs: RelocAddrs, original: Vec<u8>) {
    let inputs = model_inputs(&addrs);
    let expected = model_x86_64_relocation_bytes(relocation_type, &inputs, &original)
        .expect_err("model rejects overflowing relocation");
    let mut actual = original;
    let err = patch_buf(&mut actual, relocation_type, &addrs, "overflow.o")
        .expect_err("patch_buf rejects overflowing relocation");

    match (err, expected) {
        (
            RelocError::Overflow {
                object,
                offset,
                value,
                r_type,
            },
            RelocationByteError::Overflow {
                relocation_type,
                offset: model_offset,
                value: model_value,
                ..
            },
        ) => {
            assert_eq!(object, "overflow.o");
            assert_eq!(r_type, relocation_type);
            assert_eq!(offset, model_offset);
            assert_eq!(value, model_value);
        }
        (actual, expected) => panic!("unexpected overflow shape: {actual:?} vs {expected:?}"),
    }
}

fn filled(len: usize) -> Vec<u8> {
    vec![0xaa; len]
}

fn tlsgd_sequence() -> Vec<u8> {
    vec![
        0x66, 0x48, 0x8d, 0x3d, 0, 0, 0, 0, 0x66, 0x66, 0x48, 0xe8, 0, 0, 0, 0,
    ]
}

fn tlsdesc_sequence() -> Vec<u8> {
    vec![0x48, 0x8d, 0x05, 0, 0, 0, 0, 0xff, 0x10, 0x90]
}
