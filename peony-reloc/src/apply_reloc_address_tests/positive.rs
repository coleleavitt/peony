use peony_layout::{SectionFilter, TlsGotInfo};
use peony_object::{Binding, InputArena, InputReloc, SymbolIndex};
use peony_verification::X86_64RelocationExpression;

use super::fixtures::{
    assert_apply_matches_witness_and_model,
    layout_for,
    object,
    symbol,
    symbol_table_for,
    tdata_section,
    text_section,
};
use crate::apply::ApplyCtx;
use crate::r_x86_64;

#[test]
fn apply_reloc_records_local_global_weak_got_and_plt_addresses_for_byte_model() {
    // Given: one real layout with local, global, weak-undefined, GOT, and PLT state.
    let mut arena = InputArena::new();
    let objects = vec![object(
        "r1b-address.o",
        vec![text_section(&mut arena)],
        vec![
            symbol(0, b"_start", Binding::Global, Some(1), 0, 1),
            symbol(1, b"local", Binding::Local, Some(1), 3, 5),
            symbol(2, b"global", Binding::Global, Some(1), 8, 6),
            symbol(3, b"weak_undef", Binding::Weak, None, 0, 0),
        ],
    )];
    let mut table = symbol_table_for(&objects);
    let global_id = table.lookup(b"global").expect("global symbol exists").id;
    let weak_id = table
        .ensure_id(b"weak_undef")
        .expect("weak undefined symbol gets a GOT id");
    let layout = layout_for(
        &arena,
        &objects,
        &mut table,
        &[global_id, weak_id],
        &[global_id],
        SectionFilter::All,
        &TlsGotInfo::default(),
    );
    let text_va = layout.address_of(0, 1).expect(".text was placed");
    let global = table.lookup(b"global").expect("global was finalized");
    let weak = table
        .lookup(b"weak_undef")
        .expect("weak undefined was finalized");
    let ctx = ApplyCtx {
        symbols: &table,
        layout: &layout,
        shared: false,
        sym_index: None,
    };

    // When: apply_reloc resolves each relocation and the witness is fed to R1a.
    let local_reloc = InputReloc {
        offset: 2,
        r_type: r_x86_64::R64,
        symbol: SymbolIndex(1),
        addend: 7,
    };
    let local = assert_apply_matches_witness_and_model(
        &ctx,
        &objects[0],
        &local_reloc,
        text_va,
        vec![0xaa; 16],
        X86_64RelocationExpression::Abs64,
    );
    let plt = assert_apply_matches_witness_and_model(
        &ctx,
        &objects[0],
        &InputReloc {
            offset: 2,
            r_type: r_x86_64::PLT32,
            symbol: SymbolIndex(2),
            addend: -4,
        },
        text_va,
        vec![0xaa; 8],
        X86_64RelocationExpression::Plt32,
    );
    let got = assert_apply_matches_witness_and_model(
        &ctx,
        &objects[0],
        &InputReloc {
            offset: 2,
            r_type: r_x86_64::GOTPCREL,
            symbol: SymbolIndex(2),
            addend: -4,
        },
        text_va,
        vec![0xaa; 8],
        X86_64RelocationExpression::GotPcRel,
    );
    let weak_got = assert_apply_matches_witness_and_model(
        &ctx,
        &objects[0],
        &InputReloc {
            offset: 2,
            r_type: r_x86_64::GOTPCREL,
            symbol: SymbolIndex(3),
            addend: -4,
        },
        text_va,
        vec![0xaa; 8],
        X86_64RelocationExpression::GotPcRel,
    );

    // Then: the byte witness inputs match Rust's local/global/GOT/PLT resolution.
    assert_eq!(local.s, text_va + 3);
    assert_eq!(local.a, 7);
    assert_eq!(local.p, text_va + 2);
    assert_eq!(local.g, 0);
    assert_eq!(local.l, 0);
    assert_eq!(local.z, 5);
    assert_eq!(plt.s, global.virtual_address);
    assert_eq!(plt.l, global.plt_address);
    assert_eq!(got.g, global.got_address);
    assert_eq!(weak_got.s, 0);
    assert_eq!(weak_got.g, weak.got_address);
    assert_eq!(weak_got.l, 0);
    assert_eq!(weak_got.z, 0);
}

#[test]
fn apply_reloc_records_static_tls_address_inputs_for_byte_model() {
    // Given: a placed TLS section and a local TPOFF relocation from .text.
    let mut arena = InputArena::new();
    let objects = vec![object(
        "r1b-tls.o",
        vec![text_section(&mut arena), tdata_section(&mut arena)],
        vec![
            symbol(0, b"_start", Binding::Global, Some(1), 0, 1),
            symbol(1, b"tls_local", Binding::Local, Some(2), 2, 4),
        ],
    )];
    let mut table = symbol_table_for(&objects);
    let layout = layout_for(
        &arena,
        &objects,
        &mut table,
        &[],
        &[],
        SectionFilter::All,
        &TlsGotInfo::default(),
    );
    let text_va = layout.address_of(0, 1).expect(".text was placed");
    let tls_section_offset = layout.tls_offset(0, 2).expect(".tdata has TLS offset");
    let ctx = ApplyCtx {
        symbols: &table,
        layout: &layout,
        shared: false,
        sym_index: None,
    };

    // When: the relocation is resolved and patched through the R1a byte model.
    let addrs = assert_apply_matches_witness_and_model(
        &ctx,
        &objects[0],
        &InputReloc {
            offset: 2,
            r_type: r_x86_64::TPOFF32,
            symbol: SymbolIndex(1),
            addend: -4,
        },
        text_va,
        vec![0xaa; 8],
        X86_64RelocationExpression::TpOff32,
    );

    // Then: TLS inputs are the concrete layout TLS offset plus symbol value.
    assert_eq!(addrs.tls, tls_section_offset + 2);
    assert_eq!(addrs.tls_size, layout.tls_size);
    assert_eq!(addrs.tls_gd, 0);
    assert_eq!(addrs.tls_ie, 0);
    assert_eq!(addrs.tls_desc, 0);
    assert_eq!(addrs.tls_ldm, 0);
    assert!(!addrs.tls_imported);
}
