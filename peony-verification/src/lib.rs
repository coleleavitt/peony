//! Internal Peony verification witnesses.
//!
//! These types are deliberately small Rust-side facts for bridge tests and
//! future Rocq refinement work. They are not a stable interchange format.

#[cfg(test)]
mod reloc_byte_tests;
#[cfg(test)]
mod tests;

mod extract;
mod gc;
mod icf;
mod incremental;
mod layout;
mod layout_extract;
mod model;
mod range;
mod reloc_addr;
mod reloc_byte;

pub use extract::{
    extract_incremental_color_witnesses,
    extract_section_witnesses,
    extract_symbol_error_witness,
    extract_symbol_witnesses,
};
pub use gc::{check_gc_witness, extract_gc_witness, model_gc_reachable};
pub use icf::{
    IcfFoldKeyWitness,
    IcfFoldPairWitness,
    IcfFoldWitness,
    IcfRelocationSummaryWitness,
    IcfSectionWitness,
    IcfWitnessError,
    check_icf_fold_witnesses,
    extract_icf_fold_witnesses,
};
pub use incremental::{
    IncrementalPreservationError,
    PartialEmitPreservationWitness,
    PartialEmitWriteWitness,
    check_partial_emit_preservation,
    partial_emit_writes_from_report,
};
pub use layout::check_layout_witness;
pub use layout_extract::{
    extract_layout_segment_witnesses,
    extract_layout_window_witnesses,
    extract_layout_witness,
};
pub use model::{
    ContributionOwnerWitness,
    GcEdgeReasonWitness,
    GcEdgeWitness,
    GcReachabilityWitness,
    GcRootReasonWitness,
    GcRootWitness,
    IncrementalColorWitness,
    IncrementalColorWitnessKind,
    LayoutSegmentWitness,
    LayoutWindowWitness,
    LayoutWitness,
    RelocationWriteWitness,
    SectionKindWitness,
    SectionRefWitness,
    SectionWitness,
    SymbolBindingWitness,
    SymbolErrorWitness,
    SymbolRefWitness,
    SymbolStateWitness,
    SymbolWitness,
};
pub use range::{
    EmitWorkRangeWitness,
    HalfOpenRangeWitness,
    RangeBounds,
    RangeOwnerWitness,
    RangeWitness,
    WitnessError,
};
pub use reloc_addr::{
    ApplyRelocAddressError,
    ApplyRelocAddressWitness,
    ApplyRelocPlaceWitness,
    ApplyRelocTargetWitness,
    ApplyRelocTlsWitness,
    check_apply_reloc_address_witness,
};
pub use reloc_byte::{
    RelocationByteError,
    RelocationByteInputs,
    RelocationBytePatch,
    RelocationByteWidthKind,
    X86_64RelocationExpression,
    model_x86_64_relocation_bytes,
    x86_64_reloc,
    x86_64_relocation_expression,
};
