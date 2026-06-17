//! `peony-symbols` — Global symbol table: resolution, weak/strong rules.
//!
//! This crate owns the **global symbol table** built from all input objects.
//! It implements the standard Unix symbol resolution rules:
//!
//! 1. Strong global > weak global > undefined.
//! 2. First strong definition wins; a second strong definition is a duplicate
//!    symbol error.
//! 3. Weak definitions are replaced by strong definitions silently.
//!
//! Each defined symbol records the *defining object*, its *input section index*,
//! and its *value* (offset within that section). After layout assigns section
//! addresses, [`SymbolResolution::virtual_address`] is filled by
//! `peony_layout::finalize_symbols` as `section_address + value`.
//!
//! ## References
//!
//! * MaskRay, "Why isn't ld.lld faster?" — symbol resolution pass description
//! * mold design.md — string interning + parallel symbol table init
//! * Maier et al. arXiv:1601.04017 — lock-free linear-probing hash tables

use std::hash::{Hash, Hasher};

use peony_object::{Binding, InputObject, InputSymbol};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use thiserror::Error;

// ── PreHashed — QUAD §2.3 (Wild PassThroughHashMap pattern) ─────────────────

/// A key wrapper that pre-computes the hash once and never re-hashes.
///
/// This implements Lemma 2.3 from QUAD: symbol names hashed once during input
/// scan, reused for all subsequent lookups, giving ~10× fewer hash computations
/// for Rust mangled names (avg ~80 bytes).
#[derive(Clone, Eq)]
pub struct PreHashed<K> {
    hash: u64,
    pub key: K,
}

impl<K: Hash> PreHashed<K> {
    pub fn new(key: K) -> Self {
        let mut h = FxHasher::default();
        key.hash(&mut h);
        Self {
            hash: h.finish(),
            key,
        }
    }
}

impl<K: PartialEq> PartialEq for PreHashed<K> {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.key == other.key
    }
}

impl<K> Hash for PreHashed<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

impl<K: std::fmt::Debug> std::fmt::Debug for PreHashed<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.key.fmt(f)
    }
}

/// Convenience: hash a byte slice with FxHasher and return the u64 hash.
pub fn fx_hash(data: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    data.hash(&mut h);
    h.finish()
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SymbolError {
    #[error("duplicate symbol `{name}`: defined in both `{first}` and `{second}`")]
    DuplicateSymbol {
        name: String,
        first: String,
        second: String,
    },
    #[error("undefined symbol `{name}` (referenced but never defined)")]
    UndefinedSymbol { name: String },
}

pub type Result<T> = std::result::Result<T, SymbolError>;

// ── Symbol IDs ────────────────────────────────────────────────────────────────

/// Compact, dense identifier for a symbol in the global table.
///
/// IDs are assigned in insertion order and are stable within a single link.
/// In incremental mode they are persisted to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// Which input object defines this symbol (or where it was first seen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId(pub u32);

// ── Resolution state ──────────────────────────────────────────────────────────

/// The fully resolved state of one global symbol.
#[derive(Debug, Clone)]
pub struct SymbolResolution {
    pub id: SymbolId,
    pub binding: Binding,
    /// Object that provides the definition (`None` if still undefined).
    pub defined_in: Option<ObjectId>,
    /// Input-section index (within `defined_in`) of the definition.
    /// `None` for absolute symbols and undefined references.
    pub section_index: Option<usize>,
    /// Symbol value: offset within its input section (or absolute value).
    pub value: u64,
    /// Symbol size in bytes (for `.symtab`).
    pub size: u64,
    /// `Some((size, align))` if this is a tentative (common) definition awaiting
    /// allocation in `.bss`. A real definition overrides it.
    pub common: Option<(u64, u64)>,
    /// Defined by a shared library (resolved at runtime via the GOT).
    pub import: bool,
    /// This import is backed by executable-owned storage and an `R_X86_64_COPY`
    /// relocation because the executable directly references DSO data.
    pub copy_reloc: bool,
    /// Index assigned in `.dynsym` (for imports); 0 = unassigned.
    pub dynsym_index: u32,
    /// Virtual address assigned during layout (zero until `finalize_symbols`).
    pub virtual_address: u64,
    /// GOT slot address (0 = not needed).
    pub got_address: u64,
    /// PLT stub address (0 = not needed; static links resolve PLT32 directly).
    pub plt_address: u64,
    /// Initial-Exec GOT slot address holding this symbol's TP offset (0 = none).
    pub gottp_address: u64,
    /// For a dynamic import, the symbol-version string required from the
    /// providing library (e.g. `GLIBC_2.34`), or `None` for an unversioned
    /// reference. Drives `.gnu.version` / `.gnu.version_r`.
    pub version: Option<Vec<u8>>,
    /// For a dynamic import, the soname of the library that provides it (e.g.
    /// `libc.so.6`). Groups version requirements per-library in `.gnu.version_r`.
    pub soname: Option<String>,
    /// `STT_GNU_IFUNC`: the definition is an indirect function. A GOT slot for it
    /// gets an `R_X86_64_IRELATIVE` so the loader runs the resolver at startup.
    pub is_ifunc: bool,
    /// ELF symbol type (`STT_*`) of the definition, for `.dynsym` export tagging.
    pub st_type: u8,
    /// ELF visibility (`STV_*`) of the definition. Hidden/internal symbols are
    /// not exported from a shared object.
    pub visibility: u8,
}

impl SymbolResolution {
    /// True if this symbol should be placed in a shared object's `.dynsym` as an
    /// export: it is locally defined (not an import, not still-undefined, not a
    /// pending common) and has default/protected visibility.
    pub fn is_export(&self) -> bool {
        self.defined_in.is_some()
            && !self.import
            && self.common.is_none()
            && (self.visibility == 0 /* STV_DEFAULT */ || self.visibility == 3/* STV_PROTECTED */)
    }
}

impl SymbolResolution {
    /// A symbol is "defined" if it has a local definition or is satisfied by a
    /// shared library import.
    pub fn is_defined(&self) -> bool {
        self.defined_in.is_some() || self.import
    }
}

// ── Global symbol table ───────────────────────────────────────────────────────

/// The global symbol table built from all input objects.
pub struct SymbolTable {
    /// Map from raw symbol name bytes → resolution.
    resolutions: FxHashMap<Vec<u8>, SymbolResolution>,
    /// Dense list of all symbol names in ID order (for reverse lookup).
    names: Vec<Vec<u8>>,
    /// Names of input objects (for error messages).
    object_paths: Vec<String>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            resolutions: FxHashMap::default(),
            names: Vec::new(),
            object_paths: Vec::new(),
        }
    }

    /// Pre-size the resolution map for a link expected to define ~`symbols`
    /// distinct symbols. A large link inserts tens of thousands of symbols; an
    /// unsized map repeatedly grows and rehashes (profiling a 423-object link
    /// showed `reserve_rehash` at ~1.7% of self-time). Reserving once removes
    /// that. Capacity is a hint only — correctness is identical to `new`.
    pub fn with_capacity(symbols: usize) -> Self {
        Self {
            resolutions: FxHashMap::with_capacity_and_hasher(symbols, Default::default()),
            names: Vec::with_capacity(symbols),
            object_paths: Vec::new(),
        }
    }

    /// Reserve room for ~`additional` more distinct symbols (capacity hint only).
    pub fn reserve(&mut self, additional: usize) {
        self.resolutions.reserve(additional);
        self.names.reserve(additional);
    }

    /// Register an input object and return its [`ObjectId`].
    pub fn add_object(&mut self, path: String) -> ObjectId {
        let id = ObjectId(self.object_paths.len() as u32);
        self.object_paths.push(path);
        id
    }

    /// Process all global/weak symbols from one input object.
    pub fn process_object(&mut self, obj_id: ObjectId, obj: &InputObject) -> Result<()> {
        self.process_object_excluding(obj_id, obj, &FxHashSet::default())
    }

    /// Like [`process_object`], but skips symbols defined in `excluded` input
    /// sections (used to drop deduplicated COMDAT group members).
    pub fn process_object_excluding(
        &mut self,
        obj_id: ObjectId,
        obj: &InputObject,
        excluded: &FxHashSet<usize>,
    ) -> Result<()> {
        for sym in &obj.symbols {
            if sym.binding == Binding::Local {
                continue; // locals are never in the global table
            }
            if sym.name.is_empty() {
                continue;
            }
            if let Some(si) = sym.section {
                if excluded.contains(&si.0) {
                    continue; // symbol lives in a discarded COMDAT section
                }
            }
            self.merge_symbol(obj_id, sym)?;
        }
        Ok(())
    }

    fn merge_symbol(&mut self, obj_id: ObjectId, sym: &InputSymbol) -> Result<()> {
        if sym.is_undefined {
            // Track the strongest binding of any undefined reference: a STRONG
            // (global) undefined reference must be satisfied, whereas a purely
            // WEAK undefined reference is allowed and resolves to zero.
            match self.resolutions.get_mut(&sym.name) {
                Some(e) if e.defined_in.is_some() => {} // already defined
                Some(e) => {
                    if sym.binding == Binding::Global {
                        e.binding = Binding::Global;
                    }
                }
                None => {
                    let mut u = Self::make_undefined_resolution();
                    u.binding = sym.binding; // Weak or Global
                    self.resolutions.insert(sym.name.clone(), u);
                }
            }
            return Ok(());
        }

        if sym.is_common {
            return self.merge_common(obj_id, sym);
        }

        let def_section = sym.section.map(|s| s.0);

        // A real definition overrides any prior tentative (common) definition.
        if let Some(e) = self.resolutions.get(&sym.name) {
            if e.common.is_some() {
                let e = self.resolutions.get_mut(&sym.name).unwrap();
                e.binding = sym.binding;
                e.defined_in = Some(obj_id);
                e.section_index = def_section;
                e.value = sym.value;
                e.size = sym.size;
                e.common = None;
                e.st_type = sym.st_type;
                e.visibility = sym.visibility;
                return Ok(());
            }
        }

        if let Some(existing) = self.resolutions.get(&sym.name) {
            let action = conflict_action(existing, obj_id, sym, &self.object_paths)?;
            match action {
                ConflictAction::SatisfyUndef | ConflictAction::Upgrade => {
                    // (Re)bind to this definition. SatisfyUndef assigns a fresh ID;
                    // Upgrade keeps the existing ID (a weak def already had one).
                    let assign_id = matches!(action, ConflictAction::SatisfyUndef);
                    let new_id = if assign_id {
                        let id = SymbolId(self.names.len() as u32);
                        self.names.push(sym.name.clone());
                        Some(id)
                    } else {
                        None
                    };
                    let e = self.resolutions.get_mut(&sym.name).unwrap();
                    if let Some(id) = new_id {
                        e.id = id;
                    }
                    e.binding = sym.binding;
                    e.defined_in = Some(obj_id);
                    e.section_index = def_section;
                    e.value = sym.value;
                    e.size = sym.size;
                    e.is_ifunc = sym.is_ifunc;
                    e.st_type = sym.st_type;
                    e.visibility = sym.visibility;
                }
                ConflictAction::KeepExisting => {}
                ConflictAction::WarnLocal => {
                    tracing::warn!(
                        "local symbol `{}` unexpectedly reached global table",
                        String::from_utf8_lossy(&sym.name)
                    );
                }
            }
        } else {
            let id = SymbolId(self.names.len() as u32);
            self.names.push(sym.name.clone());
            self.resolutions.insert(
                sym.name.clone(),
                SymbolResolution {
                    id,
                    binding: sym.binding,
                    defined_in: Some(obj_id),
                    section_index: def_section,
                    value: sym.value,
                    size: sym.size,
                    common: None,
                    import: false,
                    copy_reloc: false,
                    dynsym_index: 0,
                    virtual_address: 0,
                    got_address: 0,
                    plt_address: 0,
                    gottp_address: 0,
                    version: None,
                    soname: None,
                    is_ifunc: sym.is_ifunc,
                    st_type: sym.st_type,
                    visibility: sym.visibility,
                },
            );
        }
        Ok(())
    }

    /// Merge a tentative (common) definition.
    fn merge_common(&mut self, obj_id: ObjectId, sym: &InputSymbol) -> Result<()> {
        let size = sym.size;
        let align = sym.value.max(1);
        // Snapshot the existing entry's relevant state to avoid borrow conflicts.
        let existing = self
            .resolutions
            .get(&sym.name)
            .map(|e| (e.defined_in.is_some(), e.common));
        match existing {
            // A real definition already wins; ignore the common.
            Some((true, None)) => {}
            // Merge two commons: take the maximum size and alignment.
            Some((_, Some((es, ea)))) => {
                let e = self.resolutions.get_mut(&sym.name).unwrap();
                let s = size.max(es);
                e.common = Some((s, align.max(ea)));
                e.size = s;
            }
            // Existing undefined slot → turn it into a common definition.
            Some((false, None)) => {
                let id = SymbolId(self.names.len() as u32);
                self.names.push(sym.name.clone());
                let e = self.resolutions.get_mut(&sym.name).unwrap();
                e.id = id;
                e.binding = sym.binding;
                e.defined_in = Some(obj_id);
                e.section_index = None;
                e.size = size;
                e.common = Some((size, align));
            }
            None => {
                let id = SymbolId(self.names.len() as u32);
                self.names.push(sym.name.clone());
                self.resolutions.insert(
                    sym.name.clone(),
                    SymbolResolution {
                        id,
                        binding: sym.binding,
                        defined_in: Some(obj_id),
                        section_index: None,
                        value: 0,
                        size,
                        common: Some((size, align)),
                        import: false,
                        copy_reloc: false,
                        dynsym_index: 0,
                        virtual_address: 0,
                        got_address: 0,
                        plt_address: 0,
                        gottp_address: 0,
                        version: None,
                        soname: None,
                        is_ifunc: false,
                        st_type: 0,
                        visibility: 0,
                    },
                );
            }
        }
        Ok(())
    }

    fn make_undefined_resolution() -> SymbolResolution {
        SymbolResolution {
            id: SymbolId(u32::MAX),
            binding: Binding::Global,
            defined_in: None,
            section_index: None,
            value: 0,
            size: 0,
            common: None,
            import: false,
            copy_reloc: false,
            dynsym_index: 0,
            virtual_address: 0,
            got_address: 0,
            plt_address: 0,
            gottp_address: 0,
            version: None,
            soname: None,
            is_ifunc: false,
            st_type: 0,
            visibility: 0,
        }
    }

    /// Look up a symbol by name.
    pub fn lookup(&self, name: &[u8]) -> Option<&SymbolResolution> {
        self.resolutions.get(name)
    }

    /// Build an id-indexed view of resolutions: `out[id.0] = Some(&resolution)`.
    ///
    /// The relocation hot path looks symbols up by *name* (a `Vec<u8>` re-hash
    /// per relocation — ~8% of total self-time on a large link). After
    /// resolution is frozen, a caller can build this once (one hash per distinct
    /// symbol) and then index by [`SymbolId`] in O(1) without hashing. Returns
    /// borrowed references (no clone); the view borrows the table, which outlives
    /// the emit phase that uses it. Slots for ids with no resolution (should not
    /// happen for a well-formed link) are `None`. Correctness is identical to
    /// repeated [`lookup`] calls — purely a lookup-cost optimization, gated by
    /// the byte-identical-output determinism tests.
    pub fn build_id_index(&self) -> Vec<Option<&SymbolResolution>> {
        let mut out: Vec<Option<&SymbolResolution>> = vec![None; self.names.len()];
        for name in &self.names {
            if let Some(r) = self.resolutions.get(name) {
                let idx = r.id.0 as usize;
                if idx < out.len() {
                    out[idx] = Some(r);
                }
            }
        }
        out
    }

    /// Mutable lookup — used by layout to fill addresses.
    pub fn lookup_mut(&mut self, name: &[u8]) -> Option<&mut SymbolResolution> {
        self.resolutions.get_mut(name)
    }

    /// Mark currently-undefined symbols that a shared library exports as dynamic
    /// imports (resolved at runtime). Returns how many references were satisfied.
    pub fn register_shared_exports(&mut self, exports: &[Vec<u8>]) -> usize {
        self.register_shared_exports_versioned(exports, &[], "")
    }

    /// As [`register_shared_exports`], but also records each satisfied import's
    /// version requirement (parallel to `exports`; `versions` may be shorter, in
    /// which case missing entries are treated as unversioned) and the providing
    /// library's `soname` (for per-library `.gnu.version_r` grouping).
    pub fn register_shared_exports_versioned(
        &mut self,
        exports: &[Vec<u8>],
        versions: &[Option<Vec<u8>>],
        soname: &str,
    ) -> usize {
        let mut n = 0;
        for (i, name) in exports.iter().enumerate() {
            // An undefined reference satisfied by the shared library.
            let satisfy = matches!(
                self.resolutions.get(name),
                Some(r) if r.defined_in.is_none() && !r.import
            );
            if !satisfy {
                continue;
            }
            // Imports need a real SymbolId so they participate in GOT/.dynsym.
            let id = SymbolId(self.names.len() as u32);
            self.names.push(name.clone());
            let ver = versions.get(i).cloned().flatten();
            let r = self.resolutions.get_mut(name).unwrap();
            r.import = true;
            r.copy_reloc = false;
            r.id = id;
            r.version = ver;
            r.soname = Some(soname.to_string());
            n += 1;
        }
        n
    }

    /// Mark undefined references satisfied by shared-library export records, also
    /// preserving size/type metadata needed for `R_X86_64_COPY` decisions.
    pub fn register_shared_export_symbols(
        &mut self,
        exports: &[peony_object::SharedExport],
        soname: &str,
    ) -> usize {
        let mut n = 0;
        for ex in exports {
            let satisfy = matches!(
                self.resolutions.get(&ex.name),
                Some(r) if r.defined_in.is_none() && !r.import
            );
            if !satisfy {
                continue;
            }
            let id = SymbolId(self.names.len() as u32);
            self.names.push(ex.name.clone());
            let r = self.resolutions.get_mut(&ex.name).unwrap();
            r.import = true;
            r.copy_reloc = false;
            r.id = id;
            r.version = ex.version.clone();
            r.soname = Some(soname.to_string());
            r.size = ex.size;
            r.st_type = ex.st_type;
            n += 1;
        }
        n
    }

    /// Mark an import as requiring an executable copy relocation.
    pub fn mark_copy_reloc(&mut self, name: &[u8]) {
        if let Some(r) = self.resolutions.get_mut(name) {
            if r.import {
                r.copy_reloc = true;
            }
        }
    }

    /// Whether any symbol is a dynamic import (→ a dynamic executable is needed).
    pub fn has_imports(&self) -> bool {
        self.resolutions.values().any(|r| r.import)
    }

    /// Ensure `name` has a real [`SymbolId`] (assigning one if it is still the
    /// placeholder `u32::MAX`). Used for weak-undefined symbols that are
    /// referenced through the GOT (e.g. `__gmon_start__`): they need a stable id
    /// so their GOT slot gets a recorded address (holding 0). Returns the id.
    pub fn ensure_id(&mut self, name: &[u8]) -> Option<SymbolId> {
        let needs = matches!(self.resolutions.get(name), Some(r) if r.id.0 == u32::MAX);
        if needs {
            let id = SymbolId(self.names.len() as u32);
            self.names.push(name.to_vec());
            self.resolutions.get_mut(name).unwrap().id = id;
            Some(id)
        } else {
            self.resolutions.get(name).map(|r| r.id)
        }
    }

    /// Define (or overwrite) `name` as an absolute symbol with `value`
    /// (used for `--defsym` and linker-provided symbols).
    pub fn define_absolute(&mut self, name: &[u8], value: u64) {
        let e = self
            .resolutions
            .entry(name.to_vec())
            .or_insert_with(Self::make_undefined_resolution);
        e.defined_in = Some(ObjectId(0));
        e.section_index = None;
        e.value = value;
        e.virtual_address = value;
    }

    /// The name bytes for a [`SymbolId`] (only valid for *defined* symbols).
    pub fn name_by_id(&self, id: SymbolId) -> Option<&[u8]> {
        self.names.get(id.0 as usize).map(|v| v.as_slice())
    }

    /// Iterate over all resolutions.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &SymbolResolution)> {
        self.resolutions.iter().map(|(k, v)| (k.as_slice(), v))
    }

    /// Mutable iteration over all resolutions (for address write-back).
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut SymbolResolution> {
        self.resolutions.values_mut()
    }

    /// Number of distinct objects registered.
    pub fn object_count(&self) -> usize {
        self.object_paths.len()
    }

    pub fn len(&self) -> usize {
        self.resolutions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resolutions.is_empty()
    }
}

// ── Conflict resolution helper (free function to avoid borrow issues) ─────────

enum ConflictAction {
    /// The existing entry was undefined; this definition satisfies it.
    SatisfyUndef,
    /// The existing entry was a weak definition; upgrade to this strong one.
    Upgrade,
    /// Keep the existing (stronger or equal) definition.
    KeepExisting,
    WarnLocal,
}

fn conflict_action(
    existing: &SymbolResolution,
    obj_id: ObjectId,
    sym: &InputSymbol,
    object_paths: &[String],
) -> std::result::Result<ConflictAction, SymbolError> {
    match (existing.defined_in, existing.binding, sym.binding) {
        (None, _, _) => Ok(ConflictAction::SatisfyUndef),
        (Some(_), Binding::Weak, Binding::Global) => Ok(ConflictAction::Upgrade),
        (Some(_), Binding::Global, Binding::Global) => Err(SymbolError::DuplicateSymbol {
            name: String::from_utf8_lossy(&sym.name).into_owned(),
            first: object_paths[existing.defined_in.unwrap().0 as usize].clone(),
            second: object_paths[obj_id.0 as usize].clone(),
        }),
        (Some(_), _, Binding::Weak) => Ok(ConflictAction::KeepExisting),
        (Some(_), _, Binding::Local) | (Some(_), Binding::Local, _) => {
            Ok(ConflictAction::WarnLocal)
        }
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fx_hash_is_deterministic_and_distinguishes_inputs() {
        assert_eq!(fx_hash(b"hello"), fx_hash(b"hello"));
        assert_ne!(fx_hash(b"hello"), fx_hash(b"world"));
        assert_ne!(fx_hash(b""), fx_hash(b"\0"));
    }

    #[test]
    fn define_absolute_then_lookup_roundtrips() {
        let mut t = SymbolTable::new();
        t.define_absolute(b"_etext", 0x4000);
        let r = t.lookup(b"_etext").expect("defined symbol must be found");
        assert_eq!(r.value, 0x4000);
        assert_eq!(r.virtual_address, 0x4000);
        assert!(t.lookup(b"_missing").is_none());
    }

    #[test]
    fn with_capacity_and_reserve_do_not_change_semantics() {
        let mut a = SymbolTable::new();
        let mut b = SymbolTable::with_capacity(128);
        b.reserve(256);
        for (i, name) in [b"a".as_slice(), b"bb", b"ccc"].iter().enumerate() {
            a.define_absolute(name, i as u64);
            b.define_absolute(name, i as u64);
        }
        for name in [b"a".as_slice(), b"bb", b"ccc"] {
            assert_eq!(
                a.lookup(name).map(|r| r.value),
                b.lookup(name).map(|r| r.value),
                "capacity hint must not change resolution"
            );
        }
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn build_id_index_matches_name_lookup() {
        // build_id_index must agree with lookup for every symbol that has a real
        // id — this invariant is what licenses the reloc-path optimization.
        let mut t = SymbolTable::new();
        for (i, name) in [b"alpha".as_slice(), b"beta", b"gamma"].iter().enumerate() {
            t.define_absolute(name, (i as u64 + 1) * 0x10);
            // Promote to a real id (as resolution/GOT assignment does).
            t.ensure_id(name);
        }
        let index = t.build_id_index();
        for name in [b"alpha".as_slice(), b"beta", b"gamma"] {
            let by_name = t.lookup(name).expect("present");
            let id = by_name.id.0 as usize;
            let by_id = index[id].expect("id slot populated");
            assert_eq!(by_id.value, by_name.value);
            assert_eq!(by_id.virtual_address, by_name.virtual_address);
            assert_eq!(by_id.id, by_name.id);
        }
    }

    #[test]
    fn build_id_index_is_empty_without_assigned_ids() {
        // define_absolute alone does not assign a dense id (id stays u32::MAX),
        // so the id-index has no addressable slots — documents the contract.
        let mut t = SymbolTable::new();
        t.define_absolute(b"x", 1);
        assert!(t.build_id_index().is_empty());
    }
}
