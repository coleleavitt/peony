use peony_layout::{SectionFilter, TlsGotInfo};
use peony_object::{Binding, InputArena, InputReloc, SymbolIndex};
use rustc_hash::FxHashSet;

use super::fixtures::{data_section, layout_for, object, symbol, symbol_table_for, text_section};
use crate::apply::{
    ApplyCtx,
    RelocAction,
    RelocSkipReason,
    apply_reloc,
    resolve_reloc_action_for_test,
};
use crate::{RelocError, r_x86_64};

#[test]
fn apply_reloc_declines_address_witness_when_local_target_layout_is_missing() {
    // Given: .text is live, but the local relocation target's .data placement is absent.
    let mut arena = InputArena::new();
    let objects = vec![object(
        "r1b-missing-layout.o",
        vec![text_section(&mut arena), data_section(&mut arena)],
        vec![
            symbol(0, b"_start", Binding::Global, Some(1), 0, 1),
            symbol(1, b"discarded_local", Binding::Local, Some(2), 0, 4),
        ],
    )];
    let mut table = symbol_table_for(&objects);
    let mut live = FxHashSet::default();
    live.insert((0, 1));
    let layout = layout_for(
        &arena,
        &objects,
        &mut table,
        &[],
        &[],
        SectionFilter::Only(&live),
        &TlsGotInfo::default(),
    );
    let text_va = layout.address_of(0, 1).expect(".text was placed");
    let ctx = ApplyCtx {
        symbols: &table,
        layout: &layout,
        shared: false,
        sym_index: None,
    };
    let reloc = InputReloc {
        offset: 2,
        r_type: r_x86_64::R64,
        symbol: SymbolIndex(1),
        addend: 0,
    };
    let original = vec![0xaa; 16];
    let mut actual = original.clone();

    // When: the target section has no layout address.
    let action = resolve_reloc_action_for_test(&ctx, &objects[0], 0, &reloc, text_va)
        .expect("missing local layout is a decline, not a hard relocation error");
    apply_reloc(&ctx, &objects[0], 0, &reloc, text_va, &mut actual)
        .expect("production behavior still skips the discarded target");

    // Then: R1b declines the address bridge claim and production bytes stay unchanged.
    assert_eq!(
        action,
        RelocAction::Skip(RelocSkipReason::MissingLocalSectionAddress)
    );
    assert_eq!(actual, original);
}

#[test]
fn apply_reloc_rejects_address_witness_when_strong_symbol_address_is_missing() {
    // Given: a strong undefined symbol with no global resolution address.
    let mut arena = InputArena::new();
    let objects = vec![object(
        "r1b-missing-symbol.o",
        vec![text_section(&mut arena)],
        vec![
            symbol(0, b"_start", Binding::Global, Some(1), 0, 1),
            symbol(1, b"missing", Binding::Global, None, 0, 0),
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
    let ctx = ApplyCtx {
        symbols: &table,
        layout: &layout,
        shared: false,
        sym_index: None,
    };
    let reloc = InputReloc {
        offset: 2,
        r_type: r_x86_64::R64,
        symbol: SymbolIndex(1),
        addend: 0,
    };

    // When: address resolution reaches the unresolved strong symbol.
    let err = resolve_reloc_action_for_test(&ctx, &objects[0], 0, &reloc, text_va)
        .expect_err("strong undefined symbol rejects R1b address witness");

    // Then: the bridge cannot claim a byte witness without a symbol address.
    assert!(matches!(
        err,
        RelocError::UndefinedSymbol {
            name,
            object
        } if name == "missing" && object == "r1b-missing-symbol.o"
    ));
}
