use peony_layout::Layout;
use peony_object::{Binding, InputObject};
use peony_symbols::{SymbolId, SymbolTable};

use crate::{TlsRef, r_x86_64};

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
pub(crate) fn tls_ref_offset(
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
/// whether GD/LD/TLSDESC are kept or relaxed; imported Initial-Exec TLS remains
/// loader-filled even in an executable because the provider DSO owns the TLS.
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
        if imported {
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
pub(crate) fn tls_ref_dynidx(tref: TlsRef, symbols: &SymbolTable) -> u32 {
    if let TlsRef::Global(id) = tref {
        if let Some(name) = symbols.name_by_id(id) {
            if let Some(r) = symbols.lookup(name) {
                return r.dynsym_index;
            }
        }
    }
    0
}

/// Count TLS dynamic relocations to size `.rela.dyn` before layout. Shared
/// objects need GD/LD/TLSDESC plus IE entries; executables still need TPOFF64
/// entries for imported Initial-Exec TLS slots.
pub fn count_tls_relocs(
    objects: &[InputObject],
    symbols: &SymbolTable,
    tls: &peony_layout::TlsGotInfo,
    shared: bool,
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
    if !shared {
        return tls.ie.len();
    }
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
