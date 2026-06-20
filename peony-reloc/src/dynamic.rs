use peony_layout::Layout;
use peony_object::{Binding, InputObject};
use peony_symbols::{SymbolId, SymbolTable};

use crate::{count_tls_relocs, r_x86_64};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DynamicRelocCounts {
    pub relative_total: usize,
    pub irelative: usize,
    pub symbolic_data: usize,
    pub tls: usize,
}

pub fn count_dynamic_relocs(
    objects: &[InputObject],
    symbols: &SymbolTable,
    got_syms: &[SymbolId],
    tls: &peony_layout::TlsGotInfo,
    shared: bool,
) -> DynamicRelocCounts {
    let mut counts = DynamicRelocCounts::default();
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
                    if sym.section.is_some() {
                        counts.relative_total += 1;
                        counts.irelative += usize::from(sym.is_ifunc);
                    }
                    continue;
                }
                let Some(res) = symbols.lookup(&sym.name) else {
                    continue;
                };
                if res.import && !res.copy_reloc {
                    counts.symbolic_data += 1;
                } else if res.is_defined() {
                    counts.relative_total += 1;
                    counts.irelative += usize::from(!res.import && res.is_ifunc);
                }
            }
        }
    }

    for id in got_syms {
        let Some(res) = symbols
            .name_by_id(*id)
            .and_then(|name| symbols.lookup(name))
        else {
            continue;
        };
        if (!res.import || res.copy_reloc) && res.is_defined() {
            counts.relative_total += 1;
            counts.irelative += usize::from(!res.import && res.is_ifunc);
        }
    }

    counts.tls = count_tls_relocs(objects, symbols, tls, shared);
    counts
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
#[allow(dead_code)]
pub(crate) fn count_symbolic_data_relocs(objects: &[InputObject], symbols: &SymbolTable) -> usize {
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
#[allow(dead_code)]
pub(crate) fn collect_relative(
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
