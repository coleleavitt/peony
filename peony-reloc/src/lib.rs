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

use peony_layout::Layout;
use peony_object::{Binding, InputObject, InputReloc};
use peony_symbols::{SymbolId, SymbolTable};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use thiserror::Error;

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
fn is_tls(r_type: u32) -> bool {
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
fn needs_got(r_type: u32) -> bool {
    matches!(
        r_type,
        r_x86_64::GOT32 | r_x86_64::GOTPCREL | r_x86_64::GOTPCRELX | r_x86_64::REX_GOTPCRELX
    )
}

/// True for relocation types that directly encode an imported symbol's address
/// into executable-owned storage/code. Imported data with these relocations needs
/// a copy relocation; GOT/PLT/TLS/SIZE relocs have their own mechanisms.
fn may_need_copy_reloc(r_type: u32) -> bool {
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

    fn add(&mut self, slot: SyntheticSlot) {
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

/// Assign real [`SymbolId`]s to weak-undefined symbols that are referenced via
/// the GOT, so their slots are tracked and get a recorded address (holding 0).
/// Must run on `&mut symbols` before [`scan_relocations`].
pub fn assign_weak_got_ids(objects: &[InputObject], symbols: &mut SymbolTable) {
    for obj in objects {
        for sec in &obj.sections {
            for reloc in &sec.relocs {
                if !needs_got(reloc.r_type) {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                // Only weak, currently-undefined, non-import symbols need this.
                let is_weak_undef = matches!(
                    symbols.lookup(&sym.name),
                    Some(r) if !r.is_defined() && !r.import && sym.binding == Binding::Weak
                );
                if is_weak_undef {
                    symbols.ensure_id(&sym.name);
                }
            }
        }
    }
}

/// Imported DSO data symbols that need executable-owned storage plus
/// `R_X86_64_COPY` because an allocated input section directly references them.
pub fn copy_reloc_symbols(objects: &[InputObject], symbols: &SymbolTable) -> Vec<SymbolId> {
    let mut out = FxHashMap::<SymbolId, Vec<u8>>::default();
    for obj in objects {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            // A COPY reloc is only needed when the reference lives in a section
            // the loader CANNOT relocate at runtime — i.e. a read-only allocated
            // section (`.text`, `.rodata`). A reference from a WRITABLE section
            // (`.data`, `.data.rel.*`) is resolved by a symbolic dynamic
            // relocation instead (see `collect_symbolic_data_relocs`); making it
            // a COPY would wrongly duplicate the DSO object (e.g. a C++ typeinfo),
            // breaking address-identity comparisons during exception unwinding.
            if sec.flags & peony_object::elf::SHF_WRITE != 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if !may_need_copy_reloc(reloc.r_type) {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                if sym.binding == Binding::Local {
                    continue;
                }
                // Never emit COPY relocs for C++ vtables (`_ZTV*`), VTT
                // (`_ZTT*`), or typeinfo-name (`_ZTS*`) symbols. Vtables are
                // referenced indirectly (GOT/relative) — copying them into the
                // executable would create a duplicate that breaks the program-
                // wide identity those objects rely on.
                //
                // Note: typeinfo objects (`_ZTI*`) are deliberately NOT skipped.
                // When `.text` takes a PC-relative reference to a DSO typeinfo
                // object (`R_X86_64_PC32`), a COPY reloc is the only ABI-correct
                // resolution (it binds the executable and the DSO to one copy),
                // matching what mold/bfd/gold emit. Skipping it would leave the
                // reference resolving to address 0.
                if sym.name.starts_with(b"_ZTV")
                    || sym.name.starts_with(b"_ZTT")
                    || sym.name.starts_with(b"_ZTS")
                {
                    continue;
                }
                let Some(res) = symbols.lookup(&sym.name) else {
                    continue;
                };
                if !res.import {
                    continue;
                }
                // Protected/hidden definitions cannot be preempted by a COPY
                // reloc in the executable, so they are never copy-reloc eligible.
                if res.visibility == peony_object::elf::STV_HIDDEN
                    || res.visibility == peony_object::elf::STV_PROTECTED
                {
                    continue;
                }
                // COPY relocations are for DSO data objects. Avoid turning direct
                // function calls into copy relocations if an assembler emitted PC32.
                if res.st_type == peony_object::elf::STT_FUNC
                    || res.st_type == peony_object::elf::STT_GNU_IFUNC
                    || res.st_type == peony_object::elf::STT_TLS
                {
                    continue;
                }
                if res.size == 0 && res.st_type != peony_object::elf::STT_OBJECT {
                    continue;
                }
                out.entry(res.id).or_insert_with(|| sym.name.clone());
            }
        }
    }
    let mut out: Vec<(SymbolId, Vec<u8>)> = out.into_iter().collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out.into_iter().map(|(id, _)| id).collect()
}

// ── Scan phase (parallel) ────────────────────────────────────────────────────

/// Scan all relocations to determine the required GOT and PLT slots. `shared`
/// selects shared-object TLS handling (GD/LD/IE GOT slots + `__tls_get_addr`
/// PLT) instead of the executable's GD/LD→Local-Exec relaxation.
pub fn scan_relocations(
    objects: &[InputObject],
    symbols: &SymbolTable,
    shared: bool,
) -> RelocScanResult {
    // Small links scan faster serially: spinning up rayon's global pool for a
    // few objects costs more in thread management (`sched_yield`/`futex`) than
    // the scan itself. Fan out only once there are enough objects to amortize.
    // See the parse-threshold note in main.rs: touching rayon's global pool on a
    // small link costs more in futex/sched_yield idle-spin than the scan saves.
    const PARALLEL_SCAN_THRESHOLD: usize = 256;
    let per_object: Vec<Vec<SyntheticSlot>> = if objects.len() >= PARALLEL_SCAN_THRESHOLD {
        objects
            .par_iter()
            .enumerate()
            .map(|(obj_id, obj)| scan_object(obj, obj_id, symbols, shared))
            .collect()
    } else {
        objects
            .iter()
            .enumerate()
            .map(|(obj_id, obj)| scan_object(obj, obj_id, symbols, shared))
            .collect()
    };

    let mut result = RelocScanResult::new();
    for slots in per_object {
        for slot in slots {
            result.add(slot);
        }
    }
    result
}

fn scan_object(
    obj: &InputObject,
    obj_id: usize,
    symbols: &SymbolTable,
    shared: bool,
) -> Vec<SyntheticSlot> {
    let mut slots = Vec::new();
    for sec in &obj.sections {
        for reloc in &sec.relocs {
            let Some(sym_pos) = obj.symbol_map.get(&reloc.symbol.0).copied() else {
                continue;
            };
            let Some(sym) = obj.symbols.get(sym_pos) else {
                continue;
            };
            // A helper to key a TLS reference (locals are file-scoped).
            let tls_ref = |sym: &peony_object::InputSymbol| -> TlsRef {
                if sym.binding == Binding::Local {
                    TlsRef::Local(obj_id, sym_pos)
                } else {
                    match symbols.lookup(&sym.name) {
                        Some(r) => TlsRef::Global(r.id),
                        None => TlsRef::Local(obj_id, sym_pos),
                    }
                }
            };
            // Initial-Exec (`GOTTPOFF`) ALWAYS needs a GOT slot holding the TP
            // offset — in BOTH an executable (filled statically, the offset is
            // known) and a shared object (filled at load via R_X86_64_TPOFF64).
            // The `mov x@gottpoff(%rip),%reg` access is kept in both cases.
            if reloc.r_type == r_x86_64::GOTTPOFF {
                slots.push(SyntheticSlot::TlsIe(tls_ref(sym)));
                continue;
            }
            // General-/Local-Dynamic: only a shared object keeps them (an
            // executable relaxes GD/LD → Local-Exec, needing no GOT slot).
            if shared && matches!(reloc.r_type, r_x86_64::TLSGD | r_x86_64::TLSLD) {
                match reloc.r_type {
                    r_x86_64::TLSGD => slots.push(SyntheticSlot::TlsGd(tls_ref(sym))),
                    r_x86_64::TLSLD => slots.push(SyntheticSlot::TlsLdm),
                    _ => unreachable!(),
                }
                continue;
            }
            if shared && reloc.r_type == r_x86_64::GOTPC32_TLSDESC {
                slots.push(SyntheticSlot::TlsDesc(tls_ref(sym)));
                continue;
            }

            // Slots are keyed by global SymbolId; local refs are handled directly.
            let Some(res) = symbols.lookup(&sym.name) else {
                continue;
            };
            // A GOT reference needs a slot whenever the symbol is defined, an
            // import, OR a weak-undefined reference (whose slot holds 0 so code
            // like `mov gmon@GOTPCREL,%rax; test %rax,%rax` correctly sees null).
            if needs_got(reloc.r_type)
                && (res.is_defined() || res.import || res.binding == Binding::Weak)
            {
                slots.push(SyntheticSlot::Got(res.id));
            } else if reloc.r_type == r_x86_64::PLT32 {
                // A direct `call foo@PLT` to an imported function needs a PLT
                // stub. In a shared object `__tls_get_addr` is a real import
                // (the GD/LD call is kept), so it DOES need a PLT slot; in an
                // executable the GD/LD calls are relaxed away, so it does not.
                let tls_helper = sym.name == b"__tls_get_addr";
                if res.import && (shared || !tls_helper) {
                    slots.push(SyntheticSlot::Plt(res.id));
                }
            }
        }
    }
    slots
}

/// Count the base-relative R64 data relocations a PIE will need, without
/// requiring final addresses. Used (with [`count_got_relative`]) to size
/// `.rela.dyn` before layout. This is the COMBINED count of plain RELATIVE and
/// IFUNC (IRELATIVE) R64 sites — i.e. it matches the union of the RELATIVE and
/// IRELATIVE lists returned by [`collect_dynamic_data_relocs`]; use
/// [`count_irelative`] to recover the IFUNC subset.
pub fn count_relative(objects: &[InputObject], symbols: &SymbolTable) -> usize {
    let mut n = 0;
    for obj in objects {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if reloc.r_type != r_x86_64::R64 {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                let counts = if sym.binding == Binding::Local {
                    // Local defined in a kept, allocated section.
                    sym.section.is_some()
                } else {
                    symbols
                        .lookup(&sym.name)
                        .is_some_and(|r| r.is_defined() && (!r.import || r.copy_reloc))
                };
                if counts {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Count the symbolic `R_X86_64_64` dynamic relocations needed for `R64` sites
/// in allocated, writable data that reference an *imported* symbol (one defined
/// only in a shared library, with no COPY reloc of its own). The loader writes
/// `*site = sym_address + addend`. These are exactly the R64 sites that
/// [`count_relative`]/[`collect_dynamic_data_relocs`] deliberately skip (they
/// only handle in-image targets). gcc emits these for `.data.rel.local.DW.ref.*`
/// EH "personality / typeinfo reference" slots. Used to size `.rela.dyn` before
/// layout, so it must agree with [`collect_symbolic_data_relocs`].
pub fn count_symbolic_data_relocs(objects: &[InputObject], symbols: &SymbolTable) -> usize {
    let mut n = 0;
    for obj in objects {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if reloc.r_type != r_x86_64::R64 {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                if sym.binding == Binding::Local {
                    continue;
                }
                // Imported (DSO-defined) symbol with no copy reloc → needs a
                // symbolic R64; an in-image / copy-reloc target is RELATIVE.
                if symbols
                    .lookup(&sym.name)
                    .is_some_and(|r| r.import && !r.copy_reloc)
                {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Collect the symbolic `R_X86_64_64` dynamic relocations (see
/// [`count_symbolic_data_relocs`]). Returns `(site_vaddr, dynsym_index, addend)`
/// for each `R64` site referencing an imported symbol. Must run after layout so
/// section VAs and `dynsym_index` are final.
pub fn collect_symbolic_data_relocs(
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
) -> Vec<(u64, u32, i64)> {
    let mut out = Vec::new();
    for (obj_id, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if reloc.r_type != r_x86_64::R64 {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                if sym.binding == Binding::Local {
                    continue;
                }
                let Some(res) = symbols.lookup(&sym.name) else {
                    continue;
                };
                if !res.import || res.copy_reloc || res.dynsym_index == 0 {
                    continue;
                }
                let Some(site_va) = layout.address_of(obj_id, sec.index.0) else {
                    continue;
                };
                out.push((site_va + reloc.offset, res.dynsym_index, reloc.addend));
            }
        }
    }
    out.sort_unstable();
    out
}

// ── Dynamic-base (RELATIVE) relocation collection ────────────────────────────

/// For a position-independent executable, collect the `R_X86_64_RELATIVE`
/// dynamic relocations: every `R_X86_64_64` site that targets a symbol defined
/// *in this image* (not a dynamic import) holds an absolute address the loader
/// must bias by the load base. We return `(site_vaddr, target_vaddr)` where
/// `target_vaddr = S + A` is the link-time value (which becomes the `r_addend`,
/// since the loader computes `*site = base + addend`).
///
/// Must run after [`peony_layout::finalize_symbols`] so symbol VAs are final.
pub fn collect_relative(
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
) -> Vec<(u64, u64)> {
    let (relative, _ifunc) = collect_dynamic_data_relocs(objects, symbols, layout);
    relative
}

/// Collect data (`R_X86_64_64`) dynamic relocations, partitioned into plain
/// `R_X86_64_RELATIVE` targets and `R_X86_64_IRELATIVE` (IFUNC) targets. An R64
/// site against an IFUNC must run the resolver, so it becomes IRELATIVE with the
/// resolver address as the addend; all other defined-symbol R64 sites are
/// RELATIVE. Returns `(relative, irelative)` as `(site_va, value_va)` pairs.
pub fn collect_dynamic_data_relocs(
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
) -> (Vec<(u64, u64)>, Vec<(u64, u64)>) {
    let mut out = Vec::new();
    let mut ifunc = Vec::new();
    for (obj_id, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            // Only allocated sections land in the loaded image.
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if reloc.r_type != r_x86_64::R64 {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                // Resolve the target VA exactly as the apply phase will, and note
                // whether the target is an IFUNC (→ IRELATIVE, not RELATIVE).
                let (target, is_ifunc) = if sym.binding == Binding::Local {
                    match sym.section.and_then(|si| layout.address_of(obj_id, si.0)) {
                        Some(va) => (va + sym.value, sym.is_ifunc),
                        None => continue, // absolute or dropped — no base bias needed
                    }
                } else {
                    match symbols.lookup(&sym.name) {
                        // Normal imports are handled by GLOB_DAT, not RELATIVE.
                        // COPY imports have executable-owned storage and therefore
                        // behave like output-defined data for base-bias purposes.
                        Some(r) if r.is_defined() && (!r.import || r.copy_reloc) => {
                            (r.virtual_address, r.is_ifunc)
                        }
                        _ => continue,
                    }
                };
                let Some(site_va) = layout.address_of(obj_id, sec.index.0) else {
                    continue;
                };
                let site = site_va + reloc.offset;
                if is_ifunc {
                    // IRELATIVE r_addend must be the bare resolver address: the
                    // loader computes `*site = resolver(base + addend)`. Folding
                    // the input reloc addend in would call into the middle of the
                    // resolver. (A non-zero addend on an IFUNC R64 is not
                    // meaningful; ignore it.)
                    ifunc.push((site, target));
                } else {
                    let value = (target as i64).wrapping_add(reloc.addend) as u64;
                    out.push((site, value));
                }
            }
        }
    }

    // GOT slots holding the address of a locally-defined (non-import) symbol also
    // need a dynamic relocation in a PIE: a plain RELATIVE for a normal symbol
    // (the slot holds an absolute VA the loader biases), or an IRELATIVE for an
    // IFUNC (the loader runs the resolver). Import GOT slots use GLOB_DAT.
    for (i, id) in layout.got_slots.iter().enumerate() {
        let Some(name) = symbols.name_by_id(*id) else {
            continue;
        };
        let Some(res) = symbols.lookup(name) else {
            continue;
        };
        if (res.import && !res.copy_reloc) || !res.is_defined() {
            continue;
        }
        let site = layout.got_base + (i as u64) * 8;
        if res.is_ifunc {
            ifunc.push((site, res.virtual_address));
        } else {
            out.push((site, res.virtual_address));
        }
    }

    // Deterministic order (by site) for reproducible output + DT_RELACOUNT runs.
    out.sort_unstable();
    ifunc.sort_unstable();
    (out, ifunc)
}

/// One TLS dynamic relocation for `.rela.dyn`: `(site_va, r_type, sym_index,
/// addend)`. `sym_index` is the `.dynsym` index (0 for module-relative locals).
pub type TlsDynReloc = (u64, u32, u32, i64);

/// The TLS GOT region's filled contents for a shared object:
/// * `relocs` — dynamic relocations (DTPMOD64 on each GD/LDM slot0, TPOFF64 on
///   each IE slot, and DTPOFF64 for any *imported* GD symbol).
/// * `static_writes` — `(got_va, value)` bytes written directly into `.got`
///   (the DTPOFF in a locally-defined GD pair's slot1, and the LDM slot1 = 0).
#[derive(Debug, Default)]
pub struct TlsGotContents {
    pub relocs: Vec<TlsDynReloc>,
    pub static_writes: Vec<(u64, u64)>,
}

/// Resolve a [`TlsRef`]'s offset within this module's TLS block, plus whether it
/// is an *imported* (undefined-here) TLS symbol. Returns `None` if it cannot be
/// resolved (e.g. dropped section).
fn tls_ref_offset(
    tref: TlsRef,
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
) -> Option<(u64, bool)> {
    match tref {
        TlsRef::Local(obj_id, sym_pos) => {
            let sym = objects.get(obj_id)?.symbols.get(sym_pos)?;
            let si = sym.section?;
            let base = layout.tls_offset(obj_id, si.0)?;
            Some((base + sym.value, false))
        }
        TlsRef::Global(id) => {
            let name = symbols.name_by_id(id)?;
            let res = symbols.lookup(name)?;
            match (res.defined_in, res.section_index) {
                (Some(def), Some(si)) => {
                    let base = layout.tls_offset(def.0 as usize, si)?;
                    Some((base + res.value, false))
                }
                // Imported / undefined-here TLS symbol: offset filled by loader.
                _ => Some((0, true)),
            }
        }
    }
}

/// Build the TLS GOT region's dynamic relocations and static slot writes from
/// the addresses the layout assigned. Must run post-layout. `shared` selects
/// dynamic (GD/LD/IE with loader-filled module ids / TP offsets) vs the
/// executable case (Initial-Exec slots filled statically; GD/LD are relaxed
/// away in the executable and produce no GOT pairs here).
pub fn collect_tls_got(
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    tls: &peony_layout::TlsGotInfo,
    shared: bool,
) -> TlsGotContents {
    let mut c = TlsGotContents::default();
    if shared {
        // General-Dynamic pairs: slot0 = DTPMOD64, slot1 = DTPOFF.
        for tref in &tls.gd {
            let Some(&pair) = layout.tls_gd_addr.get(tref) else {
                continue;
            };
            let (offset, imported) =
                tls_ref_offset(*tref, objects, symbols, layout).unwrap_or((0, true));
            if imported {
                // Imported TLS: both module id and offset are resolved at load.
                let dynidx = tls_ref_dynidx(*tref, symbols);
                c.relocs.push((pair, r_x86_64::DTPMOD64, dynidx, 0));
                c.relocs.push((pair + 8, r_x86_64::DTPOFF64, dynidx, 0));
            } else {
                // Locally-defined: DTPMOD64 (module, loader-filled) + static DTPOFF.
                c.relocs.push((pair, r_x86_64::DTPMOD64, 0, 0));
                c.static_writes.push((pair + 8, offset));
            }
        }
        // TLSDESC pairs: one dynamic relocation fills the two-word descriptor.
        for tref in &tls.desc {
            let Some(&pair) = layout.tls_desc_addr.get(tref) else {
                continue;
            };
            let (offset, imported) =
                tls_ref_offset(*tref, objects, symbols, layout).unwrap_or((0, true));
            let dynidx = if imported {
                tls_ref_dynidx(*tref, symbols)
            } else {
                0
            };
            let addend = if imported { 0 } else { offset as i64 };
            c.relocs.push((pair, r_x86_64::TLSDESC, dynidx, addend));
        }
        // Module Local-Dynamic pair: slot0 = DTPMOD64, slot1 = 0 (static).
        if let Some(ldm) = layout.tls_ldm_addr {
            c.relocs.push((ldm, r_x86_64::DTPMOD64, 0, 0));
            c.static_writes.push((ldm + 8, 0));
        }
    }
    // Initial-Exec slots hold the TP-relative offset of the symbol.
    for tref in &tls.ie {
        let Some(&slot) = layout.tls_ie_addr.get(tref) else {
            continue;
        };
        let (offset, imported) =
            tls_ref_offset(*tref, objects, symbols, layout).unwrap_or((0, true));
        if shared && imported {
            // Imported IE TLS: loader fills the slot (TPOFF64 against the symbol).
            let dynidx = tls_ref_dynidx(*tref, symbols);
            c.relocs.push((slot, r_x86_64::TPOFF64, dynidx, 0));
        } else if shared {
            // Locally-defined IE in a .so: loader fills the slot (module-relative
            // addend) since the TP offset isn't known until load.
            c.relocs.push((slot, r_x86_64::TPOFF64, 0, offset as i64));
        } else {
            // EXECUTABLE: the TP offset is fixed (TP is the end of the static TLS
            // block), so write it statically — `offset - tls_size`, negative.
            let tp = (offset as i64).wrapping_sub(layout.tls_size as i64);
            c.static_writes.push((slot, tp as u64));
        }
    }
    c
}

/// `.dynsym` index for an imported TLS symbol (0 if not an import / unknown).
fn tls_ref_dynidx(tref: TlsRef, symbols: &SymbolTable) -> u32 {
    if let TlsRef::Global(id) = tref {
        if let Some(name) = symbols.name_by_id(id) {
            if let Some(r) = symbols.lookup(name) {
                return r.dynsym_index;
            }
        }
    }
    0
}

/// Count the TLS dynamic relocations a shared object's TLS GOT will need, to
/// size `.rela.dyn` before layout. Each GD pair contributes 1 (local) or 2
/// (imported), each TLSDESC pair 1, the LDM pair 1, each IE slot 1.
pub fn count_tls_relocs(
    objects: &[InputObject],
    symbols: &SymbolTable,
    tls: &peony_layout::TlsGotInfo,
) -> usize {
    let imported = |tref: &TlsRef| -> bool {
        match tref {
            TlsRef::Local(..) => false,
            TlsRef::Global(id) => symbols
                .name_by_id(*id)
                .and_then(|n| symbols.lookup(n))
                .map(|r| r.defined_in.is_none())
                .unwrap_or(true),
        }
    };
    let _ = objects;
    let gd: usize = tls.gd.iter().map(|t| if imported(t) { 2 } else { 1 }).sum();
    let desc = tls.desc.len();
    let ie = tls.ie.len();
    let ldm = usize::from(tls.ldm);
    gd + desc + ie + ldm
}

/// Count GOT slots that will need an `R_X86_64_RELATIVE` (defined non-import
/// symbols referenced through the GOT in a PIE). Added to [`count_relative`] by
/// the driver since GOT slots are known before layout. Includes IFUNC GOT slots
/// (which become IRELATIVE) so `.rela.dyn` is sized for the combined total; use
/// [`count_irelative`] to find how many of those are IFUNCs.
pub fn count_got_relative(got_syms: &[SymbolId], symbols: &SymbolTable) -> usize {
    got_syms
        .iter()
        .filter(|id| {
            symbols
                .name_by_id(**id)
                .and_then(|n| symbols.lookup(n))
                .is_some_and(|r| (!r.import || r.copy_reloc) && r.is_defined())
        })
        .count()
}

/// Count the dynamic relocations that will be `R_X86_64_IRELATIVE` (IFUNC
/// targets): R64 data references to a defined IFUNC plus IFUNC GOT slots. This
/// is the subset of [`count_relative`] + [`count_got_relative`] that is NOT
/// plain RELATIVE, so the driver can gate `DT_RELACOUNT` on the true RELATIVE
/// count (`total - irelative`).
pub fn count_irelative(
    objects: &[InputObject],
    symbols: &SymbolTable,
    got_syms: &[SymbolId],
) -> usize {
    let mut n = 0;
    // R64 data references to a defined IFUNC.
    for obj in objects {
        for sec in &obj.sections {
            if sec.flags & peony_object::elf::SHF_ALLOC == 0 {
                continue;
            }
            for reloc in &sec.relocs {
                if reloc.r_type != r_x86_64::R64 {
                    continue;
                }
                let Some(sym) = obj
                    .symbols
                    .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                else {
                    continue;
                };
                let is_ifunc = if sym.binding == Binding::Local {
                    sym.section.is_some() && sym.is_ifunc
                } else {
                    symbols
                        .lookup(&sym.name)
                        .is_some_and(|r| r.is_defined() && !r.import && r.is_ifunc)
                };
                if is_ifunc {
                    n += 1;
                }
            }
        }
    }
    // IFUNC GOT slots.
    n += got_syms
        .iter()
        .filter(|id| {
            symbols
                .name_by_id(**id)
                .and_then(|n| symbols.lookup(n))
                .is_some_and(|r| !r.import && r.is_defined() && r.is_ifunc)
        })
        .count();
    n
}

// ── Apply phase ──────────────────────────────────────────────────────────────

/// Context for applying relocations.
#[derive(Copy, Clone)]
pub struct ApplyCtx<'a> {
    pub symbols: &'a SymbolTable,
    pub layout: &'a Layout,
    /// Producing a shared object: keep General-/Local-Dynamic TLS (GOT pairs +
    /// `__tls_get_addr`) instead of relaxing to Local-Exec.
    pub shared: bool,
}

/// Addresses used when computing a relocation value.
struct RelocAddrs {
    s: u64, // symbol VA
    a: i64, // addend
    p: u64, // place (relocation site) VA
    g: u64, // GOT entry VA for the symbol (0 if none)
    l: u64, // PLT stub VA (0 = resolve directly)
    z: u64, // symbol size
    got_base: u64,
    tls: u64,      // symbol's offset within the static TLS block
    tls_size: u64, // total static TLS block size
    offset: usize,
    shared: bool,  // producing a shared object (GD/LD/IE TLS, no LE relax)
    tls_gd: u64,   // GD GOT pair base VA for this symbol (shared); 0 if none
    tls_ie: u64,   // IE GOT slot VA for this symbol (shared); 0 if none
    tls_desc: u64, // TLSDESC GOT pair base VA for this symbol (shared); 0 if none
    tls_ldm: u64,  // module LDM GOT pair base VA (shared); 0 if none
}

/// Apply a single relocation, patching `buf` (the relocated section's bytes).
///
/// `obj_id` is the object's index (used to resolve local-symbol addresses).
/// `section_va` is the virtual address of the section start.
pub fn apply_reloc(
    ctx: &ApplyCtx<'_>,
    obj: &InputObject,
    obj_id: usize,
    reloc: &InputReloc,
    section_va: u64,
    buf: &mut [u8],
) -> Result<()> {
    if reloc.r_type == r_x86_64::NONE {
        return Ok(());
    }
    let Some(sym) = obj
        .symbols
        .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
    else {
        return Ok(());
    };

    // In an EXECUTABLE the `call __tls_get_addr@PLT` after a GD/LD `lea` is
    // rewritten by the TLSGD/TLSLD→LE relaxation, so its PLT32/PC32 relocation
    // must not run (it would corrupt the relaxed Local-Exec bytes). In a SHARED
    // object the call is KEPT, so the relocation must run normally (resolve the
    // real PLT stub) — do not skip it there.
    if !ctx.shared
        && matches!(reloc.r_type, r_x86_64::PLT32 | r_x86_64::PC32)
        && sym.name == b"__tls_get_addr"
    {
        return Ok(());
    }

    // Resolve the symbol's address (S), GOT slot (G), PLT (L), size (Z).
    let (s, g, l, z) = if sym.binding == Binding::Local {
        let s = match sym.section {
            Some(si) => match ctx.layout.address_of(obj_id, si.0) {
                Some(va) => va + sym.value,
                None => return Ok(()), // defined in a dropped section
            },
            None => sym.value, // absolute local
        };
        (s, 0, 0, sym.size)
    } else {
        match ctx.symbols.lookup(&sym.name) {
            Some(r) if r.is_defined() => (r.virtual_address, r.got_address, r.plt_address, r.size),
            // Weak-undefined: address resolves to 0, but a GOT reference still
            // uses the symbol's allocated GOT slot (which holds 0). Passing the
            // real `got_address` lets `mov sym@GOTPCREL,%rax; test %rax,%rax`
            // correctly observe null instead of dereferencing a bogus slot.
            Some(r) if sym.binding == Binding::Weak => (0, r.got_address, 0, 0),
            Some(_) if sym.binding == Binding::Weak => (0, 0, 0, 0),
            _ => {
                return Err(RelocError::UndefinedSymbol {
                    name: String::from_utf8_lossy(&sym.name).into_owned(),
                    object: obj.path.clone(),
                });
            }
        }
    };

    // For TLS relocations, compute the symbol's offset within the TLS block.
    let tls = if is_tls(reloc.r_type) {
        if sym.binding == Binding::Local {
            sym.section
                .and_then(|si| ctx.layout.tls_offset(obj_id, si.0))
                .map(|b| b + sym.value)
        } else {
            ctx.symbols.lookup(&sym.name).and_then(|r| {
                let def = r.defined_in?;
                r.section_index
                    .and_then(|si| ctx.layout.tls_offset(def.0 as usize, si))
                    .map(|b| b + r.value)
            })
        }
        .unwrap_or(0)
    } else {
        0
    };

    // TLS GOT addresses for this reference, keyed by `TlsRef` exactly as the
    // scan allocated them. IE (GOTTPOFF) slots exist in BOTH exe and shared
    // outputs; GD/LDM pairs only in a shared object.
    let (tls_gd, tls_ie, tls_desc) = {
        let sym_pos = obj.symbol_map.get(&reloc.symbol.0).copied();
        let tref = match sym_pos {
            Some(pos) if sym.binding == Binding::Local => Some(TlsRef::Local(obj_id, pos)),
            Some(pos) => match ctx.symbols.lookup(&sym.name) {
                Some(r) => Some(TlsRef::Global(r.id)),
                None => Some(TlsRef::Local(obj_id, pos)),
            },
            None => None,
        };
        let gd = tref
            .and_then(|t| ctx.layout.tls_gd_addr.get(&t).copied())
            .unwrap_or(0);
        let ie = tref
            .and_then(|t| ctx.layout.tls_ie_addr.get(&t).copied())
            .unwrap_or(0);
        let desc = tref
            .and_then(|t| ctx.layout.tls_desc_addr.get(&t).copied())
            .unwrap_or(0);
        (gd, ie, desc)
    };

    let addrs = RelocAddrs {
        s,
        a: reloc.addend,
        p: section_va + reloc.offset,
        g,
        l,
        z,
        got_base: ctx.layout.got_base,
        tls,
        tls_size: ctx.layout.tls_size,
        offset: reloc.offset as usize,
        shared: ctx.shared,
        tls_gd,
        tls_ie,
        tls_desc,
        tls_ldm: ctx.layout.tls_ldm_addr.unwrap_or(0),
    };

    patch_buf(buf, reloc.r_type, &addrs, &obj.path)
}

fn patch_buf(buf: &mut [u8], r_type: u32, a: &RelocAddrs, object: &str) -> Result<()> {
    use r_x86_64::*;
    let off = a.offset;
    let s = a.s as i64;
    let p = a.p as i64;
    match r_type {
        R64 => write_u64(buf, off, s.wrapping_add(a.a) as u64),
        PC64 => write_u64(buf, off, s.wrapping_add(a.a).wrapping_sub(p) as u64),
        GOTOFF64 => write_u64(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(a.got_base as i64) as u64,
        ),
        SIZE64 => write_u64(buf, off, (a.z as i64).wrapping_add(a.a) as u64),

        R32 => write_u32(buf, off, s.wrapping_add(a.a), r_type, object, off as u64)?,
        R32S => write_i32(buf, off, s.wrapping_add(a.a), r_type, object, off as u64)?,
        PC32 => write_i32(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(p),
            r_type,
            object,
            off as u64,
        )?,
        SIZE32 => write_u32(
            buf,
            off,
            (a.z as i64).wrapping_add(a.a),
            r_type,
            object,
            off as u64,
        )?,
        // PLT32: static link with a defined target resolves directly (== PC32).
        PLT32 => {
            let target = if a.l != 0 { a.l as i64 } else { s };
            write_i32(
                buf,
                off,
                target.wrapping_add(a.a).wrapping_sub(p),
                r_type,
                object,
                off as u64,
            )?
        }
        // GOT-relative: slot VA + A - P.
        GOTPCREL | GOTPCRELX | REX_GOTPCRELX => write_i32(
            buf,
            off,
            (a.g as i64).wrapping_add(a.a).wrapping_sub(p),
            r_type,
            object,
            off as u64,
        )?,
        // Offset of the symbol's slot within the GOT, + A.
        GOT32 => write_u32(
            buf,
            off,
            (a.g as i64)
                .wrapping_sub(a.got_base as i64)
                .wrapping_add(a.a),
            r_type,
            object,
            off as u64,
        )?,
        // GOT base relative to the place.
        GOTPC32 => write_i32(
            buf,
            off,
            (a.got_base as i64).wrapping_add(a.a).wrapping_sub(p),
            r_type,
            object,
            off as u64,
        )?,

        // Local-Exec TLS: offset from the thread pointer (TP at end of the block).
        TPOFF32 => {
            let v = (a.tls as i64)
                .wrapping_add(a.a)
                .wrapping_sub(a.tls_size as i64);
            tracing::trace!(
                tls_block_off = a.tls,
                addend = a.a,
                tls_size = a.tls_size,
                tpoff = v,
                "TPOFF32 (Local-Exec)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        TPOFF64 => write_u64(
            buf,
            off,
            (a.tls as i64)
                .wrapping_add(a.a)
                .wrapping_sub(a.tls_size as i64) as u64,
        ),
        // DTPOFF: offset of the symbol within its module's TLS block.
        //
        // In a SHARED object the matching `lea x@tlsld(%rip)` + `call
        // __tls_get_addr` is KEPT, and the helper returns the module's TLS block
        // base, so DTPOFF stays module-relative (`tls + addend`).
        //
        // In an EXECUTABLE peony relaxes the Local-Dynamic sequence to
        // Local-Exec (`mov %fs:0,%rax`), so the base register now holds the
        // thread pointer (the END of the static TLS block). The per-symbol
        // DTPOFF access must therefore become TP-relative (`tls + addend -
        // tls_size`, i.e. negative) to match — exactly like TPOFF. Leaving it
        // module-relative yields a positive `%fs:0 + off` that corrupts the TCB.
        DTPOFF32 => {
            let base = (a.tls as i64).wrapping_add(a.a);
            let v = if a.shared {
                base
            } else {
                base.wrapping_sub(a.tls_size as i64)
            };
            tracing::trace!(
                tls_block_off = a.tls,
                addend = a.a,
                tls_size = a.tls_size,
                shared = a.shared,
                dtpoff = v,
                "DTPOFF32"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        DTPOFF64 => {
            let base = (a.tls as i64).wrapping_add(a.a);
            let v = if a.shared {
                base
            } else {
                base.wrapping_sub(a.tls_size as i64)
            };
            write_u64(buf, off, v as u64)
        }

        // ── TLS access-model relaxation (executable ⇒ Local-Exec) ────────────
        //
        // In an executable the static TLS block is fixed, so General-Dynamic and
        // Local-Dynamic accesses are relaxed to Local-Exec, eliminating the
        // runtime `__tls_get_addr` call. We rewrite the fixed instruction
        // sequence the compiler emits and patch the TP-relative offset.
        // In a shared object the General-Dynamic access is KEPT (the static TLS
        // offset is unknown when dlopen'd): patch the `lea x@tlsgd(%rip),%rdi`
        // displacement to point at the symbol's GD GOT pair (GOTPCREL math) and
        // leave the `call __tls_get_addr@PLT` intact.
        TLSGD if a.shared => {
            let v = (a.tls_gd as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                gd_pair = format_args!("{:#x}", a.tls_gd),
                disp = v,
                "TLSGD (shared, General-Dynamic, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        TLSGD => {
            // GD→LE relaxation (x86-64 psABI). The input 16-byte sequence is:
            //   66 48 8d 3d <disp32>      data16 lea x@tlsgd(%rip),%rdi
            //   66 66 48 e8 <pc32>        data16 data16 rex.W call __tls_get_addr
            // The TLSGD reloc points at <disp32> (4 bytes into the lea), so the
            // sequence starts at off-4. Output (16 bytes):
            //   64 48 8b 04 25 00 00 00 00   mov %fs:0,%rax
            //   48 8d 80 <tpoff32>           lea x@tpoff(%rax),%rax
            let start = off.wrapping_sub(4);
            if start + 16 <= buf.len() {
                // R_X86_64_TLSGD is encoded as a PC-relative LEA displacement,
                // so assemblers commonly give it addend -4. The relaxed LE
                // immediate is no longer PC-relative; compensate for that
                // displacement addend just like lld's relaxTlsGdToLe.
                let le_off = (a.tls as i64)
                    .wrapping_add(a.a)
                    .wrapping_sub(a.tls_size as i64)
                    .wrapping_add(4) as i32;
                tracing::trace!(
                    off,
                    start,
                    orig = format_args!("{:02x?}", &buf[start..start + 16]),
                    tls_block_off = a.tls,
                    tls_size = a.tls_size,
                    le_off,
                    "TLSGD→LE relaxation"
                );
                // Verify the input matches the expected GD prologue before
                // rewriting (guards against a non-canonical sequence).
                if buf[start] != 0x66 || buf[start + 1] != 0x48 || buf[start + 2] != 0x8d {
                    tracing::warn!(
                        start,
                        got = format_args!("{:02x?}", &buf[start..start + 4]),
                        "TLSGD: unexpected prologue, skipping relaxation"
                    );
                } else {
                    buf[start..start + 16].copy_from_slice(&[
                        0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00,
                        0x00, // mov %fs:0,%rax
                        0x48, 0x8d, 0x80, 0, 0, 0, 0, // lea off(%rax),%rax
                    ]);
                    buf[start + 12..start + 16].copy_from_slice(&le_off.to_le_bytes());
                }
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSGD relaxation skipped: out of bounds"
                );
            }
        }
        // Shared object: Local-Dynamic kept — patch `lea x@tlsld(%rip),%rdi` to
        // the module LDM GOT pair; the per-symbol offset is added by the
        // (static) DTPOFF32 relocations. The `call __tls_get_addr` is kept.
        TLSLD if a.shared => {
            let v = (a.tls_ldm as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                ldm_pair = format_args!("{:#x}", a.tls_ldm),
                disp = v,
                "TLSLD (shared, Local-Dynamic, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        TLSLD => {
            // LD→LE relaxation. The input sequence is:
            //   48 8d 3d <disp32>     lea x@tlsld(%rip),%rdi   (reloc at off, lea at off-3)
            //   e8 <pc32>             call __tls_get_addr@plt  (at off+4)
            // GNU ld replaces the whole 12-byte span [off-3 .. off+9] with:
            //   66 66 66 64 48 8b 04 25 00 00 00 00   mov %fs:0,%rax (3 data16 prefixes pad)
            let start = off.wrapping_sub(3);
            if start + 12 <= buf.len() {
                tracing::trace!(
                    off,
                    start,
                    orig = format_args!("{:02x?}", &buf[start..start + 12]),
                    "TLSLD→LE relaxation"
                );
                buf[start..start + 12].copy_from_slice(&[
                    0x66, 0x66, 0x66, // data16 padding
                    0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, // mov %fs:0,%rax
                ]);
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSLD relaxation skipped: out of bounds"
                );
            }
        }
        // Initial-Exec, shared object: reference the dedicated IE GOT slot, which
        // the loader fills via an R_X86_64_TPOFF64 dynamic relocation (the TP
        // offset is unknown until load). `mov x@gottpoff(%rip),%reg` is kept.
        GOTTPOFF if a.shared => {
            let v = (a.tls_ie as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                ie_slot = format_args!("{:#x}", a.tls_ie),
                disp = v,
                "GOTTPOFF (shared, Initial-Exec, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        // Initial-Exec, executable: reference the dedicated IE GOT slot (filled
        // statically by `collect_tls_got` with the fixed TP offset). The
        // `mov x@gottpoff(%rip),%reg` access is kept; this patches its
        // displacement to the slot (GOTPCREL math). Using a real slot (not the
        // scalar GOT) is required — a missing slot would resolve to address 0.
        GOTTPOFF => {
            let v = (a.tls_ie as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                ie_slot = format_args!("{:#x}", a.tls_ie),
                disp = v,
                "GOTTPOFF (executable, Initial-Exec GOT slot)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        GOTPC32_TLSDESC if a.shared => {
            let v = (a.tls_desc as i64).wrapping_add(a.a).wrapping_sub(p);
            tracing::trace!(
                off,
                tlsdesc_pair = format_args!("{:#x}", a.tls_desc),
                disp = v,
                "GOTPC32_TLSDESC (shared, kept)"
            );
            write_i32(buf, off, v, r_type, object, off as u64)?
        }
        GOTPC32_TLSDESC => {
            // TLSDESC→LE relaxation for executables. The canonical input is:
            //   48 8d 05 <disp32>    lea x@tlsdesc(%rip),%rax
            //   ff 10                call *x@tlscall(%rax)
            // GNU ld rewrites the 9-byte pair to:
            //   48 c7 c0 <tpoff32>   mov $x@tpoff,%rax
            //   66 90                xchg %ax,%ax
            // The reloc points at <disp32>, so the LEA starts at off-3.
            let start = off.wrapping_sub(3);
            if start + 7 <= buf.len() {
                let le_off = (a.tls as i64)
                    .wrapping_add(a.a)
                    .wrapping_add(4)
                    .wrapping_sub(a.tls_size as i64);
                tracing::trace!(
                    off,
                    start,
                    orig = format_args!("{:02x?}", &buf[start..start + 7]),
                    tls_block_off = a.tls,
                    tls_size = a.tls_size,
                    le_off,
                    "TLSDESC→LE relaxation"
                );
                if buf[start] != 0x48 || buf[start + 1] != 0x8d {
                    tracing::warn!(
                        start,
                        got = format_args!("{:02x?}", &buf[start..start + 3]),
                        "TLSDESC: unexpected prologue, skipping relaxation"
                    );
                } else {
                    buf[start..start + 7].copy_from_slice(&[0x48, 0xc7, 0xc0, 0, 0, 0, 0]);
                    write_i32(buf, off, le_off, r_type, object, off as u64)?;
                }
            } else {
                tracing::warn!(
                    off,
                    buf_len = buf.len(),
                    "TLSDESC relaxation skipped: out of bounds"
                );
            }
        }
        TLSDESC_CALL => {
            // Marker relocation for `call *x@TLSCALL(%reg)`. Shared objects keep
            // the call; executables pair this with GOTPC32_TLSDESC→LE and turn
            // the two call bytes into GNU ld's canonical 2-byte NOP.
            if !a.shared {
                if off + 2 <= buf.len() {
                    buf[off..off + 2].copy_from_slice(&[0x66, 0x90]);
                } else {
                    tracing::warn!(
                        off,
                        buf_len = buf.len(),
                        "TLSDESC_CALL relaxation skipped: out of bounds"
                    );
                }
            }
        }

        R16 => write_u16(buf, off, s.wrapping_add(a.a), object, off as u64)?,
        PC16 => write_u16(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(p),
            object,
            off as u64,
        )?,
        R8 => write_u8(buf, off, s.wrapping_add(a.a), object, off as u64)?,
        PC8 => write_u8(
            buf,
            off,
            s.wrapping_add(a.a).wrapping_sub(p),
            object,
            off as u64,
        )?,

        other => {
            tracing::warn!(r_type = other, %object, "unhandled relocation type — skipping");
        }
    }
    Ok(())
}

// ── Width-checked writers ────────────────────────────────────────────────────

fn write_u64(buf: &mut [u8], off: usize, val: u64) {
    buf[off..off + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_i32(
    buf: &mut [u8],
    off: usize,
    val: i64,
    r_type: u32,
    object: &str,
    reloc_off: u64,
) -> Result<()> {
    let v = val as i32;
    if v as i64 != val {
        return Err(overflow(object, reloc_off, val, r_type));
    }
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn write_u32(
    buf: &mut [u8],
    off: usize,
    val: i64,
    r_type: u32,
    object: &str,
    reloc_off: u64,
) -> Result<()> {
    // Accept either zero-extended or sign-extended 32-bit values.
    let u = val as u64;
    if (u as u32) as u64 != u && (val as i32) as i64 != val {
        return Err(overflow(object, reloc_off, val, r_type));
    }
    buf[off..off + 4].copy_from_slice(&(val as u32).to_le_bytes());
    Ok(())
}

fn write_u16(buf: &mut [u8], off: usize, val: i64, object: &str, reloc_off: u64) -> Result<()> {
    if (val as i16) as i64 != val && (val as u16) as i64 != val {
        return Err(overflow(object, reloc_off, val, r_x86_64::R16));
    }
    buf[off..off + 2].copy_from_slice(&(val as u16).to_le_bytes());
    Ok(())
}

fn write_u8(buf: &mut [u8], off: usize, val: i64, object: &str, reloc_off: u64) -> Result<()> {
    if (val as i8) as i64 != val && (val as u8) as i64 != val {
        return Err(overflow(object, reloc_off, val, r_x86_64::R8));
    }
    buf[off] = val as u8;
    Ok(())
}

fn overflow(object: &str, offset: u64, value: i64, r_type: u32) -> RelocError {
    RelocError::Overflow {
        object: object.to_owned(),
        offset,
        value,
        r_type,
    }
}

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
        };

        patch_buf(&mut buf, r_x86_64::TLSDESC_CALL, &call_addrs, "test.o").unwrap();
        patch_buf(&mut buf, r_x86_64::GOTPC32_TLSDESC, &addrs, "test.o").unwrap();

        assert_eq!(&buf[0..3], &[0x48, 0xc7, 0xc0]);
        assert_eq!(i32::from_le_bytes(buf[3..7].try_into().unwrap()), -4);
        assert_eq!(&buf[7..9], &[0x66, 0x90]);
        assert_eq!(buf[9], 0x90);
    }
}
