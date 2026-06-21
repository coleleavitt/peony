use peony_verification::X86_64RelocationExpression::*;

use super::*;
use crate::r_x86_64;

#[test]
fn patch_buf_matches_model_for_tls_offset_and_kept_dynamic_tls_arms() {
    let cases = vec![
        (
            r_x86_64::TPOFF32,
            addrs(2, |a| {
                a.tls = 0x20;
                a.tls_size = 0x100;
                a.a = -4;
            }),
            filled(8),
            TpOff32,
        ),
        (
            r_x86_64::TPOFF64,
            addrs(4, |a| {
                a.tls = 0x20;
                a.tls_size = 0x100;
                a.a = -4;
            }),
            filled(16),
            TpOff64,
        ),
        (
            r_x86_64::DTPOFF32,
            addrs(2, |a| {
                a.shared = true;
                a.tls = 0x20;
                a.a = 4;
            }),
            filled(8),
            Dtpoff32Shared,
        ),
        (
            r_x86_64::DTPOFF32,
            addrs(2, |a| {
                a.tls = 0x20;
                a.tls_size = 0x100;
                a.a = 4;
            }),
            filled(8),
            Dtpoff32LocalExec,
        ),
        (
            r_x86_64::DTPOFF64,
            addrs(4, |a| {
                a.shared = true;
                a.tls = 0x20;
                a.a = 4;
            }),
            filled(16),
            Dtpoff64Shared,
        ),
        (
            r_x86_64::DTPOFF64,
            addrs(4, |a| {
                a.tls = 0x20;
                a.tls_size = 0x100;
                a.a = 4;
            }),
            filled(16),
            Dtpoff64LocalExec,
        ),
        (
            r_x86_64::TLSGD,
            addrs(4, |a| {
                a.shared = true;
                a.tls_gd = 0x406000;
                a.a = -4;
                a.p = 0x405000;
            }),
            filled(16),
            TlsGdShared,
        ),
        (
            r_x86_64::TLSLD,
            addrs(4, |a| {
                a.shared = true;
                a.tls_ldm = 0x406020;
                a.a = -4;
                a.p = 0x405000;
            }),
            filled(16),
            TlsLdShared,
        ),
        (
            r_x86_64::GOTTPOFF,
            addrs(2, |a| {
                a.shared = true;
                a.tls_ie = 0x407000;
                a.a = -4;
                a.p = 0x406000;
            }),
            filled(8),
            GotTpOffShared,
        ),
        (
            r_x86_64::GOTTPOFF,
            addrs(2, |a| {
                a.tls_ie = 0x407000;
                a.a = -4;
                a.p = 0x406000;
            }),
            filled(8),
            GotTpOffExecutable,
        ),
        (
            r_x86_64::GOTPC32_TLSDESC,
            addrs(3, |a| {
                a.shared = true;
                a.tls_desc = 0x408000;
                a.a = -4;
                a.p = 0x407000;
            }),
            tlsdesc_sequence(),
            TlsDescGotPcShared,
        ),
        (
            r_x86_64::TLSDESC_CALL,
            addrs(7, |a| a.shared = true),
            tlsdesc_sequence(),
            TlsDescCallShared,
        ),
        (r_x86_64::TLSDESC, base_addrs(2), filled(8), UnsupportedNoop),
    ];

    for (relocation_type, addrs, original, expression) in cases {
        assert_patch_matches_model(relocation_type, addrs, original, expression);
    }
}

#[test]
fn patch_buf_matches_model_for_tls_relaxation_rewrite_arms() {
    let cases = vec![
        (
            r_x86_64::TLSGD,
            addrs(4, |a| {
                a.tls_imported = true;
                a.tls_ie = 0x5000;
                a.a = -4;
                a.p = 0x4000;
            }),
            tlsgd_sequence(),
            TlsGdInitialExec,
        ),
        (
            r_x86_64::TLSGD,
            addrs(4, |a| {
                a.a = -4;
                a.p = 4;
                a.tls_size = 0x140;
            }),
            tlsgd_sequence(),
            TlsGdLocalExec,
        ),
        (r_x86_64::TLSLD, base_addrs(3), filled(12), TlsLdLocalExec),
        (
            r_x86_64::GOTPC32_TLSDESC,
            addrs(3, |a| {
                a.tls_imported = true;
                a.tls_ie = 0x5000;
                a.a = -4;
                a.p = 0x4000;
            }),
            tlsdesc_sequence(),
            TlsDescGotPcInitialExec,
        ),
        (
            r_x86_64::GOTPC32_TLSDESC,
            addrs(3, |a| {
                a.a = -4;
                a.p = 3;
                a.tls = 4;
                a.tls_size = 8;
            }),
            tlsdesc_sequence(),
            TlsDescGotPcLocalExec,
        ),
        (
            r_x86_64::TLSDESC_CALL,
            base_addrs(7),
            tlsdesc_sequence(),
            TlsDescCallLocalExec,
        ),
        (
            r_x86_64::TLSGD,
            addrs(4, |a| a.tls_size = 0x140),
            filled(16),
            TlsGdLocalExec,
        ),
        (
            r_x86_64::GOTPC32_TLSDESC,
            base_addrs(3),
            filled(10),
            TlsDescGotPcLocalExec,
        ),
    ];

    for (relocation_type, addrs, original, expression) in cases {
        assert_patch_matches_model(relocation_type, addrs, original, expression);
    }
}
