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

use peony_object::{InputArena, InputObject, Name, SymbolIndex};
use rustc_hash::FxHashMap;

/// A canonical key identifying a foldable section's observable content: a
/// 128-bit digest of its raw bytes (zero-copy: hashed straight from the arena,
/// never cloned) plus a normalised view of its relocations (target *name*, not
/// index, so the same call across two objects hashes equal). The byte *length*
/// is part of the key so two sections must agree on length before a digest hit
/// is trusted; a digest collision is verified by a real byte compare in
/// [`compute_fold_map`] before folding (sound — no wrong folds).
#[derive(PartialEq, Eq, Hash)]
struct FoldKey {
    flags: u64,
    /// 128-bit content digest of the section bytes.
    digest: u128,
    /// Section byte length (guards the digest).
    len: u32,
    /// (offset, r_type, addend, target-symbol-name) per relocation, in order.
    relocs: Vec<(u64, u32, i64, Vec<u8>)>,
}

/// A fast 128-bit content digest (double-FNV) of a section's bytes. Used as the
/// ICF fold key so identical bytes hash equal without cloning the bytes.
fn content_digest(bytes: &[u8]) -> u128 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h1 = OFFSET;
    let mut h2 = 0x9e37_79b9_7f4a_7c15u64;
    for &b in bytes {
        h1 = (h1 ^ b as u64).wrapping_mul(PRIME);
        h2 = (h2.wrapping_add(b as u64)).wrapping_mul(PRIME) ^ (h2 >> 29);
    }
    ((h1 as u128) << 64) | h2 as u128
}

/// Resolve a relocation's target symbol to its name bytes (stable across
/// objects). Returns `None` if the symbol can't be resolved — such a section is
/// not folded (conservative).
fn reloc_target_name(obj: &InputObject, sym: SymbolIndex) -> Option<&[u8]> {
    let pos = *obj.symbol_map.get(&sym.0)?;
    obj.symbols.get(pos).map(|s| s.name.as_bytes())
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

/// SHT_LLVM_ADDRSIG — the compiler's explicit list of address-significant
/// symbols. ICF is sound ONLY for symbols the compiler proves are NOT
/// address-significant, which is exactly "has an `.llvm_addrsig` section AND the
/// symbol is not listed in it." Inferring this from relocations is unsound
/// (rustc references foldable functions through section+addend and runtime-built
/// tables that a static scan cannot see — folding ripgrep's flag handlers that
/// way silently corrupted its output), so we REQUIRE the addrsig table.
const SHT_LLVM_ADDRSIG: u32 = 0x6fff_4c03;

/// The address-significance information.
#[derive(Default)]
struct AddrTaint {
    /// Symbol NAMES whose address is taken via a named-symbol relocation.
    by_name: rustc_hash::FxHashSet<Name>,
    /// (object_id, section_index) of sections whose address is taken via a
    /// SECTION-relative relocation (e.g. a Rust vtable storing `section + addend`
    /// to point at a function inside that section). Rust/C++ reference foldable
    /// functions this way, so a name-only scan misses them — we must exclude the
    /// whole referenced section.
    by_section: rustc_hash::FxHashSet<(usize, usize)>,
    /// Object ids that carry an `.llvm_addrsig` section. A section is ONLY ever
    /// folded if its object is in this set (otherwise we have no proof any
    /// symbol is address-insignificant, so we fold nothing — sound).
    objects_with_addrsig: rustc_hash::FxHashSet<usize>,
    /// (object_id, symbol-position) explicitly marked address-significant by an
    /// `.llvm_addrsig` table.
    addrsig_syms: rustc_hash::FxHashSet<(usize, usize)>,
}

/// Decode an unsigned LEB128 from `data` at `*pos`, advancing `*pos`.
fn read_uleb128(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Build the address-taint set: every symbol/section whose address is taken by a
/// non-call relocation anywhere in the link. A section is ineligible for folding
/// if it defines a name-taken symbol OR is itself the (object, section) target of
/// a section-relative address-taking reloc.
fn address_taint(arena: &InputArena, objects: &[InputObject]) -> AddrTaint {
    let mut t = AddrTaint::default();
    for (obj_id, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            for r in &sec.relocs {
                taint_one_reloc(&mut t, obj, obj_id, r);
            }
        }
        parse_addrsig(arena, &mut t, obj, obj_id);
    }
    t
}

/// Parse this object's `.llvm_addrsig` section (if present): record that the
/// object HAS the table, and which of its symbols are address-significant. The
/// table is a sequence of ULEB128 ELF symbol-table indices.
fn parse_addrsig(arena: &InputArena, t: &mut AddrTaint, obj: &InputObject, obj_id: usize) {
    let Some(sec) = obj.sections.iter().find(|s| s.sh_type == SHT_LLVM_ADDRSIG) else {
        return;
    };
    t.objects_with_addrsig.insert(obj_id);
    let data = arena.bytes(sec.data);
    let mut pos = 0usize;
    while pos < data.len() {
        let Some(sym_idx) = read_uleb128(data, &mut pos) else {
            break;
        };
        if let Some(&sym_pos) = obj.symbol_map.get(&(sym_idx as usize)) {
            t.addrsig_syms.insert((obj_id, sym_pos));
        }
    }
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
    // HARD REQUIREMENT: only fold sections from objects that carry an
    // `.llvm_addrsig` table. Without it we have no sound proof that any symbol is
    // address-INsignificant (relocation inference is unsound — see the module
    // header), so we decline to fold. This makes ICF a safe no-op on rustc output
    // (no addrsig) while still folding clang/LLVM `-faddrsig` C/C++.
    if !taint.objects_with_addrsig.contains(&obj_id) {
        return true;
    }
    let sidx = sec.index.0;
    // The section itself is the target of a section-relative address-taking
    // reloc (Rust vtable / function-pointer table pointing `section + addend`).
    if taint.by_section.contains(&(obj_id, sidx)) {
        return true;
    }
    for (sym_pos, sym) in obj.symbols.iter().enumerate() {
        if sym.section.map(|s| s.0) != Some(sidx) {
            continue;
        }
        if sym.is_undefined {
            continue;
        }
        // The compiler explicitly marked this symbol address-significant.
        if taint.addrsig_syms.contains(&(obj_id, sym_pos)) {
            return true;
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
    arena: &InputArena,
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
        digest: content_digest(arena.bytes(sec.data)),
        len: sec.data.len() as u32,
        relocs,
    })
}

/// Compute the fold map over all `.text` sections of `objects`. Only sections
/// that are byte+reloc identical and free of address-significant symbols are
/// folded. Deterministic: the canonical is the first `(object_id, section_index)`
/// seen in iteration order, so output is stable.
pub fn compute_fold_map(arena: &InputArena, objects: &[InputObject]) -> FoldMap {
    // The canonical value carries both the output key `(obj_id, section_index)`
    // AND the `(obj_id, pos)` needed to re-read the canonical bytes for a
    // collision check, so a 128-bit digest collision can never fold two sections
    // whose bytes actually differ.
    let mut canonical: FxHashMap<FoldKey, (usize, usize, usize)> = FxHashMap::default();
    let mut folds: FoldMap = FxHashMap::default();
    // Address-significance taint — sections defining/pointed-at by address-taking
    // relocations are never folded (the sound substitute for `.llvm_addrsig`).
    let taint = address_taint(arena, objects);

    for (obj_id, obj) in objects.iter().enumerate() {
        for pos in 0..obj.sections.len() {
            let Some(key) = fold_key_for(arena, obj, obj_id, pos, &taint) else {
                continue;
            };
            let Some(&(c_obj, c_pos, c_index)) = canonical.get(&key) else {
                // First section with this key becomes the canonical.
                canonical.insert(key, (obj_id, pos, obj.sections[pos].index.0));
                continue;
            };
            // Verify byte-equality before folding (digest collision guard). On the
            // astronomically-rare digest collision with differing bytes, do not
            // fold `here` (sound — it is simply left un-canonicalised).
            let here_bytes = arena.bytes(obj.sections[pos].data);
            let canon_bytes = arena.bytes(objects[c_obj].sections[c_pos].data);
            if here_bytes == canon_bytes {
                folds.insert((obj_id, obj.sections[pos].index.0), (c_obj, c_index));
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

    fn text_section(arena: &mut InputArena, index: usize, data: &[u8]) -> InputSection {
        InputSection {
            index: SectionIndex(index),
            name: peony_object::Name::from_slice(b".text.f"),
            kind: SectionKind::Text,
            sh_type: 1,
            data: arena.intern_bytes(data),
            align: 1,
            size: data.len() as u64,
            flags: 0x6, // ALLOC | EXEC
            relocs: Vec::new(),
        }
    }

    fn local_sym(name: &[u8], sec: usize) -> InputSymbol {
        InputSymbol {
            index: SymbolIndex(0),
            name: peony_object::Name::from_slice(name),
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

    /// An `.llvm_addrsig` section whose body is the ULEB128 list of the given raw
    /// symbol indices (the address-significant ones). Empty list = the object
    /// opts into ICF with nothing address-significant.
    fn addrsig_section(
        arena: &mut InputArena,
        index: usize,
        sig_sym_indices: &[u64],
    ) -> InputSection {
        let mut data = Vec::new();
        for &i in sig_sym_indices {
            // single-byte ULEB128 is enough for small test indices
            assert!(i < 0x80);
            data.push(i as u8);
        }
        InputSection {
            index: SectionIndex(index),
            name: peony_object::Name::from_slice(b".llvm_addrsig"),
            kind: SectionKind::Other,
            sh_type: super::SHT_LLVM_ADDRSIG,
            size: data.len() as u64,
            data: arena.intern_bytes(&data),
            align: 1,
            flags: 0,
            relocs: Vec::new(),
        }
    }

    /// Build an object that OPTS INTO ICF (carries an empty `.llvm_addrsig`).
    fn obj(
        arena: &mut InputArena,
        path: &str,
        sections: Vec<InputSection>,
        symbols: Vec<InputSymbol>,
    ) -> InputObject {
        obj_addrsig(arena, path, sections, symbols, Some(&[]))
    }

    /// Build an object; `addrsig` is `Some(significant indices)` to include an
    /// `.llvm_addrsig` table, or `None` to omit it entirely (ICF must decline).
    fn obj_addrsig(
        arena: &mut InputArena,
        path: &str,
        mut sections: Vec<InputSection>,
        symbols: Vec<InputSymbol>,
        addrsig: Option<&[u64]>,
    ) -> InputObject {
        if let Some(sig) = addrsig {
            let idx = sections.iter().map(|s| s.index.0).max().unwrap_or(0) + 1;
            sections.push(addrsig_section(arena, idx, sig));
        }
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
        let mut arena = InputArena::new();
        let s0 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o0 = obj(&mut arena, "a.o", s0, vec![local_sym(b"f", 1)]);
        let s1 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o1 = obj(&mut arena, "b.o", s1, vec![local_sym(b"g", 1)]);
        let folds = compute_fold_map(&arena, &[o0, o1]);
        // The second section folds onto the first.
        assert_eq!(folds.get(&(1, 1)), Some(&(0, 1)));
        assert_eq!(folds.len(), 1);
    }

    #[test]
    fn does_not_fold_different_bytes() {
        let mut arena = InputArena::new();
        let s0 = vec![text_section(&mut arena, 1, &[0xc3])];
        let o0 = obj(&mut arena, "a.o", s0, vec![local_sym(b"f", 1)]);
        let s1 = vec![text_section(&mut arena, 1, &[0x90])];
        let o1 = obj(&mut arena, "b.o", s1, vec![local_sym(b"g", 1)]);
        assert!(compute_fold_map(&arena, &[o0, o1]).is_empty());
    }

    #[test]
    fn does_not_fold_address_significant_global() {
        // A globally-visible function may have its address taken → never fold.
        let mut g0 = local_sym(b"f", 1);
        g0.binding = Binding::Global;
        let mut g1 = local_sym(b"g", 1);
        g1.binding = Binding::Global;
        let mut arena = InputArena::new();
        let s0 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o0 = obj(&mut arena, "a.o", s0, vec![g0]);
        let s1 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o1 = obj(&mut arena, "b.o", s1, vec![g1]);
        assert!(compute_fold_map(&arena, &[o0, o1]).is_empty());
    }

    #[test]
    fn does_not_fold_vtable_even_if_local() {
        let mut arena = InputArena::new();
        let s0 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o0 = obj(&mut arena, "a.o", s0, vec![local_sym(b"_ZTV1A", 1)]);
        let s1 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o1 = obj(&mut arena, "b.o", s1, vec![local_sym(b"_ZTV1B", 1)]);
        assert!(compute_fold_map(&arena, &[o0, o1]).is_empty());
    }

    #[test]
    fn does_not_fold_without_addrsig_table() {
        // Objects with NO `.llvm_addrsig` section must never fold — we have no
        // proof any symbol is address-insignificant. This is what makes ICF a
        // safe no-op on rustc output (which omits the table).
        let mut arena = InputArena::new();
        let s0 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o0 = obj_addrsig(&mut arena, "a.o", s0, vec![local_sym(b"f", 1)], None);
        let s1 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o1 = obj_addrsig(&mut arena, "b.o", s1, vec![local_sym(b"g", 1)], None);
        assert!(
            compute_fold_map(&arena, &[o0, o1]).is_empty(),
            "no .llvm_addrsig ⇒ ICF must decline to fold"
        );
    }

    #[test]
    fn does_not_fold_addrsig_listed_symbol() {
        // The symbol at raw index 0 is marked address-significant by the table.
        let mut s0 = local_sym(b"f", 1);
        s0.index = SymbolIndex(0);
        let mut s1 = local_sym(b"g", 1);
        s1.index = SymbolIndex(0);
        let mut arena = InputArena::new();
        let sec0 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o0 = obj_addrsig(&mut arena, "a.o", sec0, vec![s0], Some(&[0]));
        let sec1 = vec![text_section(&mut arena, 1, &[0xc3, 0x90])];
        let o1 = obj_addrsig(&mut arena, "b.o", sec1, vec![s1], Some(&[0]));
        assert!(
            compute_fold_map(&arena, &[o0, o1]).is_empty(),
            "addrsig-listed symbol's section must not fold"
        );
    }
}
