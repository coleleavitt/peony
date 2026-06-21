use super::*;
use crate::r_x86_64;

#[test]
fn patch_buf_rejects_modeled_32_bit_overflows_without_truncating() {
    let cases = vec![
        (r_x86_64::R32, addrs(0, |a| a.s = 0x1_0000_0000), filled(8)),
        (r_x86_64::R32S, addrs(0, |a| a.s = 0x8000_0000), filled(8)),
        (r_x86_64::PC32, addrs(0, |a| a.s = 0x8000_0000), filled(8)),
        (
            r_x86_64::GOT32,
            addrs(0, |a| a.g = 0x1_0000_0000),
            filled(8),
        ),
        (
            r_x86_64::TPOFF32,
            addrs(0, |a| a.tls = 0x8000_0000),
            filled(8),
        ),
        (
            r_x86_64::TLSGD,
            addrs(4, |a| {
                a.tls = 0x8000_0000;
                a.tls_size = 0;
            }),
            tlsgd_sequence(),
        ),
        (
            r_x86_64::TLSGD,
            addrs(4, |a| {
                a.tls_imported = true;
                a.tls_ie = 0x8000_0008;
            }),
            tlsgd_sequence(),
        ),
    ];

    for (relocation_type, addrs, original) in cases {
        assert_patch_overflow_matches_model(relocation_type, addrs, original);
    }
}
