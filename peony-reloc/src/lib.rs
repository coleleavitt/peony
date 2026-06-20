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
#[cfg(test)]
use apply::{RelocAddrs, patch_buf};
pub use copy::copy_reloc_symbols;
pub use dynamic::{
    DynamicRelocCounts,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlsgd_to_local_exec_compensates_pc_relative_addend() {
        let mut buf = [
            0x66, 0x48, 0x8d, 0x3d, 0, 0, 0, 0, 0x66, 0x66, 0x48, 0xe8, 0, 0, 0, 0,
        ];
        let addrs = RelocAddrs {
            s: 0,
            a: -4,
            p: 4,
            g: 0,
            l: 0,
            z: 0,
            got_base: 0,
            tls: 0,
            tls_size: 0x140,
            offset: 4,
            shared: false,
            tls_gd: 0,
            tls_ie: 0,
            tls_desc: 0,
            tls_ldm: 0,
            tls_imported: false,
        };

        patch_buf(&mut buf, r_x86_64::TLSGD, &addrs, "test.o").unwrap();

        assert_eq!(
            &buf[0..12],
            &[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0x48, 0x8d, 0x80]
        );
        assert_eq!(i32::from_le_bytes(buf[12..16].try_into().unwrap()), -0x140);
    }

    #[test]
    fn tlsdesc_to_local_exec_matches_gnu_ld_sequence() {
        let mut buf = [
            0x48, 0x8d, 0x05, 0, 0, 0, 0, // lea x@tlsdesc(%rip),%rax
            0xff, 0x10, // call *x@tlscall(%rax)
            0x90,
        ];
        let addrs = RelocAddrs {
            s: 0,
            a: -4,
            p: 3,
            g: 0,
            l: 0,
            z: 0,
            got_base: 0,
            tls: 4,
            tls_size: 8,
            offset: 3,
            shared: false,
            tls_gd: 0,
            tls_ie: 0,
            tls_desc: 0,
            tls_ldm: 0,
            tls_imported: false,
        };

        let call_addrs = RelocAddrs {
            s: 0,
            a: 0,
            p: 7,
            g: 0,
            l: 0,
            z: 0,
            got_base: 0,
            tls: 4,
            tls_size: 8,
            offset: 7,
            shared: false,
            tls_gd: 0,
            tls_ie: 0,
            tls_desc: 0,
            tls_ldm: 0,
            tls_imported: false,
        };

        patch_buf(&mut buf, r_x86_64::TLSDESC_CALL, &call_addrs, "test.o").unwrap();
        patch_buf(&mut buf, r_x86_64::GOTPC32_TLSDESC, &addrs, "test.o").unwrap();

        assert_eq!(&buf[0..3], &[0x48, 0xc7, 0xc0]);
        assert_eq!(i32::from_le_bytes(buf[3..7].try_into().unwrap()), -4);
        assert_eq!(&buf[7..9], &[0x66, 0x90]);
        assert_eq!(buf[9], 0x90);
    }
}
