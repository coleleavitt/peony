use peony_symbols::SymbolId;
use rustc_hash::FxHashMap;

// ── x86-64 relocation type constants ─────────────────────────────────────────

pub mod r_x86_64 {
    pub const NONE: u32 = 0;
    pub const R64: u32 = 1;
    pub const PC32: u32 = 2;
    pub const GOT32: u32 = 3;
    pub const PLT32: u32 = 4;
    pub const GOTPCREL: u32 = 9;
    pub const R32: u32 = 10;
    pub const R32S: u32 = 11;
    pub const R16: u32 = 12;
    pub const PC16: u32 = 13;
    pub const R8: u32 = 14;
    pub const PC8: u32 = 15;
    pub const DTPMOD64: u32 = 16; // TLS module id (GD/LDM GOT slot0), loader-filled
    pub const DTPOFF64: u32 = 17;
    pub const TPOFF64: u32 = 18;
    pub const TLSGD: u32 = 19; // General-Dynamic: relaxed to Local-Exec in an exe
    pub const TLSLD: u32 = 20; // Local-Dynamic: relaxed to Local-Exec in an exe
    pub const DTPOFF32: u32 = 21;
    pub const GOTTPOFF: u32 = 22; // Initial-Exec GOT slot with the TP offset
    pub const TPOFF32: u32 = 23;
    pub const PC64: u32 = 24;
    pub const GOTOFF64: u32 = 25;
    pub const GOTPC32: u32 = 26;
    pub const SIZE32: u32 = 32;
    pub const SIZE64: u32 = 33;
    pub const GOTPC32_TLSDESC: u32 = 34;
    pub const TLSDESC_CALL: u32 = 35;
    pub const TLSDESC: u32 = 36;
    pub const GOTPCRELX: u32 = 41;
    pub const REX_GOTPCRELX: u32 = 42;
}

/// True for the Local-Exec / Local-Dynamic TLS relocations we resolve statically.
pub(crate) fn is_tls(r_type: u32) -> bool {
    matches!(
        r_type,
        r_x86_64::TPOFF32
            | r_x86_64::TPOFF64
            | r_x86_64::DTPOFF32
            | r_x86_64::DTPOFF64
            | r_x86_64::TLSGD
            | r_x86_64::TLSLD
            | r_x86_64::GOTTPOFF
            | r_x86_64::GOTPC32_TLSDESC
            | r_x86_64::TLSDESC_CALL
            | r_x86_64::TLSDESC
    )
}

/// True for relocation types that reference a symbol through the GOT.
pub(crate) fn needs_got(r_type: u32) -> bool {
    matches!(
        r_type,
        r_x86_64::GOT32 | r_x86_64::GOTPCREL | r_x86_64::GOTPCRELX | r_x86_64::REX_GOTPCRELX
    )
}

/// True for relocation types that directly encode an imported symbol's address
/// into executable-owned storage/code. Imported data with these relocations needs
/// a copy relocation; GOT/PLT/TLS/SIZE relocs have their own mechanisms.
pub(crate) fn may_need_copy_reloc(r_type: u32) -> bool {
    matches!(
        r_type,
        r_x86_64::R64
            | r_x86_64::PC64
            | r_x86_64::R32
            | r_x86_64::R32S
            | r_x86_64::PC32
            | r_x86_64::R16
            | r_x86_64::PC16
            | r_x86_64::R8
            | r_x86_64::PC8
    )
}

// ── Synthetic slots ─────────────────────────────────────────────────────────

pub use peony_layout::TlsRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntheticSlot {
    Got(SymbolId),
    Plt(SymbolId),
    /// General-Dynamic TLS GOT *pair* (DTPMOD64 + DTPOFF) for a TLS symbol. Only
    /// allocated when producing a shared object (an executable relaxes GD→LE).
    TlsGd(TlsRef),
    /// Initial-Exec TLS GOT slot (GOTTPOFF → TPOFF64) for a TLS symbol, in a `.so`.
    TlsIe(TlsRef),
    /// The module's single Local-Dynamic (LDM) TLS GOT pair (DTPMOD64 + 0).
    TlsLdm,
    /// TLSDESC GOT pair for GNU2 TLS descriptors in a shared object.
    TlsDesc(TlsRef),
}

/// Result of the relocation scan: the GOT/PLT slots required, in stable order.
pub struct RelocScanResult {
    pub slots: Vec<SyntheticSlot>,
    pub slot_set: FxHashMap<SyntheticSlot, u64>,
}

impl RelocScanResult {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            slot_set: FxHashMap::default(),
        }
    }

    pub(crate) fn add(&mut self, slot: SyntheticSlot) {
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(v) = self.slot_set.entry(slot) {
            v.insert(0);
            self.slots.push(slot);
        }
    }

    /// The symbols needing a GOT slot, in slot order.
    pub fn got_symbols(&self) -> Vec<SymbolId> {
        self.slots
            .iter()
            .filter_map(|s| match s {
                SyntheticSlot::Got(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// The symbols needing a PLT entry (imported functions called via `@PLT`).
    pub fn plt_symbols(&self) -> Vec<SymbolId> {
        self.slots
            .iter()
            .filter_map(|s| match s {
                SyntheticSlot::Plt(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// TLS refs needing a General-Dynamic GOT pair, in slot order (shared).
    pub fn tls_gd_refs(&self) -> Vec<TlsRef> {
        self.slots
            .iter()
            .filter_map(|s| match s {
                SyntheticSlot::TlsGd(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    /// TLS refs needing an Initial-Exec GOT slot, in slot order (shared).
    pub fn tls_ie_refs(&self) -> Vec<TlsRef> {
        self.slots
            .iter()
            .filter_map(|s| match s {
                SyntheticSlot::TlsIe(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    /// Whether the module needs a Local-Dynamic (LDM) TLS GOT pair (shared).
    pub fn needs_tls_ldm(&self) -> bool {
        self.slots
            .iter()
            .any(|s| matches!(s, SyntheticSlot::TlsLdm))
    }

    /// TLS refs needing TLSDESC GOT pairs, in slot order.
    pub fn tls_desc_refs(&self) -> Vec<TlsRef> {
        self.slots
            .iter()
            .filter_map(|s| match s {
                SyntheticSlot::TlsDesc(r) => Some(*r),
                _ => None,
            })
            .collect()
    }
}

impl Default for RelocScanResult {
    fn default() -> Self {
        Self::new()
    }
}
