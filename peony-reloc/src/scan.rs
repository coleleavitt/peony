use peony_object::{Binding, InputObject};
use peony_symbols::SymbolTable;
use rayon::prelude::*;

use crate::{RelocScanResult, SyntheticSlot, TlsRef, needs_got, r_x86_64};

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
                if sym.binding != Binding::Weak {
                    continue;
                }
                // Only weak, currently-undefined, non-import symbols need this.
                let is_weak_undef = matches!(
                    symbols.lookup(&sym.name),
                    Some(r) if !r.is_defined() && !r.import
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
            if !shared
                && matches!(reloc.r_type, r_x86_64::TLSGD | r_x86_64::GOTPC32_TLSDESC)
                && sym.binding != Binding::Local
                && symbols.lookup(&sym.name).is_some_and(|r| r.import)
            {
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
