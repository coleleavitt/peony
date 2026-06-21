use peony_verification::X86_64RelocationExpression::*;

use super::*;
use crate::r_x86_64;

#[test]
fn patch_buf_matches_model_for_scalar_and_got_relocation_arms() {
    let cases = vec![
        (
            r_x86_64::R64,
            addrs(4, |a| {
                a.s = 0x400120;
                a.a = 0x18;
            }),
            filled(16),
            Abs64,
        ),
        (
            r_x86_64::PC64,
            addrs(4, |a| {
                a.s = 0x401020;
                a.a = -8;
                a.p = 0x400100;
            }),
            filled(16),
            Pc64,
        ),
        (
            r_x86_64::GOTOFF64,
            addrs(4, |a| {
                a.s = 0x405100;
                a.a = -0x10;
                a.got_base = 0x400000;
            }),
            filled(16),
            GotOff64,
        ),
        (
            r_x86_64::SIZE64,
            addrs(4, |a| {
                a.z = 0x1234;
                a.a = -4;
            }),
            filled(16),
            Size64,
        ),
        (
            r_x86_64::R32,
            addrs(2, |a| {
                a.s = 0xffff_fffe;
                a.a = 1;
            }),
            filled(8),
            Abs32,
        ),
        (
            r_x86_64::R32S,
            addrs(2, |a| {
                a.s = 0x7fff_ff00;
                a.a = -0x20;
            }),
            filled(8),
            Abs32Signed,
        ),
        (
            r_x86_64::PC32,
            addrs(2, |a| {
                a.s = 0x401020;
                a.a = -4;
                a.p = 0x400100;
            }),
            filled(8),
            Pc32,
        ),
        (
            r_x86_64::SIZE32,
            addrs(2, |a| {
                a.z = 0x1234;
                a.a = 4;
            }),
            filled(8),
            Size32,
        ),
        (
            r_x86_64::PLT32,
            addrs(2, |a| {
                a.s = 0x401020;
                a.a = -4;
                a.p = 0x400100;
            }),
            filled(8),
            Plt32,
        ),
        (
            r_x86_64::PLT32,
            addrs(2, |a| {
                a.s = 0x401020;
                a.l = 0x402000;
                a.a = -4;
                a.p = 0x400100;
            }),
            filled(8),
            Plt32,
        ),
        (
            r_x86_64::GOTPCREL,
            addrs(2, |a| {
                a.g = 0x405000;
                a.a = -4;
                a.p = 0x400080;
            }),
            filled(8),
            GotPcRel,
        ),
        (
            r_x86_64::GOTPCRELX,
            addrs(2, |a| {
                a.g = 0x405010;
                a.a = -4;
                a.p = 0x400080;
            }),
            filled(8),
            GotPcRelx,
        ),
        (
            r_x86_64::REX_GOTPCRELX,
            addrs(2, |a| {
                a.g = 0x405020;
                a.a = -4;
                a.p = 0x400080;
            }),
            filled(8),
            RexGotPcRelx,
        ),
        (
            r_x86_64::GOT32,
            addrs(2, |a| {
                a.g = 0x405100;
                a.got_base = 0x405000;
                a.a = 8;
            }),
            filled(8),
            Got32,
        ),
        (
            r_x86_64::GOTPC32,
            addrs(2, |a| {
                a.got_base = 0x405000;
                a.a = -4;
                a.p = 0x404000;
            }),
            filled(8),
            GotPc32,
        ),
        (r_x86_64::R16, addrs(1, |a| a.s = 0x1234), filled(4), Abs16),
        (
            r_x86_64::PC16,
            addrs(1, |a| {
                a.s = 0x1320;
                a.a = -4;
                a.p = 0x1200;
            }),
            filled(4),
            Pc16,
        ),
        (r_x86_64::R8, addrs(1, |a| a.s = 0x7f), filled(4), Abs8),
        (
            r_x86_64::PC8,
            addrs(1, |a| {
                a.s = 0x1204;
                a.a = -4;
                a.p = 0x1200;
            }),
            filled(4),
            Pc8,
        ),
    ];

    for (relocation_type, addrs, original, expression) in cases {
        assert_patch_matches_model(relocation_type, addrs, original, expression);
    }
}
