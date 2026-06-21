//! `peony-reloc` — Relocation scanning and application (x86-64 ELF).
//!
//! Implements MaskRay's pass 6 (scan → which GOT slots are needed) and the
//! relocation-application part of pass 9.
//!
//! ## Symbol address resolution
//!
//! * **Global / weak** symbols are resolved through the global [`SymbolTable`]
//!   (their final VA / GOT address were written back by
//!   `peony_layout::finalize_symbols`).
//! * **Local / section** symbols are not in the global table; their address is
//!   computed directly from the defining object's section placement via
//!   [`Layout::address_of`].
//!
//! ## Static linking note
//!
//! For a fully-resolved static executable a `PLT32` reference to a *defined*
//! symbol is resolved directly to the symbol (identical to `PC32`); no PLT stub
//! is synthesised. GOT-relative references still allocate a GOT slot holding the
//! symbol's absolute address.

use thiserror::Error;

mod apply;
mod copy;
mod dynamic;
mod scan;
mod tls;
mod types;

pub use apply::{ApplyCtx, SymIndex, apply_reloc};
pub use copy::copy_reloc_symbols;
pub use dynamic::{
    DynamicRelocCounts,
    collect_data_and_symbolic_relocs,
    collect_dynamic_data_relocs,
    collect_symbolic_data_relocs,
    count_dynamic_relocs,
    count_relative,
};
pub use scan::{assign_weak_got_ids, scan_relocations};
pub use tls::{
    TlsDynReloc,
    TlsGotContents,
    collect_tls_got,
    count_got_relative,
    count_irelative,
    count_tls_relocs,
};
pub use types::{RelocScanResult, SyntheticSlot, TlsRef, r_x86_64};
pub(crate) use types::{is_tls, may_need_copy_reloc, needs_got};

#[cfg(test)]
mod apply_reloc_address_tests;
#[cfg(test)]
mod patch_buf_tests;

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RelocError {
    #[error("undefined symbol `{name}` referenced in `{object}`")]
    UndefinedSymbol { name: String, object: String },
    #[error(
        "relocation overflow in `{object}` at offset {offset:#x}: value {value} out of range for type {r_type}"
    )]
    Overflow {
        object: String,
        offset: u64,
        value: i64,
        r_type: u32,
    },
}

pub type Result<T> = std::result::Result<T, RelocError>;
