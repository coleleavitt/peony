//! `peony-object` — ELF object file parsing and section/symbol extraction.
//!
//! This crate wraps the [`object`] crate and exposes the subset of ELF that
//! the linker cares about:
//!
//! * Parsed [`InputObject`]s loaded from disk via `mmap`.
//! * Per-object [`InputSection`] and [`InputSymbol`] tables.
//! * Archive (`.rlib` / `.a`) member iteration with byte-level comparison
//!   for incremental diffing (rustc does not set timestamps on archive members).
//!
//! ## Design notes
//!
//! Objects are memory-mapped and parsed in parallel by the caller (via
//! `rayon`).  This crate is intentionally stateless — all parsing state lives
//! in the returned structs; the caller is responsible for lifetime management.
//!
//! Section names that follow the Rust mangled-name convention
//! (`.text._ZN…`, `.rodata._ZN…`) are *stable* across incremental rebuilds
//! and can be used directly as diff keys.  Anonymous sections
//! (`.rodata..L__unnamed_N`) must be matched by their referrer set — that
//! logic lives in `peony-cache`.

use std::path::Path;

use memmap2::Mmap;
use object::read::elf::{ElfFile64, SectionHeader};
use object::{Endianness, Object, ObjectSection, ObjectSymbol};
// Re-export the index newtypes so downstream crates (and their tests) can name
// the types of `InputSection::index` / `InputSymbol::section` / `InputReloc::symbol`.
pub use object::{SectionIndex, SymbolIndex};
use rustc_hash::FxHashMap;
use thiserror::Error;

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ObjectError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("ELF parse error in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: object::Error,
    },
    #[error("unsupported ELF class or architecture in {path}")]
    UnsupportedArch { path: String },
}

pub type Result<T> = std::result::Result<T, ObjectError>;

// ── Section kinds ─────────────────────────────────────────────────────────────

/// Coarse classification of an ELF input section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    /// Executable code (`.text*`).
    Text,
    /// Read-only data (`.rodata*`).
    ReadOnly,
    /// Read-write data (`.data*`).
    Data,
    /// Zero-initialised data (`.bss*`).
    Bss,
    /// DWARF debug information (`.debug_*`).
    Debug,
    /// Exception-handling frames (`.eh_frame`).
    EhFrame,
    /// Mergeable strings (SHF_MERGE | SHF_STRINGS).
    MergeString,
    /// Mergeable fixed-width constants (SHF_MERGE without SHF_STRINGS).
    MergeConst,
    /// Init/fini array (`.init_array`, `.fini_array`).
    InitArray,
    /// Thread-local initialized data (`.tdata`, `SHF_TLS`).
    Tdata,
    /// Thread-local zero data (`.tbss`, `SHF_TLS` + NOBITS).
    Tbss,
    /// Anything else we pass through without interpretation.
    Other,
}

// ── Zero-copy section backing store ────────────────────────────────────────────

/// Link-wide backing store for all input section bytes. Holds every input
/// `mmap` (bare objects *and* archives) for the whole link, plus the rare
/// transformed-bytes cases (decompressed `.debug_*`). Sections reference their
/// bytes by index into this arena — so [`InputSection`] carries no lifetime and
/// stays `Copy`-cheap, while the actual 17MB+ of section data is never copied
/// out of the `mmap` (it used to be `memcpy`'d into a per-section `Vec<u8>`,
/// which dominated parse bandwidth and inflated peak RSS).
///
/// The arena must outlive layout and emit (its bytes are read during the output
/// blit). It is created in `load_and_resolve` and returned alongside the
/// objects so it lives for the whole link.
#[derive(Default)]
pub struct InputArena {
    /// Memory-mapped input files, indexed by `file_id` ([`DataSrc::Mmap`]).
    mmaps: Vec<Mmap>,
    /// Transformed/synthesized bytes (decompressed debug), indexed by
    /// [`DataSrc::Owned`]. Allocated only for genuinely-transformed sections.
    owned: Vec<Vec<u8>>,
}

impl InputArena {
    pub fn new() -> Self {
        Self::default()
    }

    /// Move a mapped input file in; returns its `file_id` for [`DataSrc::Mmap`].
    pub fn push_mmap(&mut self, m: Mmap) -> u32 {
        let id = self.mmaps.len() as u32;
        self.mmaps.push(m);
        id
    }

    /// Store transformed bytes; returns the index for [`DataSrc::Owned`].
    pub fn push_owned(&mut self, v: Vec<u8>) -> u32 {
        let id = self.owned.len() as u32;
        self.owned.push(v);
        id
    }

    /// Append an object's locally-produced owned buffers (from
    /// [`parse_bare_parallel`], which used `owned_base = 0`) to the arena and
    /// rebase that object's `Owned` section handles by the arena's prior owned
    /// length. Called serially in object order after a parallel parse, so the
    /// final owned indices are deterministic regardless of parse completion
    /// order. No-op for the common object (no owned/compressed-debug sections).
    pub fn merge_parsed_owned(&mut self, obj: &mut InputObject, local_owned: Vec<Vec<u8>>) {
        if local_owned.is_empty() {
            return;
        }
        let base = self.owned.len() as u32;
        for sec in &mut obj.sections {
            if let DataSrc::Owned(i) = sec.data.src {
                sec.data.src = DataSrc::Owned(base + i);
            }
        }
        self.owned.extend(local_owned);
    }

    /// Intern raw bytes and return a [`SectionData`] handle pointing at them.
    /// Convenience for callers (and tests) that have bytes in hand rather than a
    /// file range — the bytes go into the owned store.
    pub fn intern_bytes(&mut self, bytes: &[u8]) -> SectionData {
        assert!(
            bytes.len() <= u32::MAX as usize,
            "section too large for u32 len"
        );
        let len = bytes.len() as u32;
        let id = self.push_owned(bytes.to_vec());
        SectionData {
            src: DataSrc::Owned(id),
            off: 0,
            len,
        }
    }

    /// The full bytes of a mapped input file (used to parse straight from the map).
    #[inline]
    pub fn mmap_bytes(&self, file_id: u32) -> &[u8] {
        &self.mmaps[file_id as usize]
    }

    /// The full bytes of an owned buffer (e.g. an archive member blob). The
    /// inner `Vec<u8>` is heap-stable across later `push_owned` calls (only the
    /// outer `Vec` of buffers may realloc), so a slice taken here stays valid.
    #[inline]
    pub fn owned_bytes(&self, owned_id: u32) -> &[u8] {
        &self.owned[owned_id as usize]
    }

    /// Resolve a [`SectionData`] handle to its byte slice.
    #[inline]
    pub fn bytes(&self, d: SectionData) -> &[u8] {
        let base: &[u8] = match d.src {
            DataSrc::Mmap(i) => &self.mmaps[i as usize],
            DataSrc::Owned(i) => &self.owned[i as usize],
        };
        &base[d.off as usize..d.off as usize + d.len as usize]
    }
}

/// Where an [`InputSection`]'s bytes live. `Copy + Send + Sync + 'static` — it
/// carries no lifetime, so it does not infect `InputObject`/`SymbolTable`/layout
/// with an arena lifetime parameter.
#[derive(Debug, Clone, Copy)]
pub struct SectionData {
    pub src: DataSrc,
    /// Byte offset of the section within its backing buffer.
    pub off: u32,
    /// Section byte length (the contribution length; may be < `InputSection.size`
    /// for BSS, and is 4 bytes shorter than the file range for a stripped
    /// `.eh_frame` terminator).
    pub len: u32,
}

impl SectionData {
    /// An empty handle (no bytes), e.g. for BSS / zero-length sections.
    pub const EMPTY: SectionData = SectionData {
        src: DataSrc::Mmap(0),
        off: 0,
        len: 0,
    };
    /// Byte length, without needing the arena.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The backing buffer kind for [`SectionData`]. Archives are NOT a separate
/// variant — an archive's whole file is `mmap`'d into `mmaps` and a member
/// section is `Mmap(archive_file_id)` with an absolute offset.
#[derive(Debug, Clone, Copy)]
pub enum DataSrc {
    /// Index into [`InputArena::mmaps`].
    Mmap(u32),
    /// Index into [`InputArena::owned`].
    Owned(u32),
}

// ── Core data types ───────────────────────────────────────────────────────────

/// A single ELF section extracted from an [`InputObject`].
#[derive(Debug, Clone)]
pub struct InputSection {
    /// Index within the parent object's section table.
    pub index: SectionIndex,
    /// Section name, interned as a `Vec<u8>` (may not be valid UTF-8).
    pub name: Vec<u8>,
    /// Coarse classification.
    pub kind: SectionKind,
    /// Raw ELF section type (`SHT_*`).
    pub sh_type: u32,
    /// Handle to the section's raw bytes in the [`InputArena`] (zero-copy: a
    /// borrow into the input `mmap`, not an owned copy).
    pub data: SectionData,
    /// Section alignment (must be a power of two).
    pub align: u64,
    /// Size in bytes (may differ from `data.len()` for BSS).
    pub size: u64,
    /// SHF_* flags from the ELF header.
    pub flags: u64,
    /// Relocations targeting this section.
    pub relocs: Vec<InputReloc>,
}

/// A single relocation entry within an [`InputSection`].
#[derive(Debug, Clone)]
pub struct InputReloc {
    /// Offset within the section being relocated.
    pub offset: u64,
    /// Relocation type (architecture-specific; e.g. `R_X86_64_PC32 = 2`).
    pub r_type: u32,
    /// Index of the symbol this relocation references.
    pub symbol: SymbolIndex,
    /// Addend (for RELA; zero for REL).
    pub addend: i64,
}

/// Binding strength of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Local,
    Global,
    Weak,
}

/// A symbol defined or referenced in an [`InputObject`].
#[derive(Debug, Clone)]
pub struct InputSymbol {
    pub index: SymbolIndex,
    /// Symbol name bytes (interned from mmap; clone to persist).
    pub name: Vec<u8>,
    pub binding: Binding,
    pub is_undefined: bool,
    /// Tentative (common) definition: `value` holds the required alignment and
    /// `size` the byte size. The linker allocates space in `.bss`.
    pub is_common: bool,
    /// `STT_GNU_IFUNC`: an indirect function whose `value` is a resolver. A
    /// reference to it gets an `R_X86_64_IRELATIVE` dynamic relocation so the
    /// loader runs the resolver and stores its result.
    pub is_ifunc: bool,
    /// ELF symbol type (`STT_*`) — `STT_FUNC`/`STT_OBJECT`/`STT_NOTYPE`/…. Used
    /// to tag exported `.dynsym` entries in a shared object so `dlsym` reports
    /// the right kind.
    pub st_type: u8,
    /// ELF symbol visibility (`STV_*`). A `STV_HIDDEN`/`STV_INTERNAL` symbol is
    /// never exported from a shared object's `.dynsym`.
    pub visibility: u8,
    /// Section index for defined symbols; `None` for absolute/common/undefined.
    pub section: Option<SectionIndex>,
    /// Value (offset within section for defined symbols; alignment for common).
    pub value: u64,
    pub size: u64,
}

/// A parsed ELF-64 input object file.
///
/// All section/symbol bytes are copied out of the source buffer during parsing,
/// so the object is self-contained (the backing file/mmap need not outlive it).
pub struct InputObject {
    /// Original path (for diagnostics). For archive members: `archive(member.o)`.
    pub path: String,
    pub sections: Vec<InputSection>,
    pub symbols: Vec<InputSymbol>,
    /// Map from section index → position in `sections`.
    pub section_map: FxHashMap<usize, usize>,
    /// Map from symbol index → position in `symbols`.
    pub symbol_map: FxHashMap<usize, usize>,
    /// COMDAT groups defined in this object (for cross-object deduplication).
    pub comdat_groups: Vec<ComdatGroup>,
}

/// A COMDAT section group: a set of member sections kept only once across all
/// objects that share the same `signature` (e.g. a C++ inline function).
#[derive(Debug, Clone)]
pub struct ComdatGroup {
    /// The group signature symbol's name.
    pub signature: Vec<u8>,
    /// Member section indices (within the defining object).
    pub members: Vec<usize>,
}

impl std::fmt::Debug for InputObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputObject")
            .field("path", &self.path)
            .field("sections", &self.sections.len())
            .field("symbols", &self.symbols.len())
            .finish()
    }
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Open and parse a single ELF-64 object file into `arena`.
///
/// The file is memory-mapped into the arena and the returned [`InputObject`]'s
/// sections borrow their bytes from that map (zero-copy) — only section/symbol
/// *metadata* is materialised, never the bulk section bytes.
pub fn parse_object(arena: &mut InputArena, path: &Path) -> Result<InputObject> {
    let file = std::fs::File::open(path).map_err(|e| ObjectError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    // SAFETY: the arena holds the map read-only for the whole link; never mutated.
    let mmap = unsafe { Mmap::map(&file) }.map_err(|e| ObjectError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let file_id = arena.push_mmap(mmap);
    let bytes = arena.mmap_bytes(file_id);
    // SAFETY: `bytes` borrows `arena.mmaps[file_id]`, which is never moved or
    // mutated for the arena's lifetime; parse_bytes only needs it transiently
    // and copies nothing out. We reborrow to decouple from `&mut arena` (the
    // owned-debug sink path needs &mut arena). The map's backing pages are
    // stable, so this raw reborrow is sound.
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(bytes.as_ptr(), bytes.len()) };
    parse_bytes_into(arena, file_id, path.display().to_string(), bytes)
}

/// Parse an ELF-64 object whose bytes are already in `arena` at `file_id`
/// (a bare object's mmap, or an archive's mmap with member at `base_off`).
/// Sections reference `arena.mmaps[file_id]` by absolute offset — no copy.
/// `base_off` is the member's byte offset within the backing buffer (0 for a
/// bare object that maps to the start of its own file).
pub fn parse_bytes_into(
    arena: &mut InputArena,
    file_id: u32,
    path: String,
    data: &[u8],
) -> Result<InputObject> {
    // Serial path: owned bufs (rare; compressed debug) go straight into the arena.
    let base = arena.owned.len() as u32;
    let mut sink = Vec::new();
    let obj = parse_backed(DataSrc::Mmap(file_id), base, 0, &mut sink, path, data)?;
    arena.owned.extend(sink);
    Ok(obj)
}

/// Parse an archive member whose bytes were copied into the arena's `owned`
/// store at `owned_id`. The member is a self-contained ELF blob starting at
/// offset 0 of that buffer, so section offsets are taken directly. (Archives
/// are not zero-copy from the `.a` file — a small, non-hot cost — but bare
/// objects, the bulk of a link, are.)
pub fn parse_owned_member(
    arena: &mut InputArena,
    owned_id: u32,
    path: String,
    data: &[u8],
) -> Result<InputObject> {
    let base = arena.owned.len() as u32;
    let mut sink = Vec::new();
    let obj = parse_backed(DataSrc::Owned(owned_id), base, 0, &mut sink, path, data)?;
    arena.owned.extend(sink);
    Ok(obj)
}

/// Parse a bare object straight from its mapped bytes, WITHOUT touching the
/// shared arena — for parallel parse. Returns the object plus any
/// transformed-bytes buffers it produced (compressed `.debug_*`); the caller
/// must append those to `arena.owned` IN OBJECT ORDER and the `Owned` handles
/// are pre-rebased relative to `owned_base` so the rebase is just the append.
/// `owned_base` must equal the arena's `owned` length at the point this object's
/// buffers will land — the caller computes a per-object prefix sum (deterministic,
/// independent of thread completion order). See [`InputArena::reserve_owned_base`].
pub fn parse_bare_parallel(
    file_id: u32,
    owned_base: u32,
    path: String,
    data: &[u8],
) -> Result<(InputObject, Vec<Vec<u8>>)> {
    let mut sink = Vec::new();
    let obj = parse_backed(DataSrc::Mmap(file_id), owned_base, 0, &mut sink, path, data)?;
    Ok((obj, sink))
}

/// Parse an ELF-64 object whose bytes live at `backing` (a `Mmap` or `Owned`
/// slot). Transformed-bytes sections (compressed debug) are pushed into
/// `owned_sink` and their [`DataSrc::Owned`] handle is `owned_base + sink_index`
/// — so a caller that appends `owned_sink` to `arena.owned` starting at
/// `owned_base` makes every handle correct, with NO shared-arena write during
/// parse (this is what lets parse run in parallel deterministically). `base_off`
/// is the object's offset within its backing buffer. No bytes copied for `Mmap`.
fn parse_backed(
    backing: DataSrc,
    owned_base: u32,
    base_off: u64,
    owned_sink: &mut Vec<Vec<u8>>,
    path: String,
    data: &[u8],
) -> Result<InputObject> {
    let elf: ElfFile64<Endianness> = ElfFile64::parse(data).map_err(|e| ObjectError::Parse {
        path: path.clone(),
        source: e,
    })?;

    let mut sections = Vec::new();
    let mut section_map = FxHashMap::default();
    let endian = elf.endian();

    for section in elf.sections() {
        let idx = section.index();
        let sh_type = section.elf_section_header().sh_type(endian);
        let input_name = section.name_bytes().unwrap_or(b"");
        let is_debug = is_debug_section_name(input_name);
        let name = normalize_debug_section_name(input_name);
        let sh_flags = match section.flags() {
            object::SectionFlags::Elf { sh_flags } => sh_flags,
            _ => 0,
        };
        let kind = classify_section(&name, sh_flags);

        // Determine the section's byte handle WITHOUT copying. The common case
        // (uncompressed) is a slice of the backing mmap at its file range. Only
        // a *compressed* debug section needs owned (decompressed) bytes.
        let (file_off, file_len) = section.file_range().unwrap_or((0, 0));
        let is_compressed = matches!(
            section.compressed_file_range(),
            Ok(r) if r.format != object::CompressionFormat::None
        );

        let mut data_handle: SectionData;
        let mut owned_len: Option<u64> = None;
        if is_debug && is_compressed {
            // Rare: decompress into the arena's owned store (transformed bytes).
            let bytes = section
                .uncompressed_data()
                .map_err(|e| ObjectError::Parse {
                    path: path.clone(),
                    source: e,
                })?
                .into_owned();
            let len = bytes.len();
            assert!(len <= u32::MAX as usize, "section too large for u32 len");
            // Push to the LOCAL sink; the handle is rebased by `owned_base` so the
            // caller's append-to-arena makes it correct without any shared write.
            let oid = owned_base + owned_sink.len() as u32;
            owned_sink.push(bytes);
            owned_len = Some(len as u64);
            data_handle = SectionData {
                src: DataSrc::Owned(oid),
                off: 0,
                len: len as u32,
            };
        } else if sh_type == elf::SHT_NOBITS || file_len == 0 {
            // BSS / empty: no backing bytes.
            data_handle = SectionData::EMPTY;
        } else {
            // Zero-copy (Mmap) / shared-buffer (Owned archive): a slice into the
            // backing buffer at the absolute offset. No bytes copied.
            let abs_off = base_off + file_off;
            assert!(
                abs_off + file_len <= u32::MAX as u64,
                "section offset+len {} exceeds u32 (archive/object > 4GiB not supported)",
                abs_off + file_len
            );
            data_handle = SectionData {
                src: backing,
                off: abs_off as u32,
                len: file_len as u32,
            };
        }
        let relocs = collect_relocs(&section);

        // Strip the trailing 4-byte zero CIE terminator from each input
        // `.eh_frame`: per-object terminators would otherwise appear mid-stream
        // when contributions are concatenated, causing the runtime unwinder
        // (`_Unwind_Find_FDE`) to stop at the first one. The linker emits a
        // single terminator for the merged section (handled at emit time).
        let mut size = if is_debug {
            owned_len.unwrap_or(file_len)
        } else {
            section.size()
        };
        if kind == SectionKind::EhFrame {
            // `.eh_frame` is never compressed-debug, so its bytes are a slice of
            // this object's backing buffer `data` at the section's file range
            // (no copy). Read straight from `data` — no arena needed during parse.
            let raw_data = &data[file_off as usize..(file_off + file_len) as usize];
            // Determine whether the section *ends* with a genuine 4-byte zero
            // terminator by walking records to the end. A trailing run of zero
            // bytes that is reached at a record boundary is the terminator; zeros
            // that are *inside* the last FDE (e.g. CFA padding) must NOT be
            // stripped. `scan_eh_frame` reports `leftover` = unparsed trailing
            // bytes; a clean section has leftover 0 and may end in a terminator
            // record that the scan counts via `terms`.
            let (cies, fdes, terms, leftover) = scan_eh_frame(raw_data);
            // The terminator (if any) is exactly the final 4 bytes AND the parse
            // must consume the whole section (leftover == 0) reaching it as a
            // record. We detect it by checking the byte position the scan ends.
            let ends_with_terminator = ends_with_eh_terminator(raw_data);
            tracing::trace!(
                obj = %path,
                input_size = raw_data.len(),
                sh_size = size,
                cies,
                fdes,
                terms,
                leftover,
                ends_with_terminator,
                "parse .eh_frame contribution"
            );
            if ends_with_terminator {
                // Zero-copy strip: expose a 4-byte-shorter slice instead of
                // truncating an owned Vec. The terminator stays in the mmap but
                // is excluded from this section's contribution.
                data_handle.len = data_handle.len.saturating_sub(4);
                size = size.saturating_sub(4);
            }
        }

        let isec = InputSection {
            index: idx,
            name,
            kind,
            sh_type,
            data: data_handle,
            align: section.align(),
            size,
            flags: sh_flags,
            relocs,
        };
        section_map.insert(idx.0, sections.len());
        sections.push(isec);
    }

    let mut symbols = Vec::new();
    let mut symbol_map = FxHashMap::default();

    // Raw ELF symbol table, so we can read `st_type()` (for STT_GNU_IFUNC,
    // which the high-level API folds into SymbolKind::Text).
    let elf_symtab = elf.elf_symbol_table();

    for sym in elf.symbols() {
        let idx = sym.index();
        let name = sym.name_bytes().unwrap_or(b"").to_vec();
        let binding = if sym.scope() == object::SymbolScope::Compilation {
            Binding::Local
        } else if sym.is_weak() {
            Binding::Weak
        } else {
            Binding::Global
        };
        // Read the raw ELF type + visibility (the high-level API folds IFUNC into
        // Text and exposes no visibility). Default to NOTYPE/DEFAULT if absent.
        let raw = elf_symtab.symbol(idx).ok();
        let st_type = raw.map(|s| s.st_type()).unwrap_or(elf::STT_NOTYPE);
        let visibility = raw.map(|s| s.st_visibility()).unwrap_or(elf::STV_DEFAULT);
        let is_ifunc = st_type == elf::STT_GNU_IFUNC;
        let isym = InputSymbol {
            index: idx,
            name,
            binding,
            is_undefined: sym.is_undefined(),
            is_common: sym.is_common(),
            is_ifunc,
            st_type,
            visibility,
            section: sym.section_index(),
            value: sym.address(),
            size: sym.size(),
        };
        symbol_map.insert(idx.0, symbols.len());
        symbols.push(isym);
    }

    let comdat_groups = parse_comdat_groups(&elf);

    Ok(InputObject {
        path,
        sections,
        symbols,
        section_map,
        symbol_map,
        comdat_groups,
    })
}

/// Parse `SHT_GROUP` sections with the `GRP_COMDAT` flag.
fn parse_comdat_groups(elf: &ElfFile64<Endianness>) -> Vec<ComdatGroup> {
    const SHT_GROUP: u32 = 17;
    const GRP_COMDAT: u32 = 0x1;
    let endian = elf.endian();
    let mut groups = Vec::new();
    for section in elf.sections() {
        let hdr = section.elf_section_header();
        if hdr.sh_type(endian) != SHT_GROUP {
            continue;
        }
        let Ok(data) = section.data() else { continue };
        if data.len() < 4 {
            continue;
        }
        let flags = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if flags & GRP_COMDAT == 0 {
            continue;
        }
        let members = data[4..]
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize)
            .collect();
        let sig_idx = hdr.sh_info(endian) as usize;
        let signature = elf
            .symbol_by_index(SymbolIndex(sig_idx))
            .ok()
            .and_then(|s| s.name_bytes().ok())
            .map(|n| n.to_vec())
            .unwrap_or_default();
        groups.push(ComdatGroup { signature, members });
    }
    groups
}

fn classify_section(name: &[u8], flags: u64) -> SectionKind {
    // SHF_MERGE = 0x10, SHF_STRINGS = 0x20, SHF_TLS = 0x400
    const SHF_MERGE: u64 = 0x10;
    const SHF_STRINGS: u64 = 0x20;
    const SHF_TLS: u64 = 0x400;

    if flags & SHF_TLS != 0 {
        return if name.starts_with(b".tbss") {
            SectionKind::Tbss
        } else {
            SectionKind::Tdata
        };
    }

    if flags & SHF_MERGE != 0 {
        return if flags & SHF_STRINGS != 0 {
            SectionKind::MergeString
        } else {
            SectionKind::MergeConst
        };
    }

    if name.starts_with(b".text") {
        SectionKind::Text
    } else if name.starts_with(b".rodata") {
        SectionKind::ReadOnly
    } else if name.starts_with(b".data") {
        SectionKind::Data
    } else if name.starts_with(b".bss") {
        SectionKind::Bss
    } else if is_debug_section_name(name) {
        SectionKind::Debug
    } else if name == b".eh_frame" {
        SectionKind::EhFrame
    } else if name.starts_with(b".init_array") || name.starts_with(b".fini_array") {
        SectionKind::InitArray
    } else {
        SectionKind::Other
    }
}

fn is_debug_section_name(name: &[u8]) -> bool {
    name == b".debug"
        || name.starts_with(b".debug_")
        || name.starts_with(b".zdebug_")
        || name.starts_with(b".gnu.debuglto_")
}

fn normalize_debug_section_name(name: &[u8]) -> Vec<u8> {
    if let Some(rest) = name.strip_prefix(b".zdebug_") {
        let mut normalized = b".debug_".to_vec();
        normalized.extend_from_slice(rest);
        normalized
    } else {
        name.to_vec()
    }
}

fn collect_relocs(section: &object::read::elf::ElfSection64<Endianness>) -> Vec<InputReloc> {
    let mut out = Vec::new();
    for (offset, reloc) in section.relocations() {
        let symbol = match reloc.target() {
            object::RelocationTarget::Symbol(s) => s,
            _ => continue,
        };
        // Use the *raw* ELF relocation type, NOT the `object` crate's generic
        // `RelocationKind` (whose discriminants only coincidentally match a few
        // ELF type numbers).
        let r_type = match reloc.flags() {
            object::RelocationFlags::Elf { r_type } => r_type,
            _ => continue,
        };
        out.push(InputReloc {
            offset,
            r_type,
            symbol,
            addend: reloc.addend(),
        });
    }
    out
}

// ── Archive support ───────────────────────────────────────────────────────────

/// A single member extracted from an `.rlib` / `.a` archive.
pub struct ArchiveMember {
    pub name: String,
    pub data: Vec<u8>,
}

/// Iterate over all members of an `ar`-format archive, yielding raw bytes.
///
/// Used for `.rlib` files: rustc does **not** set modification timestamps on
/// archive members, so byte-level comparison is required for incremental
/// diffing.
pub fn iter_archive_members(path: &Path) -> Result<Vec<ArchiveMember>> {
    let data = std::fs::read(path).map_err(|e| ObjectError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    // Minimal `ar` parser: magic + repeated header+data blocks.
    const MAGIC: &[u8] = b"!<arch>\n";
    if !data.starts_with(MAGIC) {
        return Ok(Vec::new()); // treat non-archive as empty
    }

    let mut members = Vec::new();
    let mut pos = MAGIC.len();

    while pos + 60 <= data.len() {
        let header = &data[pos..pos + 60];
        pos += 60;

        // Identifier: bytes 0..16, right-padded with spaces.
        let name = std::str::from_utf8(&header[0..16])
            .unwrap_or("")
            .trim_end()
            .to_string();

        // Size: bytes 48..58, ASCII decimal.
        let size_str = std::str::from_utf8(&header[48..58]).unwrap_or("0").trim();
        let size: usize = size_str.parse().unwrap_or(0);

        if pos + size > data.len() {
            break;
        }
        let member_data = data[pos..pos + size].to_vec();
        pos += size;
        if !size.is_multiple_of(2) {
            pos += 1; // `ar` pads odd-sized entries to even alignment
        }

        // Skip symbol table and string table pseudo-members.
        if name == "/" || name == "//" || name == "__.SYMDEF" {
            continue;
        }

        members.push(ArchiveMember {
            name,
            data: member_data,
        });
    }

    Ok(members)
}

// ── Shared object (.so) support ─────────────────────────────────────────────────

/// A parsed shared library: the name to record as `DT_NEEDED` and the global
/// symbols it exports (used to satisfy dynamic imports).
#[derive(Debug, Clone)]
pub struct SharedObject {
    pub soname: String,
    pub exports: Vec<Vec<u8>>,
    /// For each export, the symbol version string it is defined at (e.g.
    /// `GLIBC_2.34`), or `None` for an unversioned/base definition. Parallel to
    /// `exports`. Used to synthesise `.gnu.version_r` so the dynamic loader
    /// binds the correct versioned definition.
    pub export_versions: Vec<Option<Vec<u8>>>,
    /// Full metadata for each dynamic export, parallel to `exports`.
    pub export_symbols: Vec<SharedExport>,
}

/// Metadata for one exported dynamic symbol in a shared object.
#[derive(Debug, Clone)]
pub struct SharedExport {
    pub name: Vec<u8>,
    pub version: Option<Vec<u8>>,
    pub size: u64,
    pub st_type: u8,
}

/// Count the FDE (Frame Description Entry) records in a `.eh_frame` section.
///
/// `.eh_frame` is a sequence of length-prefixed records: a 4-byte length (0
/// terminates; 0xffffffff introduces a 8-byte extended length), then a 4-byte
/// CIE pointer that is 0 for a CIE and non-zero for an FDE. We count the FDEs
/// to size `.eh_frame_hdr`.
pub fn count_fdes(data: &[u8]) -> usize {
    let (cies, fdes, terms, leftover) = scan_eh_frame(data);
    tracing::trace!(
        len = data.len(),
        cies,
        fdes,
        terms,
        leftover,
        "count_fdes: scanned input .eh_frame"
    );
    fdes
}

/// Returns `true` if a `.eh_frame` blob ends with a genuine 4-byte zero
/// terminator record (reached at a record boundary), as opposed to an FDE whose
/// final bytes merely happen to be zero. Walks the records to the last one.
pub fn ends_with_eh_terminator(data: &[u8]) -> bool {
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if len == 0 {
            // A zero-length record at a boundary: terminator iff it is the very
            // last 4 bytes of the section.
            return pos + 4 == data.len();
        }
        if len == 0xffff_ffff {
            return false;
        }
        let rec_start = pos + 4;
        if rec_start + len > data.len() {
            return false; // malformed; don't strip
        }
        pos = rec_start + len;
    }
    false
}

/// Scan a `.eh_frame` blob, returning `(cie_count, fde_count, terminators,
/// leftover_bytes)`. Shared by [`count_fdes`] and diagnostics so both paths use
/// identical record-walking logic.
pub fn scan_eh_frame(data: &[u8]) -> (usize, usize, usize, usize) {
    let mut pos = 0usize;
    let (mut cies, mut fdes, mut terms) = (0usize, 0usize, 0usize);
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if len == 0 {
            terms += 1;
            pos += 4;
            continue;
        }
        if len == 0xffff_ffff {
            break;
        }
        let rec_start = pos + 4;
        if rec_start + 4 > data.len() || rec_start + len > data.len() {
            break;
        }
        let cie_ptr = u32::from_le_bytes(data[rec_start..rec_start + 4].try_into().unwrap());
        if cie_ptr == 0 {
            cies += 1;
        } else {
            fdes += 1;
        }
        pos = rec_start + len;
    }
    (cies, fdes, terms, data.len().saturating_sub(pos))
}

/// Iterate the FDEs in a `.eh_frame`, yielding each FDE record's byte offset
/// within the section and the `PC begin` field's signed 4-byte value at that
/// offset+8 (a PC-relative reference to the described function). Returns
/// `(fde_section_offset, pc_begin_field_offset, pc_begin_rel)` per FDE.
pub fn iter_fdes(data: &[u8]) -> Vec<(usize, usize, i64)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if len == 0 {
            // CIE terminator between concatenated input `.eh_frame`s; skip it.
            pos += 4;
            continue;
        }
        if len == 0xffff_ffff {
            break;
        }
        let rec_start = pos + 4;
        if rec_start + 8 > data.len() || rec_start + len > data.len() {
            break;
        }
        let cie_ptr = u32::from_le_bytes(data[rec_start..rec_start + 4].try_into().unwrap());
        if cie_ptr != 0 {
            // FDE: `PC begin` follows the CIE pointer (4 bytes in), encoded
            // (for the GCC default) as DW_EH_PE_pcrel|sdata4 — a signed 4-byte
            // offset relative to its own location.
            let pcbegin_off = rec_start + 4;
            let rel =
                i32::from_le_bytes(data[pcbegin_off..pcbegin_off + 4].try_into().unwrap()) as i64;
            out.push((pos, pcbegin_off, rel));
        }
        pos = rec_start + len;
    }
    out
}

/// Read `DT_SONAME` from a parsed shared object, if present.
fn elf_soname(elf: &ElfFile64<Endianness>, data: &[u8]) -> Option<String> {
    use object::read::elf::Dyn;
    let endian = elf.endian();
    let (dynamic, dyn_index) = elf.elf_section_table().dynamic(endian, data).ok()??;
    // The string table referenced by the dynamic section.
    let strings = elf
        .elf_section_table()
        .strings(endian, data, dyn_index)
        .ok()?;
    for d in dynamic {
        if d.tag32(endian) == Some(elf::DT_SONAME as u32) {
            if let Ok(name) = d.string(endian, strings) {
                return Some(String::from_utf8_lossy(name).into_owned());
            }
        }
    }
    None
}

/// Returns true if `path` is an ELF shared object (`ET_DYN`).
pub fn is_shared_object(path: &Path) -> bool {
    matches!(classify_file(path), FileKind::Shared)
}

/// Coarse input-file classification from the leading bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A relocatable object (`ET_REL`) or anything that isn't an archive/shared.
    Bare,
    /// An `ar` archive (`!<arch>\n`).
    Archive,
    /// An ELF shared object (`ET_DYN`).
    Shared,
}

/// Classify an input by reading ONLY its 20-byte header — a single open+read,
/// replacing the former `is_archive` + `is_shared_object` double-open per input.
/// On a 23-object Rust link that halves the classification syscalls.
pub fn classify_file(path: &Path) -> FileKind {
    use std::io::Read;
    let mut hdr = [0u8; 20];
    let Ok(mut f) = std::fs::File::open(path) else {
        return FileKind::Bare;
    };
    // A short read still lets us inspect what we got (archives are 8 bytes).
    let n = f.read(&mut hdr).unwrap_or(0);
    classify_bytes(&hdr[..n])
}

/// Classify from already-loaded leading bytes — the zero-syscall variant used
/// when an input has been mmap'd once and is reused across phases (see
/// [`MappedInput`]). Identical logic to [`classify_file`] minus the open+read.
pub fn classify_bytes(h: &[u8]) -> FileKind {
    if h.len() >= 8 && &h[0..8] == b"!<arch>\n" {
        return FileKind::Archive;
    }
    if h.len() >= 18 && &h[0..4] == b"\x7fELF" && u16::from_le_bytes([h[16], h[17]]) == 3 {
        return FileKind::Shared;
    }
    FileKind::Bare
}

/// A single read-only memory map of an input file, opened ONCE and reused for
/// every phase that needs its bytes (classification, `_start` probe, parse).
/// Previously each input was opened 3× — once per phase. Holding the `Mmap`
/// keeps `bytes()` valid for the map's lifetime without copying the file.
pub struct MappedInput {
    _mmap: Mmap,
    // A raw pointer + len would alias the mmap; instead expose via accessor.
}

impl MappedInput {
    /// mmap `path` once. Returns `None` if the file can't be opened/mapped (an
    /// empty file can't be mmap'd; callers treat that as a 0-byte input).
    pub fn open(path: &Path) -> Option<MappedInput> {
        let file = std::fs::File::open(path).ok()?;
        // SAFETY: read-only view; never mutated, dropped when MappedInput drops.
        let mmap = unsafe { Mmap::map(&file) }.ok()?;
        Some(MappedInput { _mmap: mmap })
    }

    /// The mapped bytes.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self._mmap
    }

    /// Consume the wrapper and yield the underlying map, e.g. to move it into an
    /// [`InputArena`] so sections can borrow from it for the whole link.
    #[inline]
    pub fn into_mmap(self) -> Mmap {
        self._mmap
    }
}

/// Does this object define a non-local, defined `_start`? Used by the driver to
/// decide whether to inject the C runtime. This mmaps and walks ONLY the symbol
/// table — no section copy, no relocation parse — so it is far cheaper than a
/// full [`parse_object`] (which the driver formerly ran over every input just to
/// answer this yes/no question).
pub fn object_defines_global_start(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    // SAFETY: read-only view; we never mutate and drop it before returning.
    let Ok(mmap) = (unsafe { Mmap::map(&file) }) else {
        return false;
    };
    let Ok(elf) = ElfFile64::<Endianness>::parse(&*mmap) else {
        return false;
    };
    elf.symbols()
        .any(|s| s.name_bytes() == Ok(b"_start") && !s.is_undefined() && s.is_global())
}

/// Parse a shared object's exported dynamic symbols and its `DT_SONAME`
/// (falling back to the file's base name).
pub fn parse_shared_object(path: &Path) -> Result<SharedObject> {
    let data = std::fs::read(path).map_err(|e| ObjectError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let elf: ElfFile64<Endianness> =
        ElfFile64::parse(data.as_slice()).map_err(|e| ObjectError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;

    // Build the symbol-version table so each export can be tagged with its
    // version string (needed to synthesise `.gnu.version_r` in the output).
    let endian = elf.endian();
    let versions = elf
        .elf_section_table()
        .versions(endian, data.as_slice())
        .ok()
        .flatten();

    let mut exports = Vec::new();
    let mut export_versions = Vec::new();
    let mut export_symbols = Vec::new();
    for sym in elf.dynamic_symbols() {
        if sym.is_undefined() || sym.is_local() {
            continue;
        }
        let Ok(name) = sym.name_bytes() else { continue };
        if name.is_empty() {
            continue;
        }
        // Resolve this symbol's version name, if the .so carries version info.
        let ver = versions.as_ref().and_then(|vt| {
            let vidx = vt.version_index(endian, sym.index());
            match vt.version(vidx) {
                Ok(Some(v)) => Some(v.name().to_vec()),
                _ => None,
            }
        });
        let name = name.to_vec();
        let st_type = match sym.kind() {
            object::SymbolKind::Text => elf::STT_FUNC,
            object::SymbolKind::Data => elf::STT_OBJECT,
            object::SymbolKind::Tls => elf::STT_TLS,
            object::SymbolKind::Section => elf::STT_SECTION,
            object::SymbolKind::File => elf::STT_FILE,
            _ => elf::STT_NOTYPE,
        };
        export_symbols.push(SharedExport {
            name: name.clone(),
            version: ver.clone(),
            size: sym.size(),
            st_type,
        });
        exports.push(name);
        export_versions.push(ver);
    }

    // Prefer the embedded DT_SONAME over the file name.
    let soname = elf_soname(&elf, data.as_slice()).unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    });

    Ok(SharedObject {
        soname,
        exports,
        export_versions,
        export_symbols,
    })
}

// ── ELF64 format constants ──────────────────────────────────────────────────────

/// Authoritative ELF64 + x86-64 constants used by the layout and emit passes.
///
/// Values are taken verbatim from the ELF-64 / TIS ELF specs and the AMD64 SysV
/// ABI (see `research/SPEC_AND_LITERATURE_DIGEST.md` §A–B).
pub mod elf {
    // ── e_ident ──
    pub const ELFMAG: [u8; 4] = [0x7f, b'E', b'L', b'F'];
    pub const ELFCLASS64: u8 = 2;
    pub const ELFDATA2LSB: u8 = 1;
    pub const EV_CURRENT: u8 = 1;
    pub const ELFOSABI_SYSV: u8 = 0;

    // ── e_type ──
    pub const ET_REL: u16 = 1;
    pub const ET_EXEC: u16 = 2;
    pub const ET_DYN: u16 = 3;

    // ── e_machine ──
    pub const EM_X86_64: u16 = 62;

    // ── fixed record sizes ──
    pub const EHDR_SIZE: u64 = 64;
    pub const PHDR_SIZE: u64 = 56;
    pub const SHDR_SIZE: u64 = 64;
    pub const SYM_SIZE: u64 = 24;
    pub const RELA_SIZE: u64 = 24;

    // ── p_type ──
    pub const PT_NULL: u32 = 0;
    pub const PT_LOAD: u32 = 1;
    pub const PT_DYNAMIC: u32 = 2;
    pub const PT_INTERP: u32 = 3;
    pub const PT_NOTE: u32 = 4;
    pub const PT_PHDR: u32 = 6;
    pub const PT_TLS: u32 = 7;
    pub const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
    pub const PT_GNU_STACK: u32 = 0x6474_e551;
    pub const PT_GNU_RELRO: u32 = 0x6474_e552;
    pub const PT_GNU_PROPERTY: u32 = 0x6474_e553;

    // ── p_flags ──
    pub const PF_X: u32 = 0x1;
    pub const PF_W: u32 = 0x2;
    pub const PF_R: u32 = 0x4;

    // ── sh_type ──
    pub const SHT_NULL: u32 = 0;
    pub const SHT_PROGBITS: u32 = 1;
    pub const SHT_SYMTAB: u32 = 2;
    pub const SHT_STRTAB: u32 = 3;
    pub const SHT_RELA: u32 = 4;
    pub const SHT_HASH: u32 = 5;
    pub const SHT_DYNAMIC: u32 = 6;
    pub const SHT_NOTE: u32 = 7;
    pub const SHT_NOBITS: u32 = 8;
    pub const SHT_DYNSYM: u32 = 11;
    pub const SHT_INIT_ARRAY: u32 = 14;
    pub const SHT_FINI_ARRAY: u32 = 15;
    pub const SHT_PREINIT_ARRAY: u32 = 16;
    pub const SHT_GNU_HASH: u32 = 0x6fff_fff6;
    pub const SHT_GNU_VERDEF: u32 = 0x6fff_fffd;
    pub const SHT_GNU_VERNEED: u32 = 0x6fff_fffe;
    pub const SHT_GNU_VERSYM: u32 = 0x6fff_ffff;
    pub const SHT_SYMTAB_SHNDX: u32 = 18;

    /// GNU build-id note type.
    pub const NT_GNU_BUILD_ID: u32 = 3;

    // ── dynamic section tags (Elf64_Dyn d_tag) ──
    pub const DT_NULL: i64 = 0;
    pub const DT_NEEDED: i64 = 1;
    pub const DT_PLTRELSZ: i64 = 2;
    pub const DT_PLTGOT: i64 = 3;
    pub const DT_HASH: i64 = 4;
    pub const DT_PLTREL: i64 = 20;
    pub const DT_JMPREL: i64 = 23;
    pub const DT_STRTAB: i64 = 5;
    pub const DT_SONAME: i64 = 14;
    pub const DT_SYMTAB: i64 = 6;
    pub const DT_RELA: i64 = 7;
    pub const DT_RELASZ: i64 = 8;
    pub const DT_RELAENT: i64 = 9;
    pub const DT_STRSZ: i64 = 10;
    pub const DT_SYMENT: i64 = 11;
    pub const DT_INIT: i64 = 12;
    pub const DT_FINI: i64 = 13;
    pub const DT_INIT_ARRAY: i64 = 25;
    pub const DT_FINI_ARRAY: i64 = 26;
    pub const DT_INIT_ARRAYSZ: i64 = 27;
    pub const DT_FINI_ARRAYSZ: i64 = 28;
    pub const DT_FLAGS: i64 = 30;
    pub const DT_RELACOUNT: i64 = 0x6fff_fff9;
    pub const DT_GNU_HASH: i64 = 0x6fff_fef5;
    pub const DT_FLAGS_1: i64 = 0x6fff_fffb;
    pub const DT_VERSYM: i64 = 0x6fff_fff0;
    pub const DT_VERNEED: i64 = 0x6fff_fffe;
    pub const DT_VERNEEDNUM: i64 = 0x6fff_ffff;
    pub const DF_BIND_NOW: u64 = 0x8;
    pub const DF_1_NOW: u64 = 0x1;
    pub const DF_1_PIE: u64 = 0x0800_0000;

    // ── dynamic relocation types ──
    pub const R_X86_64_64: u32 = 1;
    pub const R_X86_64_COPY: u32 = 5;
    pub const R_X86_64_GLOB_DAT: u32 = 6;
    pub const R_X86_64_JUMP_SLOT: u32 = 7;
    pub const R_X86_64_RELATIVE: u32 = 8;
    pub const R_X86_64_IRELATIVE: u32 = 37;

    /// Default ELF interpreter for x86-64 glibc.
    pub const DEFAULT_INTERP: &[u8] = b"/lib64/ld-linux-x86-64.so.2\0";

    // ── sh_flags ──
    pub const SHF_WRITE: u64 = 0x1;
    pub const SHF_ALLOC: u64 = 0x2;
    pub const SHF_EXECINSTR: u64 = 0x4;
    pub const SHF_MERGE: u64 = 0x10;
    pub const SHF_STRINGS: u64 = 0x20;
    pub const SHF_TLS: u64 = 0x400;

    // ── special section indices ──
    pub const SHN_UNDEF: u16 = 0;
    pub const SHN_LORESERVE: u16 = 0xff00;
    pub const SHN_ABS: u16 = 0xfff1;
    pub const SHN_COMMON: u16 = 0xfff2;
    pub const SHN_XINDEX: u16 = 0xffff;

    // ── symbol binding / type / visibility ──
    pub const STB_LOCAL: u8 = 0;
    pub const STB_GLOBAL: u8 = 1;
    pub const STB_WEAK: u8 = 2;
    pub const STT_NOTYPE: u8 = 0;
    pub const STT_OBJECT: u8 = 1;
    pub const STT_FUNC: u8 = 2;
    pub const STT_GNU_IFUNC: u8 = 10;
    pub const STT_SECTION: u8 = 3;
    pub const STT_FILE: u8 = 4;
    pub const STT_TLS: u8 = 6;
    pub const STV_DEFAULT: u8 = 0;
    pub const STV_INTERNAL: u8 = 1;
    pub const STV_HIDDEN: u8 = 2;
    pub const STV_PROTECTED: u8 = 3;

    /// `st_info = (bind << 4) | (type & 0xf)`.
    #[inline]
    pub const fn st_info(bind: u8, typ: u8) -> u8 {
        (bind << 4) | (typ & 0xf)
    }
}

#[cfg(test)]
mod dyn_probe {
    #[test]
    fn probe_shared() {
        // smoke: parsing a system libc.so.6 should yield exports
        for p in [
            "/lib/x86_64-linux-gnu/libc.so.6",
            "/usr/lib/x86_64-linux-gnu/libc.so.6",
            "/lib64/libc.so.6",
        ] {
            let path = std::path::Path::new(p);
            if path.exists() {
                let r = super::parse_shared_object(path).unwrap();
                assert!(!r.exports.is_empty(), "libc should export symbols");
                assert!(
                    r.exports.iter().any(|e| e == b"printf"),
                    "libc should export printf"
                );
                return;
            }
        }
    }

    /// `is_shared_object` reads only the 20-byte ELF header. Exercise its
    /// fast-path edge cases (the auto-review flagged them as untested): a
    /// too-short file, a non-ELF file, and a relocatable object (ET_REL) must all
    /// classify as NOT shared; only ET_DYN is shared.
    #[test]
    fn is_shared_object_edge_cases() {
        let dir = std::env::temp_dir().join(format!("peony-isso-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let short = dir.join("short");
        std::fs::write(&short, b"\x7fELF").unwrap(); // < 20 bytes → false
        assert!(!super::is_shared_object(&short));

        let nonelf = dir.join("nonelf");
        std::fs::write(&nonelf, vec![0u8; 64]).unwrap(); // bad magic → false
        assert!(!super::is_shared_object(&nonelf));

        // A minimal 20-byte header with ELF magic and e_type = ET_REL (1).
        let mut rel = vec![0u8; 20];
        rel[0..4].copy_from_slice(b"\x7fELF");
        rel[16] = 1; // e_type = ET_REL
        let relp = dir.join("rel");
        std::fs::write(&relp, &rel).unwrap();
        assert!(!super::is_shared_object(&relp), "ET_REL is not shared");

        rel[16] = 3; // e_type = ET_DYN
        let dynp = dir.join("dyn");
        std::fs::write(&dynp, &rel).unwrap();
        assert!(super::is_shared_object(&dynp), "ET_DYN is shared");

        let missing = dir.join("nope");
        assert!(!super::is_shared_object(&missing), "missing file → false");

        std::fs::remove_dir_all(&dir).ok();
    }
}
