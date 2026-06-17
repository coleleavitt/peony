//! Identical Code Folding (ICF) — fold byte-and-relocation-identical `.text`
//! sections to a single copy, redirecting all references to the canonical one.
//!
//! Soundness (proved in `rocq-tests/ICFSoundness.v`): folding two functions is
//! observationally safe exactly when neither is *address-significant* (its
//! pointer identity is never compared). We honour that here by excluding any
//! section that defines a symbol whose address could be taken — conservatively,
//! every globally-visible function symbol, plus C++ vtable/typeinfo (`_ZTV`/
//! `_ZTI`/`_ZTS`) which the C++ ABI requires to be unique.
//!
//! This MVP is deliberately conservative — it folds only sections that are
//! *exactly* identical in bytes AND in their relocations' (offset, type, addend,
//! resolved target name). It does NOT do the iterative Merkle-hash fixpoint mold
//! uses (which folds mutually-recursive identical functions); it requires the
//! reloc *targets* to resolve to the same name, which already catches the common
//! case (duplicate monomorphisations, identical small helpers) with no chance of
//! an unsound merge. Anything uncertain is simply not folded.

use peony_object::{InputObject, SymbolIndex};
use rustc_hash::FxHashMap;

/// A canonical key identifying a foldable section's observable content: its raw
/// bytes plus a normalised view of its relocations (target *name*, not index, so
/// the same call across two objects hashes equal).
#[derive(PartialEq, Eq, Hash)]
struct FoldKey {
    flags: u64,
    data: Vec<u8>,
    /// (offset, r_type, addend, target-symbol-name) per relocation, in order.
    relocs: Vec<(u64, u32, i64, Vec<u8>)>,
}

/// Resolve a relocation's target symbol to its name bytes (stable across
/// objects). Returns `None` if the symbol can't be resolved — such a section is
/// not folded (conservative).
fn reloc_target_name(obj: &InputObject, sym: SymbolIndex) -> Option<&[u8]> {
    let pos = *obj.symbol_map.get(&sym.0)?;
    obj.symbols.get(pos).map(|s| s.name.as_slice())
}

/// x86-64 relocation types that take a symbol's ADDRESS (as opposed to a direct
/// PC-relative call/branch). If any reference to a symbol uses one of these, the
/// symbol's address is observable and the section defining it must NOT be folded
/// (folding would make two distinct functions compare equal by address — the
/// hazard that breaks e.g. a function-pointer dispatch table). Direct calls
/// (PC32=2, PLT32=4) do NOT take an address and are fold-safe.
fn reloc_takes_address(r_type: u32) -> bool {
    // NOT address-taking: R_X86_64_PC32 (2), R_X86_64_PLT32 (4), R_X86_64_NONE.
    !matches!(r_type, 0 | 2 | 4)
}

/// The address-significance information inferred from how symbols/sections are
/// referenced (the sound substitute for a compiler `.llvm_addrsig` table).
#[derive(Default)]
struct AddrTaint {
    /// Symbol NAMES whose address is taken via a named-symbol relocation.
    by_name: rustc_hash::FxHashSet<Vec<u8>>,
    /// (object_id, section_index) of sections whose address is taken via a
    /// SECTION-relative relocation (e.g. a Rust vtable storing `section + addend`
    /// to point at a function inside that section). Rust/C++ reference foldable
    /// functions this way, so a name-only scan misses them — we must exclude the
    /// whole referenced section.
    by_section: rustc_hash::FxHashSet<(usize, usize)>,
}

/// Build the address-taint set: every symbol/section whose address is taken by a
/// non-call relocation anywhere in the link. A section is ineligible for folding
/// if it defines a name-taken symbol OR is itself the (object, section) target of
/// a section-relative address-taking reloc.
fn address_taint(objects: &[InputObject]) -> AddrTaint {
    let mut t = AddrTaint::default();
    for (obj_id, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            for r in &sec.relocs {
                taint_one_reloc(&mut t, obj, obj_id, r);
            }
        }
    }
    t
}

/// Record the address-taint contribution of a single relocation.
fn taint_one_reloc(
    t: &mut AddrTaint,
    obj: &InputObject,
    obj_id: usize,
    r: &peony_object::InputReloc,
) {
    if !reloc_takes_address(r.r_type) {
        return;
    }
    let Some(&pos) = obj.symbol_map.get(&r.symbol.0) else {
        return;
    };
    let Some(sym) = obj.symbols.get(pos) else {
        return;
    };
    if sym.name.is_empty() {
        // Nameless section symbol: the address points INTO a section
        // (sym.section). Taint that whole section — we can't tell which function
        // at the addend offset is meant, so conservatively exclude it.
        if let Some(si) = sym.section {
            t.by_section.insert((obj_id, si.0));
        }
    } else {
        t.by_name.insert(sym.name.clone());
    }
}

/// Is this section ineligible for folding because it defines an
/// address-significant symbol? A symbol is address-significant if it is a C++
/// vtable/typeinfo (ABI-unique), is globally visible (its address may be
/// compared across the program), or has its address taken by any relocation in
/// the link (`addr_taken`).
fn defines_address_significant(
    obj: &InputObject,
    obj_id: usize,
    sec_pos: usize,
    taint: &AddrTaint,
) -> bool {
    let Some(sec) = obj.sections.get(sec_pos) else {
        return true;
    };
    let sidx = sec.index.0;
    // The section itself is the target of a section-relative address-taking
    // reloc (Rust vtable / function-pointer table pointing `section + addend`).
    if taint.by_section.contains(&(obj_id, sidx)) {
        return true;
    }
    for sym in &obj.symbols {
        if sym.section.map(|s| s.0) != Some(sidx) {
            continue;
        }
        if sym.is_undefined {
            continue;
        }
        // Nameless section symbols (STT_SECTION) don't constitute an
        // address-significant *function* identity; ignore them here.
        if sym.name.is_empty() {
            continue;
        }
        // Vtable / typeinfo / typeinfo-name must stay unique (C++ ABI).
        if sym.name.starts_with(b"_ZTV")
            || sym.name.starts_with(b"_ZTI")
            || sym.name.starts_with(b"_ZTS")
        {
            return true;
        }
        // Its address is taken somewhere (function-pointer table, &fn, vtable
        // slot, etc.) — folding would alias two distinct functions. This is the
        // load-bearing safety condition, inferred from how the symbol is
        // actually referenced (sound substitute for `.llvm_addrsig`).
        if taint.by_name.contains(&sym.name) {
            return true;
        }
        // A WEAK definition participates in cross-module override semantics
        // (another object may define the "same" symbol differently); never fold.
        if matches!(sym.binding, peony_object::Binding::Weak) {
            return true;
        }
        // A default-visibility GLOBAL is externally observable/interposable: a
        // shared library or another TU could take its address or rely on its
        // identity, which we cannot see from this link. Only fold globals that
        // are hidden/internal (definitely local to this output). Local-binding
        // symbols are always safe.
        if !matches!(sym.binding, peony_object::Binding::Local)
            && sym.visibility == peony_object::elf::STV_DEFAULT
        {
            return true;
        }
    }
    false
}

/// The result of ICF analysis: a map from a folded (duplicate) input section to
/// its canonical representative `(object_id, section_index)`. The canonical
/// section keeps its bytes; the duplicates are excluded from emission and have
/// their addresses aliased to the canonical's.
pub type FoldMap = FxHashMap<(usize, usize), (usize, usize)>;

/// Build the fold key for one section, or `None` if it is ineligible (not text,
/// empty, address-significant, or has an unresolvable reloc target).
fn fold_key_for(
    obj: &InputObject,
    obj_id: usize,
    pos: usize,
    taint: &AddrTaint,
) -> Option<FoldKey> {
    let sec = obj.sections.get(pos)?;
    if sec.kind != peony_object::SectionKind::Text || sec.data.is_empty() {
        return None;
    }
    if defines_address_significant(obj, obj_id, pos, taint) {
        return None;
    }
    let mut relocs = Vec::with_capacity(sec.relocs.len());
    for r in &sec.relocs {
        let name = reloc_target_name(obj, r.symbol)?;
        relocs.push((r.offset, r.r_type, r.addend, name.to_vec()));
    }
    Some(FoldKey {
        flags: sec.flags,
        data: sec.data.clone(),
        relocs,
    })
}

/// Compute the fold map over all `.text` sections of `objects`. Only sections
/// that are byte+reloc identical and free of address-significant symbols are
/// folded. Deterministic: the canonical is the first `(object_id, section_index)`
/// seen in iteration order, so output is stable.
pub fn compute_fold_map(objects: &[InputObject]) -> FoldMap {
    let mut canonical: FxHashMap<FoldKey, (usize, usize)> = FxHashMap::default();
    let mut folds: FoldMap = FxHashMap::default();
    // Address-significance taint — sections defining/pointed-at by address-taking
    // relocations are never folded (the sound substitute for `.llvm_addrsig`).
    let taint = address_taint(objects);

    for (obj_id, obj) in objects.iter().enumerate() {
        for pos in 0..obj.sections.len() {
            let Some(key) = fold_key_for(obj, obj_id, pos, &taint) else {
                continue;
            };
            let here = (obj_id, obj.sections[pos].index.0);
            match canonical.get(&key) {
                Some(&canon) => {
                    folds.insert(here, canon);
                }
                None => {
                    canonical.insert(key, here);
                }
            }
        }
    }
    folds
}

#[cfg(test)]
mod tests {
    use peony_object::{
        Binding,
        InputSection,
        InputSymbol,
        SectionIndex,
        SectionKind,
        SymbolIndex,
    };
    use rustc_hash::FxHashMap;

    use super::*;

    fn text_section(index: usize, data: &[u8]) -> InputSection {
        InputSection {
            index: SectionIndex(index),
            name: b".text.f".to_vec(),
            kind: SectionKind::Text,
            sh_type: 1,
            data: data.to_vec(),
            align: 1,
            size: data.len() as u64,
            flags: 0x6, // ALLOC | EXEC
            relocs: Vec::new(),
        }
    }

    fn local_sym(name: &[u8], sec: usize) -> InputSymbol {
        InputSymbol {
            index: SymbolIndex(0),
            name: name.to_vec(),
            binding: Binding::Local,
            is_undefined: false,
            is_common: false,
            is_ifunc: false,
            st_type: 0,
            visibility: 0,
            section: Some(SectionIndex(sec)),
            value: 0,
            size: 0,
        }
    }

    fn obj(path: &str, sections: Vec<InputSection>, symbols: Vec<InputSymbol>) -> InputObject {
        let mut section_map = FxHashMap::default();
        for (pos, s) in sections.iter().enumerate() {
            section_map.insert(s.index.0, pos);
        }
        let mut symbol_map = FxHashMap::default();
        for (pos, s) in symbols.iter().enumerate() {
            symbol_map.insert(s.index.0, pos);
        }
        InputObject {
            path: path.to_string(),
            sections,
            symbols,
            section_map,
            symbol_map,
            comdat_groups: Vec::new(),
        }
    }

    #[test]
    fn folds_two_identical_local_text_sections() {
        // Two objects, each with a byte-identical local .text section.
        let o0 = obj(
            "a.o",
            vec![text_section(1, &[0xc3, 0x90])],
            vec![local_sym(b"f", 1)],
        );
        let o1 = obj(
            "b.o",
            vec![text_section(1, &[0xc3, 0x90])],
            vec![local_sym(b"g", 1)],
        );
        let folds = compute_fold_map(&[o0, o1]);
        // The second section folds onto the first.
        assert_eq!(folds.get(&(1, 1)), Some(&(0, 1)));
        assert_eq!(folds.len(), 1);
    }

    #[test]
    fn does_not_fold_different_bytes() {
        let o0 = obj(
            "a.o",
            vec![text_section(1, &[0xc3])],
            vec![local_sym(b"f", 1)],
        );
        let o1 = obj(
            "b.o",
            vec![text_section(1, &[0x90])],
            vec![local_sym(b"g", 1)],
        );
        assert!(compute_fold_map(&[o0, o1]).is_empty());
    }

    #[test]
    fn does_not_fold_address_significant_global() {
        // A globally-visible function may have its address taken → never fold.
        let mut g0 = local_sym(b"f", 1);
        g0.binding = Binding::Global;
        let mut g1 = local_sym(b"g", 1);
        g1.binding = Binding::Global;
        let o0 = obj("a.o", vec![text_section(1, &[0xc3, 0x90])], vec![g0]);
        let o1 = obj("b.o", vec![text_section(1, &[0xc3, 0x90])], vec![g1]);
        assert!(compute_fold_map(&[o0, o1]).is_empty());
    }

    #[test]
    fn does_not_fold_vtable_even_if_local() {
        let o0 = obj(
            "a.o",
            vec![text_section(1, &[0xc3, 0x90])],
            vec![local_sym(b"_ZTV1A", 1)],
        );
        let o1 = obj(
            "b.o",
            vec![text_section(1, &[0xc3, 0x90])],
            vec![local_sym(b"_ZTV1B", 1)],
        );
        assert!(compute_fold_map(&[o0, o1]).is_empty());
    }
}
