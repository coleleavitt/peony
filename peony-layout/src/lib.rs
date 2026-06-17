//! `peony-layout` — Section grouping, address assignment, segments, synthetic
//! sections, and the output section-header plan.
//!
//! Implements MaskRay's linker passes 5 and 8 plus the synthetic-section
//! creation needed to emit a *loadable* static ELF executable:
//!
//! * Group input sections into output sections by name prefix.
//! * Classify each into a permission class (RO / RX / RW) and lay them out into
//!   page-aligned `PT_LOAD` segments, reserving the ELF + program headers in the
//!   first read-only segment.
//! * Maintain the **page-congruence invariant** `file_offset == vaddr - base`
//!   (with a page-aligned base), so every loadable byte satisfies
//!   `p_vaddr ≡ p_offset (mod page)` automatically (see
//!   `research/SPEC_AND_LITERATURE_DIGEST.md` §A and mold's `align_with_skew`).
//! * Synthesise `.got` from the relocation scan, and `.symtab` / `.strtab` /
//!   `.shstrtab`, and build the section-header table plan.
//!
//! After layout, [`finalize_symbols`] writes each symbol's virtual address
//! (`section_address + value`) and GOT slot address back into the symbol table,
//! and [`check_undefined`] rejects unresolved references.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use peony_object::{InputObject, SectionKind, elf};
use peony_symbols::{SymbolId, SymbolTable};
use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;
use ws_deque::{Steal, Worker};

// ── S3-GC grain size constant (SPEC §9) ─────────────────────────────────────
const S3GC_GRAIN_SIZE: usize = 256;

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum LayoutError {
    #[error("section alignment {align} is not a power of two in `{object}`")]
    BadAlignment { align: u64, object: String },
    #[error("output would exceed the address space")]
    AddressOverflow,
    #[error("no entry point: symbol `{0}` is not defined")]
    NoEntry(String),
    #[error("undefined symbol `{0}`")]
    Undefined(String),
    #[error(
        "internal error: program-header count mismatch (predicted {predicted}, emitted {actual})"
    )]
    PhdrCountMismatch { predicted: u64, actual: u64 },
}

pub type Result<T> = std::result::Result<T, LayoutError>;

// ── Output model ────────────────────────────────────────────────────────────

/// A contribution from one input section to an output section.
#[derive(Debug, Clone)]
pub struct SectionContribution {
    pub object_id: usize,
    pub section_index: usize,
    /// Offset of this contribution within the output section.
    pub offset: u64,
    pub size: u64,
}

/// How an output section's bytes are produced at emit time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecSource {
    /// Concatenated input-section bytes (from `contributions`).
    Input,
    /// `.bss` — occupies memory but no file bytes.
    Bss,
    /// `.got` — one 8-byte slot per entry in [`Layout::got_slots`].
    Got,
    /// `.symtab` — built by emit from the symbol table + [`Layout::symtab`].
    SymTab,
    /// `.strtab` — verbatim [`Layout::strtab`].
    StrTab,
    /// `.shstrtab` — verbatim [`Layout::shstrtab`].
    ShStrTab,
    /// `.note.gnu.build-id` — emit writes the note header + content hash.
    NoteBuildId,
    /// Dynamic-linking sections — emit writes the corresponding blob from
    /// [`Layout::dyn_blobs`].
    Interp,
    Hash,
    DynSym,
    DynStr,
    RelaDyn,
    Dynamic,
    Plt,
    GotPlt,
    RelaPlt,
    /// `.gnu.version` — one `Elf64_Half` per `.dynsym` entry.
    GnuVersion,
    /// `.gnu.version_r` — verneed records for the required library versions.
    GnuVersionR,
    /// `.eh_frame_hdr` — sorted FDE binary-search table (PT_GNU_EH_FRAME).
    EhFrameHdr,
    /// `.gnu.hash` — GNU-style dynamic symbol hash table (DT_GNU_HASH).
    GnuHash,
}

/// Inputs needed to lay out a dynamic executable.
#[derive(Debug, Clone, Default)]
pub struct DynamicInfo {
    /// Imported symbol names, in `.dynsym` order (dynsym index = position + 1).
    pub imports: Vec<Vec<u8>>,
    /// Per-import version requirement, parallel to `imports`: the version string
    /// (e.g. `GLIBC_2.34`), or `None` for an unversioned import. Drives
    /// `.gnu.version` / `.gnu.version_r`.
    pub import_versions: Vec<Option<Vec<u8>>>,
    /// Per-import providing-library soname, parallel to `imports` (e.g.
    /// `libc.so.6`). Groups version requirements per-library in `.gnu.version_r`.
    pub import_sonames: Vec<Option<String>>,
    /// `DT_NEEDED` shared-library names.
    pub needed: Vec<String>,
    /// True when producing a position-independent executable (ET_DYN). PIE
    /// requires `R_X86_64_RELATIVE` dynamic relocations for every absolute
    /// pointer the loader must bias.
    pub pie: bool,
    /// Number of `R_X86_64_RELATIVE` relocations to reserve space for in
    /// `.rela.dyn` (computed by the driver via `peony_reloc::count_relative`).
    /// The actual entries are filled post-layout from
    /// `peony_reloc::collect_relative` and emitted by `peony-emit`.
    pub n_relative: usize,
}

/// Pre-built byte blobs for the dynamic-linking sections (filled by layout,
/// written verbatim by emit).
#[derive(Debug, Default)]
pub struct DynBlobs {
    pub interp: Vec<u8>,
    pub hash: Vec<u8>,
    pub dynsym: Vec<u8>,
    pub dynstr: Vec<u8>,
    pub rela_dyn: Vec<u8>,
    pub dynamic: Vec<u8>,
    pub plt: Vec<u8>,
    pub got_plt: Vec<u8>,
    pub rela_plt: Vec<u8>,
    pub gnu_version: Vec<u8>,
    pub gnu_version_r: Vec<u8>,
    /// Number of verneed entries (DT_VERNEEDNUM).
    pub verneed_num: u64,
    /// GLOB_DAT relocations, assembled into `rela_dyn` after the RELATIVE
    /// entries by `Layout::append_relative_relocs`.
    pub rela_glob_dat: Vec<u8>,
    /// `.eh_frame_hdr` bytes, built post-layout by `Layout::build_eh_frame_hdr`.
    pub eh_frame_hdr: Vec<u8>,
    /// `.gnu.hash` GNU-style dynamic symbol hash table.
    pub gnu_hash: Vec<u8>,
}

/// A merged output section with a full section-header plan.
#[derive(Debug, Clone)]
pub struct OutputSection {
    pub name: String,
    pub kind: SectionKind,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub sh_addr: u64,
    pub sh_offset: u64,
    pub sh_size: u64,
    pub sh_link: u32,
    pub sh_info: u32,
    pub sh_addralign: u64,
    pub sh_entsize: u64,
    /// Offset of this section's name in `.shstrtab` (filled during layout).
    pub sh_name: u32,
    /// Index into the section-header table (= position in `output_sections` + 1).
    pub shndx: u32,
    pub contributions: Vec<SectionContribution>,
    pub source: SecSource,
}

/// One ELF program header (segment).
#[derive(Debug, Clone)]
pub struct ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// A planned `.symtab` entry. For globals, `st_value`/`st_size` are resolved at
/// emit time from the symbol table (after [`finalize_symbols`]); for locals they
/// are precomputed here (`local`).
#[derive(Debug, Clone)]
pub struct SymEntry {
    pub name: Vec<u8>,
    pub name_off: u32,
    pub shndx: u16,
    pub info: u8,
    /// `Some((value, size))` for a local symbol; `None` to look up by name.
    pub local: Option<(u64, u64)>,
}

/// The fully resolved output layout consumed by `peony-emit`.
pub struct Layout {
    pub output_sections: Vec<OutputSection>,
    /// (object_id, input_section_index) → virtual address of that placement.
    pub addresses: FxHashMap<(usize, usize), u64>,
    pub segments: Vec<ProgramHeader>,
    pub entry: u64,
    pub e_type: u16,
    pub phoff: u64,
    pub phnum: u64,
    pub shoff: u64,
    pub shnum: u64,
    pub shstrndx: u64,
    pub file_size: u64,
    /// GOT slots in order (symbol per 8-byte slot).
    pub got_slots: Vec<SymbolId>,
    pub got_base: u64,
    pub strtab: Vec<u8>,
    pub shstrtab: Vec<u8>,
    pub symtab: Vec<SymEntry>,
    // Addresses for linker-defined symbols.
    pub image_base: u64,
    pub bss_start: u64,
    pub edata: u64,
    pub end: u64,
    /// Final virtual address of each common symbol (by id).
    pub common: Vec<(SymbolId, u64)>,
    /// Pre-built dynamic-section bytes (empty for a static link).
    pub dyn_blobs: DynBlobs,
    /// PLT stub symbols in slot order, and the `.plt` base address.
    pub plt_slots: Vec<SymbolId>,
    pub plt_base: u64,
    /// Total size of the static TLS block (aligned); 0 if no TLS.
    pub tls_size: u64,
    /// (object_id, input_section_index) → offset within the TLS block.
    pub tls_offsets: FxHashMap<(usize, usize), u64>,
}

impl Layout {
    /// TLS-block offset of an input TLS section's placement.
    pub fn tls_offset(&self, object_id: usize, section_index: usize) -> Option<u64> {
        self.tls_offsets.get(&(object_id, section_index)).copied()
    }
}

impl Layout {
    pub fn address_of(&self, object_id: usize, section_index: usize) -> Option<u64> {
        self.addresses.get(&(object_id, section_index)).copied()
    }

    /// Fill the reserved leading `R_X86_64_RELATIVE` prefix of `.rela.dyn`.
    ///
    /// `relative` is `(site_vaddr, link_time_target_vaddr)` from
    /// [`peony_reloc::collect_relative`]. Each becomes a RELA entry with
    /// `r_info = R_X86_64_RELATIVE` (symbol 0) and `r_addend = target`, so the
    /// loader writes `*site = load_base + target`. These occupy the zero-filled
    /// prefix `compute_layout` reserved (RELATIVE entries precede GLOB_DAT so
    /// glibc's DT_RELACOUNT fast path stays valid). `relative.len()` must not
    /// exceed the reserved `n_relative`; any unused prefix stays `R_X86_64_NONE`.
    pub fn append_relative_relocs(&mut self, relative: &[(u64, u64)]) {
        self.append_dynamic_relocs(relative, &[]);
    }

    /// Assemble `.rela.dyn`: `R_X86_64_RELATIVE` entries first (covered by
    /// `DT_RELACOUNT`), then `R_X86_64_IRELATIVE` IFUNC entries, then the stashed
    /// `R_X86_64_GLOB_DAT` imports — with no type-0 gap anywhere (BFD/eu-elflint
    /// reject `NONE` records in `.rela.dyn`). `irelative` is `(got_slot_va,
    /// resolver_va)`; the loader runs the resolver and stores its result.
    pub fn append_dynamic_relocs(&mut self, relative: &[(u64, u64)], irelative: &[(u64, u64)]) {
        let cap = self.rela_dyn_section_size() as usize;
        let glob_len = self.dyn_blobs.rela_glob_dat.len();
        let mut bytes = Vec::with_capacity(cap);
        let mut written = 0u64;
        for &(site, target) in relative {
            if bytes.len() + elf::RELA_SIZE as usize > cap - glob_len {
                break; // never crowd out the IRELATIVE/GLOB_DAT suffixes
            }
            let r_info = elf::R_X86_64_RELATIVE as u64; // symbol index 0
            bytes.extend_from_slice(&site.to_le_bytes());
            bytes.extend_from_slice(&r_info.to_le_bytes());
            bytes.extend_from_slice(&(target as i64).to_le_bytes());
            written += 1;
        }
        // IRELATIVE: IFUNC GOT slots resolved by running the resolver at startup.
        for &(site, resolver) in irelative {
            if bytes.len() + elf::RELA_SIZE as usize > cap - glob_len {
                break;
            }
            let r_info = elf::R_X86_64_IRELATIVE as u64; // symbol index 0
            bytes.extend_from_slice(&site.to_le_bytes());
            bytes.extend_from_slice(&r_info.to_le_bytes());
            bytes.extend_from_slice(&(resolver as i64).to_le_bytes());
        }
        let glob = std::mem::take(&mut self.dyn_blobs.rela_glob_dat);
        bytes.extend_from_slice(&glob);
        let actual_sz = bytes.len() as u64;
        self.dyn_blobs.rela_dyn = bytes;
        // Correct DT_RELACOUNT (leading relatives) and DT_RELASZ (actual bytes),
        // and shrink the .rela.dyn section header to the real content so there
        // are no trailing NONE entries.
        self.patch_dt_relacount(written);
        self.patch_dt_relasz(actual_sz);
        self.set_rela_dyn_size(actual_sz);
    }

    /// Build `.eh_frame_hdr`: the version-1 header plus a binary-search table of
    /// `(initial_location, fde_address)` pairs sorted by PC, both encoded as
    /// 4-byte signed offsets relative to the start of `.eh_frame_hdr`
    /// (DW_EH_PE_datarel | sdata4). Must run after layout assigns addresses.
    ///
    /// `objects` supplies the input `.eh_frame` bytes; `self` supplies the
    /// output addresses of each `.eh_frame` contribution.
    pub fn build_eh_frame_hdr(&mut self, objects: &[InputObject]) {
        let Some(hdr_va) = self
            .output_sections
            .iter()
            .find(|s| s.source == SecSource::EhFrameHdr)
            .map(|s| s.sh_addr)
        else {
            return;
        };
        let Some(eh) = self.output_sections.iter().find(|s| s.name == ".eh_frame") else {
            return;
        };
        let eh_va = eh.sh_addr;

        // Collect (func_pc, fde_va) for every FDE, computing absolute addresses.
        let mut entries: Vec<(i64, i64)> = Vec::new();
        for c in &eh.contributions {
            let Some(obj) = objects.get(c.object_id) else {
                continue;
            };
            let Some(&pos) = obj.section_map.get(&c.section_index) else {
                continue;
            };
            let Some(isec) = obj.sections.get(pos) else {
                continue;
            };
            let contrib_va = eh_va + c.offset;
            for (fde_off, pcbegin_off, rel) in peony_object::iter_fdes(&isec.data) {
                // PC begin is pcrel to its own field location.
                let pcbegin_va = contrib_va + pcbegin_off as u64;
                let func_pc = (pcbegin_va as i64).wrapping_add(rel);
                let fde_va = (contrib_va + fde_off as u64) as i64;
                entries.push((func_pc, fde_va));
            }
        }
        entries.sort_by_key(|&(pc, _)| pc);

        // Header (DW_EH_PE constants): eh_frame_ptr = pcrel|sdata4 (0x1b),
        // fde_count = udata4 (0x03), table = datarel|sdata4 (0x3b).
        let mut hdr = Vec::with_capacity(12 + entries.len() * 8);
        hdr.push(1u8); // version
        hdr.push(0x1b); // eh_frame_ptr_enc
        hdr.push(0x03); // fde_count_enc
        hdr.push(0x3b); // table_enc
        // eh_frame_ptr: offset to .eh_frame relative to this field (pcrel).
        let efp_field_va = hdr_va + 4;
        hdr.extend_from_slice(&((eh_va as i64 - efp_field_va as i64) as i32).to_le_bytes());
        hdr.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (func_pc, fde_va) in &entries {
            hdr.extend_from_slice(&((func_pc - hdr_va as i64) as i32).to_le_bytes());
            hdr.extend_from_slice(&((fde_va - hdr_va as i64) as i32).to_le_bytes());
        }
        self.dyn_blobs.eh_frame_hdr = hdr;
    }

    /// The reserved `.rela.dyn` section size from layout (full prefix + globs).
    fn rela_dyn_section_size(&self) -> u64 {
        self.output_sections
            .iter()
            .find(|s| s.source == SecSource::RelaDyn)
            .map(|s| s.sh_size)
            .unwrap_or(0)
    }

    /// Shrink the `.rela.dyn` section header size to the actual content.
    fn set_rela_dyn_size(&mut self, size: u64) {
        if let Some(s) = self
            .output_sections
            .iter_mut()
            .find(|s| s.source == SecSource::RelaDyn)
        {
            s.sh_size = size;
        }
    }

    fn patch_dt_relasz(&mut self, size: u64) {
        self.patch_dt_tag(elf::DT_RELASZ, size);
    }

    fn patch_dt_relacount(&mut self, count: u64) {
        self.patch_dt_tag(elf::DT_RELACOUNT, count);
    }

    /// Overwrite the value of a `DT_*` entry in the `.dynamic` blob.
    fn patch_dt_tag(&mut self, tag: i64, value: u64) {
        let dynbytes = &mut self.dyn_blobs.dynamic;
        let mut i = 0;
        while i + 16 <= dynbytes.len() {
            let t = i64::from_le_bytes(dynbytes[i..i + 8].try_into().unwrap());
            if t == tag {
                dynbytes[i + 8..i + 16].copy_from_slice(&value.to_le_bytes());
                return;
            }
            if t == elf::DT_NULL {
                return;
            }
            i += 16;
        }
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

pub struct LayoutConfig {
    pub base_address: u64,
    pub page_size: u64,
    /// Name of the entry symbol (default `_start`).
    pub entry_symbol: String,
    /// Emit a `.note.gnu.build-id` note (+ `PT_NOTE`).
    pub build_id: bool,
    /// Omit `.symtab`/`.strtab` from the output (`-s`).
    pub strip: bool,
    /// Produce a position-independent executable (`ET_DYN`, base 0); the kernel
    /// applies a load bias, so only PC-relative code/data is valid.
    pub pie: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            base_address: 0x40_0000,
            page_size: 0x1000,
            entry_symbol: "_start".to_string(),
            build_id: false,
            strip: false,
            pie: false,
        }
    }
}

/// Size of a `.note.gnu.build-id` note: namesz+descsz+type (12) + "GNU\0" (4)
/// + a 16-byte hash.
pub const BUILD_ID_NOTE_SIZE: u64 = 12 + 4 + 16;
/// Byte length of the build-id hash.
pub const BUILD_ID_LEN: usize = 16;

// ── Permission classes ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Perm {
    Ro,
    Rx,
    Rw,
}

/// An output-section being assembled during grouping.
struct Builder {
    name: String,
    kind: SectionKind,
    sh_flags: u64,
    sh_type: u32,
    align: u64,
    size: u64,
    perm: Perm,
    is_nobits: bool,
    contributions: Vec<SectionContribution>,
}

/// The classic ELF (SysV) hash used for `vna_hash`/`vd_hash` in version records.
fn elf_gnu_hash_ver(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in name {
        h = (h << 4).wrapping_add(b as u32);
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

// ── Layout computation ──────────────────────────────────────────────────────

/// Run the full layout pass. `got_syms` is the ordered list of symbols that need
/// a GOT slot (extracted from the relocation scan by the caller).
pub fn compute_layout(
    objects: &[InputObject],
    symbols: &SymbolTable,
    got_syms: &[SymbolId],
    plt_syms: &[SymbolId],
    live: Option<&FxHashSet<(usize, usize)>>,
    dynamic: Option<&DynamicInfo>,
    config: &LayoutConfig,
) -> Result<Layout> {
    let page = config.page_size.max(1);
    let base = config.base_address;

    // Pre-build the dynamic symbol/string/hash blobs (sizes drive section layout);
    // `.rela.dyn` and `.dynamic` are filled after addresses are known.
    let mut dyn_blobs = DynBlobs::default();
    let mut import_dynsym: FxHashMap<Vec<u8>, u32> = FxHashMap::default();
    let mut needed_off: Vec<u32> = Vec::new();
    if let Some(dynj) = dynamic {
        let mut dynstr = vec![0u8];
        let mut import_name_off = Vec::with_capacity(dynj.imports.len());
        for (i, name) in dynj.imports.iter().enumerate() {
            import_name_off.push(dynstr.len() as u32);
            dynstr.extend_from_slice(name);
            dynstr.push(0);
            import_dynsym.insert(name.clone(), (i + 1) as u32);
        }
        let mut soname_off: Vec<u32> = Vec::with_capacity(dynj.needed.len());
        for so in &dynj.needed {
            soname_off.push(dynstr.len() as u32);
            needed_off.push(dynstr.len() as u32);
            dynstr.extend_from_slice(so.as_bytes());
            dynstr.push(0);
        }
        let mut dynsym = vec![0u8; 24]; // null symbol
        for &off in &import_name_off {
            let mut e = [0u8; 24];
            e[0..4].copy_from_slice(&off.to_le_bytes()); // st_name
            e[4] = elf::st_info(elf::STB_GLOBAL, elf::STT_NOTYPE); // st_info
            // st_other = 0, st_shndx = SHN_UNDEF (0), st_value/st_size = 0
            dynsym.extend_from_slice(&e);
        }

        // ── Symbol versioning: .gnu.version + .gnu.version_r ──────────────────
        //
        // Group each distinct (soname, version) requirement into its providing
        // library's Verneed record, assigning a globally-unique `vna_other`
        // index (≥ 2; 0 = local, 1 = global/base). The versym array maps every
        // dynsym entry to its index. This handles multi-library links (e.g. a
        // C++ program needing libc, libstdc++, and libgcc_s) correctly.
        let soname_index: FxHashMap<&str, usize> = dynj
            .needed
            .iter()
            .enumerate()
            .map(|(i, s)| (s.as_str(), i))
            .collect();
        // Per-library ordered list of (version, vidx, name_off).
        let mut per_lib: Vec<Vec<(Vec<u8>, u16, u32)>> = vec![Vec::new(); dynj.needed.len()];
        let mut version_idx: FxHashMap<(usize, Vec<u8>), u16> = FxHashMap::default();
        let mut next_vidx: u16 = 2;
        for (ver, son) in dynj.import_versions.iter().zip(&dynj.import_sonames) {
            let (Some(v), Some(s)) = (ver, son) else {
                continue;
            };
            let Some(&lib) = soname_index.get(s.as_str()) else {
                continue;
            };
            let key = (lib, v.clone());
            if !version_idx.contains_key(&key) {
                version_idx.insert(key, next_vidx);
                let off = dynstr.len() as u32;
                dynstr.extend_from_slice(v);
                dynstr.push(0);
                per_lib[lib].push((v.clone(), next_vidx, off));
                next_vidx += 1;
            }
        }

        // versym: Elf64_Half per dynsym entry (null sym → 0, then one per import).
        let mut versym: Vec<u8> = Vec::with_capacity((dynj.imports.len() + 1) * 2);
        versym.extend_from_slice(&0u16.to_le_bytes()); // null symbol
        for (ver, son) in dynj.import_versions.iter().zip(&dynj.import_sonames) {
            let idx = match (ver, son) {
                (Some(v), Some(s)) => soname_index
                    .get(s.as_str())
                    .and_then(|&lib| version_idx.get(&(lib, v.clone())).copied())
                    .unwrap_or(1),
                _ => 1, // global/unversioned
            };
            versym.extend_from_slice(&idx.to_le_bytes());
        }

        // verneed: one Verneed per library that has versioned requirements, each
        // with one Vernaux per distinct version.
        let mut verneed: Vec<u8> = Vec::new();
        let mut verneed_num: u64 = 0;
        let active_libs: Vec<usize> = (0..dynj.needed.len())
            .filter(|&l| !per_lib[l].is_empty())
            .collect();
        for (li, &lib) in active_libs.iter().enumerate() {
            let versions = &per_lib[lib];
            let lib_off = soname_off.get(lib).copied().unwrap_or(0);
            let cnt = versions.len() as u16;
            let vn_next = if li + 1 < active_libs.len() {
                // size of this Verneed: 16 header + 16*cnt aux
                16u32 + 16 * cnt as u32
            } else {
                0
            };
            // Elf64_Verneed: vn_version(2) vn_cnt(2) vn_file(4) vn_aux(4) vn_next(4) = 16
            verneed.extend_from_slice(&1u16.to_le_bytes());
            verneed.extend_from_slice(&cnt.to_le_bytes());
            verneed.extend_from_slice(&lib_off.to_le_bytes());
            verneed.extend_from_slice(&16u32.to_le_bytes()); // vn_aux → first aux
            verneed.extend_from_slice(&vn_next.to_le_bytes());
            // Elf64_Vernaux: vna_hash(4) vna_flags(2) vna_other(2) vna_name(4) vna_next(4) = 16
            for (i, (vname, vidx, name_off)) in versions.iter().enumerate() {
                let next = if i + 1 < versions.len() { 16u32 } else { 0u32 };
                verneed.extend_from_slice(&elf_gnu_hash_ver(vname).to_le_bytes());
                verneed.extend_from_slice(&0u16.to_le_bytes()); // vna_flags
                verneed.extend_from_slice(&vidx.to_le_bytes()); // vna_other
                verneed.extend_from_slice(&name_off.to_le_bytes()); // vna_name
                verneed.extend_from_slice(&next.to_le_bytes()); // vna_next
            }
            verneed_num += 1;
        }

        // SysV hash with no exported symbols (we only import).
        let nchain = (dynj.imports.len() + 1) as u32;
        let mut hash = Vec::new();
        hash.extend_from_slice(&1u32.to_le_bytes()); // nbucket
        hash.extend_from_slice(&nchain.to_le_bytes()); // nchain
        hash.extend_from_slice(&0u32.to_le_bytes()); // bucket[0] = STN_UNDEF
        for _ in 0..nchain {
            hash.extend_from_slice(&0u32.to_le_bytes()); // chain[]
        }

        // GNU hash table. The executable's `.dynsym` contains only undefined
        // imports (no exported definitions), so the hashable range is empty:
        //   nbuckets=1, symoffset=nsyms (all symbols are below symoffset and
        //   thus excluded from the hash), bloom_size=1 (one all-zero word),
        //   bloom_shift=0. bucket[0]=0 (no chain). This is the minimal valid
        //   GNU hash glibc accepts; ld.so prefers DT_GNU_HASH when present.
        let nsyms = (dynj.imports.len() + 1) as u32; // incl. null symbol
        let mut gnu_hash = Vec::new();
        gnu_hash.extend_from_slice(&1u32.to_le_bytes()); // nbuckets
        gnu_hash.extend_from_slice(&nsyms.to_le_bytes()); // symoffset (all excluded)
        gnu_hash.extend_from_slice(&1u32.to_le_bytes()); // bloom_size (words)
        gnu_hash.extend_from_slice(&0u32.to_le_bytes()); // bloom_shift
        gnu_hash.extend_from_slice(&0u64.to_le_bytes()); // bloom[0] = 0 (matches nothing)
        gnu_hash.extend_from_slice(&0u32.to_le_bytes()); // bucket[0] = 0 (empty)

        dyn_blobs.dynstr = dynstr;
        dyn_blobs.dynsym = dynsym;
        dyn_blobs.hash = hash;
        dyn_blobs.gnu_hash = gnu_hash;
        dyn_blobs.interp = elf::DEFAULT_INTERP.to_vec();
        // Only attach versym/verneed when there is something to version.
        if verneed_num > 0 {
            dyn_blobs.gnu_version = versym;
            dyn_blobs.gnu_version_r = verneed;
            dyn_blobs.verneed_num = verneed_num;
        }
    }
    // Number of GOT slots that resolve to dynamic imports (→ GLOB_DAT relocs).
    let n_glob_dat = got_syms
        .iter()
        .filter(|id| {
            symbols
                .name_by_id(**id)
                .and_then(|n| symbols.lookup(n))
                .is_some_and(|r| r.import)
        })
        .count();

    // ── Pass 5: group allocatable input sections into output sections ────────
    let mut builders: FxHashMap<String, Builder> = FxHashMap::default();
    // Preserve first-seen order for determinism.
    let mut order: Vec<String> = Vec::new();

    for (obj_idx, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            if !is_allocatable(&sec.name, sec.kind, sec.flags) {
                continue;
            }
            if sec.size == 0 {
                continue;
            }
            // --gc-sections: skip sections not reachable from the roots.
            if let Some(live) = live {
                if !live.contains(&(obj_idx, sec.index.0)) {
                    continue;
                }
            }
            let align = if sec.align == 0 { 1 } else { sec.align };
            if !align.is_power_of_two() {
                return Err(LayoutError::BadAlignment {
                    align,
                    object: obj.path.clone(),
                });
            }

            let out_name = output_section_name(&sec.name);
            let is_nobits = matches!(sec.kind, SectionKind::Bss | SectionKind::Tbss);
            let b = builders.entry(out_name.clone()).or_insert_with(|| {
                // Preserve the meaningful section flags, including SHF_TLS so the
                // loader maps `.tdata`/`.tbss` as the static TLS template.
                let flag_mask = elf::SHF_ALLOC | elf::SHF_WRITE | elf::SHF_EXECINSTR | elf::SHF_TLS;
                // Init/fini arrays carry function pointers and have dedicated
                // section types the loader/linters expect.
                let sh_type = if is_nobits {
                    elf::SHT_NOBITS
                } else {
                    match out_name.as_str() {
                        ".init_array" => elf::SHT_INIT_ARRAY,
                        ".fini_array" => elf::SHT_FINI_ARRAY,
                        ".preinit_array" => elf::SHT_PREINIT_ARRAY,
                        _ => elf::SHT_PROGBITS,
                    }
                };
                order.push(out_name.clone());
                Builder {
                    name: out_name.clone(),
                    kind: sec.kind,
                    sh_flags: sec.flags & flag_mask,
                    sh_type,
                    align,
                    size: 0,
                    perm: perm_of(sec.flags),
                    is_nobits,
                    contributions: Vec::new(),
                }
            });

            let off = align_up(b.size, align);
            b.contributions.push(SectionContribution {
                object_id: obj_idx,
                section_index: sec.index.0,
                offset: off,
                size: sec.size,
            });
            b.size = off + sec.size;
            b.align = b.align.max(align);
        }
    }

    // Bucket builders by permission class (deterministic name order within each).
    let mut ro: Vec<Builder> = Vec::new();
    let mut rx: Vec<Builder> = Vec::new();
    let mut rw_pb: Vec<Builder> = Vec::new();
    let mut rw_bss: Vec<Builder> = Vec::new();
    for name in &order {
        let b = builders.remove(name).unwrap();
        match (b.perm, b.is_nobits) {
            (Perm::Ro, _) => ro.push(b),
            (Perm::Rx, _) => rx.push(b),
            (Perm::Rw, false) => rw_pb.push(b),
            (Perm::Rw, true) => rw_bss.push(b),
        }
    }
    ro.sort_by(|a, b| a.name.cmp(&b.name));
    rx.sort_by(|a, b| a.name.cmp(&b.name));
    rw_pb.sort_by(|a, b| a.name.cmp(&b.name));
    rw_bss.sort_by(|a, b| a.name.cmp(&b.name));

    // Synthesise .got (RW progbits) from the scan.
    if !got_syms.is_empty() {
        rw_pb.push(Builder {
            name: ".got".to_string(),
            kind: SectionKind::Data,
            sh_flags: elf::SHF_ALLOC | elf::SHF_WRITE,
            sh_type: elf::SHT_PROGBITS,
            align: 8,
            size: (got_syms.len() as u64) * 8,
            perm: Perm::Rw,
            is_nobits: false,
            contributions: Vec::new(),
        });
    }

    // Allocate tentative (common) symbols into a synthetic `.bss` (no input
    // contributions — that lets us find it again after placement).
    let mut common_syms: Vec<(SymbolId, u64, u64)> = symbols
        .iter()
        .filter_map(|(_, r)| r.common.map(|(s, a)| (r.id, s, a)))
        .collect();
    common_syms.sort_by_key(|c| c.0.0);
    let mut common_offsets: Vec<(SymbolId, u64)> = Vec::new();
    let mut common_size = 0u64;
    let mut common_align = 1u64;
    for (id, sz, al) in &common_syms {
        let al = (*al).max(1);
        common_align = common_align.max(al);
        let off = align_up(common_size, al);
        common_offsets.push((*id, off));
        common_size = off + *sz;
    }
    if common_size > 0 {
        rw_bss.push(Builder {
            name: ".bss".to_string(),
            kind: SectionKind::Bss,
            sh_flags: elf::SHF_ALLOC | elf::SHF_WRITE,
            sh_type: elf::SHT_NOBITS,
            align: common_align,
            size: common_size,
            perm: Perm::Rw,
            is_nobits: true,
            contributions: Vec::new(),
        });
    }

    // Synthesise .note.gnu.build-id (RO) if requested.
    if config.build_id {
        ro.push(Builder {
            name: ".note.gnu.build-id".to_string(),
            kind: SectionKind::Other,
            sh_flags: elf::SHF_ALLOC,
            sh_type: elf::SHT_NOTE,
            align: 4,
            size: BUILD_ID_NOTE_SIZE,
            perm: Perm::Ro,
            is_nobits: false,
            contributions: Vec::new(),
        });
    }

    // Synthesise `.eh_frame_hdr` when `.eh_frame` is present: a sorted FDE
    // binary-search table the unwinder finds via PT_GNU_EH_FRAME. Size is
    // 12-byte header + 8 bytes per FDE; the contents are filled post-layout.
    let n_fdes: usize = objects
        .iter()
        .flat_map(|o| o.sections.iter())
        .filter(|s| s.kind == SectionKind::EhFrame)
        .map(|s| peony_object::count_fdes(&s.data))
        .sum();
    tracing::debug!(
        n_fdes,
        "eh_frame_hdr: counted FDEs across input .eh_frame sections"
    );
    if n_fdes > 0 {
        ro.push(Builder {
            name: ".eh_frame_hdr".to_string(),
            kind: SectionKind::Other,
            sh_flags: elf::SHF_ALLOC,
            sh_type: elf::SHT_PROGBITS,
            align: 4,
            size: 12 + 8 * n_fdes as u64,
            perm: Perm::Ro,
            is_nobits: false,
            contributions: Vec::new(),
        });
    }

    // Synthesise dynamic-linking sections (RO except .dynamic which is RW).
    let mut dyn_entries = 0usize;
    if let Some(dynj) = dynamic {
        let ro_dyn = [
            (
                ".interp",
                elf::SHT_PROGBITS,
                1u64,
                dyn_blobs.interp.len() as u64,
            ),
            (".hash", elf::SHT_HASH, 8, dyn_blobs.hash.len() as u64),
            (
                ".gnu.hash",
                elf::SHT_GNU_HASH,
                8,
                dyn_blobs.gnu_hash.len() as u64,
            ),
            (".dynsym", elf::SHT_DYNSYM, 8, dyn_blobs.dynsym.len() as u64),
            (".dynstr", elf::SHT_STRTAB, 1, dyn_blobs.dynstr.len() as u64),
            (
                ".gnu.version",
                elf::SHT_GNU_VERSYM,
                2,
                dyn_blobs.gnu_version.len() as u64,
            ),
            (
                ".gnu.version_r",
                elf::SHT_GNU_VERNEED,
                8,
                dyn_blobs.gnu_version_r.len() as u64,
            ),
            (
                ".rela.dyn",
                elf::SHT_RELA,
                8,
                ((n_glob_dat + dynj.n_relative) as u64) * elf::RELA_SIZE,
            ),
        ];
        for (name, sh_type, align, size) in ro_dyn {
            if size == 0 {
                continue;
            }
            ro.push(Builder {
                name: name.to_string(),
                kind: SectionKind::Other,
                sh_flags: elf::SHF_ALLOC,
                sh_type,
                align,
                size,
                perm: Perm::Ro,
                is_nobits: false,
                contributions: Vec::new(),
            });
        }
        // PLT machinery for direct `call foo@PLT` to imports (eager binding).
        let n_plt = plt_syms.len();
        if n_plt > 0 {
            rx.push(Builder {
                name: ".plt".to_string(),
                kind: SectionKind::Text,
                sh_flags: elf::SHF_ALLOC | elf::SHF_EXECINSTR,
                sh_type: elf::SHT_PROGBITS,
                align: 16,
                size: (n_plt as u64) * 16,
                perm: Perm::Rx,
                is_nobits: false,
                contributions: Vec::new(),
            });
            rw_pb.push(Builder {
                name: ".got.plt".to_string(),
                kind: SectionKind::Data,
                sh_flags: elf::SHF_ALLOC | elf::SHF_WRITE,
                sh_type: elf::SHT_PROGBITS,
                align: 8,
                size: ((n_plt + 3) as u64) * 8, // 3 reserved + 1 per stub
                perm: Perm::Rw,
                is_nobits: false,
                contributions: Vec::new(),
            });
            ro.push(Builder {
                name: ".rela.plt".to_string(),
                kind: SectionKind::Other,
                sh_flags: elf::SHF_ALLOC,
                sh_type: elf::SHT_RELA,
                align: 8,
                size: (n_plt as u64) * elf::RELA_SIZE,
                perm: Perm::Ro,
                is_nobits: false,
                contributions: Vec::new(),
            });
        }

        // .dynamic entries: NEEDED* + HASH,STRTAB,SYMTAB,STRSZ,SYMENT + RELA group?
        //                   + PLT group? + INIT/FINI_ARRAY? + NULL
        let n_initfini = rw_pb
            .iter()
            .filter(|b| b.name == ".init_array" || b.name == ".fini_array")
            .count();
        let has_rela = n_glob_dat + dynj.n_relative > 0;
        // FLAGS entries: PIE → DT_FLAGS + DT_FLAGS_1 (2); else PLT-only → DT_FLAGS (1).
        let n_flags = if dynj.pie {
            2
        } else if n_plt > 0 {
            1
        } else {
            0
        };
        let has_ver = dyn_blobs.verneed_num > 0;
        // DT_INIT / DT_FINI when the corresponding (executable) sections exist.
        let n_init = rx.iter().filter(|b| b.name == ".init").count();
        let n_fini = rx.iter().filter(|b| b.name == ".fini").count();
        dyn_entries = dynj.needed.len()
            + 5                                  // HASH,STRTAB,SYMTAB,STRSZ,SYMENT
            + 1                                  // DT_GNU_HASH
            + if has_rela { 3 } else { 0 }       // DT_RELA, DT_RELASZ, DT_RELAENT
            + if dynj.n_relative > 0 { 1 } else { 0 } // DT_RELACOUNT
            + if n_plt > 0 { 4 } else { 0 }      // PLTGOT,PLTRELSZ,PLTREL,JMPREL
            + if has_ver { 3 } else { 0 }        // VERSYM,VERNEED,VERNEEDNUM
            + n_init + n_fini                    // DT_INIT, DT_FINI
            + n_flags
            + n_initfini * 2
            + 1; // DT_NULL
        rw_pb.push(Builder {
            name: ".dynamic".to_string(),
            kind: SectionKind::Data,
            sh_flags: elf::SHF_ALLOC | elf::SHF_WRITE,
            sh_type: elf::SHT_DYNAMIC,
            align: 8,
            size: (dyn_entries as u64) * 16,
            perm: Perm::Rw,
            is_nobits: false,
            contributions: Vec::new(),
        });
    }

    // ── Determine segment count (needed before we reserve header space) ──────
    let has_rx = !rx.is_empty();
    let has_rw = !rw_pb.is_empty() || !rw_bss.is_empty();
    let has_tls = rw_pb
        .iter()
        .chain(rw_bss.iter())
        .any(|b| matches!(b.kind, SectionKind::Tdata | SectionKind::Tbss));
    let num_load = 1 + usize::from(has_rx) + usize::from(has_rw); // RO(headers) + RX? + RW?
    // PT_PHDR + PT_NOTE? + (PT_INTERP + PT_DYNAMIC)? + PT_TLS? + PT_GNU_EH_FRAME?
    //   + loads + PT_GNU_RELRO? + PT_GNU_STACK
    let has_eh_frame_hdr = n_fdes > 0;
    // RELRO is emitted for exactly the dynamic links. This single boolean is the
    // ONLY gate used both here (header count) and at the push site below, so the
    // count can never disagree with what is emitted. A dynamic link always has a
    // `.dynamic` section, which is itself a RELRO member, so the span is always
    // non-empty — but the push falls back to `.dynamic` defensively regardless.
    let has_relro = dynamic.is_some();
    let phnum = (1
        + usize::from(config.build_id)
        + if dynamic.is_some() { 2 } else { 0 }
        + usize::from(has_tls)
        + usize::from(has_eh_frame_hdr)
        + usize::from(has_relro)
        + num_load
        + 1) as u64;
    let header_size = elf::EHDR_SIZE + phnum * elf::PHDR_SIZE;

    // ── Pass 8: assign virtual addresses + file offsets ──────────────────────
    // Invariant for all loadable content: file_offset == vaddr - base.
    let mut sections: Vec<OutputSection> = Vec::new();
    let mut addresses: FxHashMap<(usize, usize), u64> = FxHashMap::default();
    let mut sec_to_out: FxHashMap<(usize, usize), usize> = FxHashMap::default();

    // RO segment: headers, then read-only sections.
    let ro_seg_start = base;
    let mut va = base + header_size; // reserve ehdr + phdrs in the RO segment
    place(
        &ro,
        base,
        &mut va,
        &mut sections,
        &mut addresses,
        &mut sec_to_out,
    );
    let ro_seg_end = va;

    // RX segment.
    let mut rx_seg: Option<(u64, u64)> = None;
    if has_rx {
        va = align_up(va, page);
        let start = va;
        place(
            &rx,
            base,
            &mut va,
            &mut sections,
            &mut addresses,
            &mut sec_to_out,
        );
        rx_seg = Some((start, va));
    }

    // RW segment: progbits (file-backed) then nobits (.bss).
    // TLS contiguity: the static TLS template requires `.tdata` immediately
    // followed by `.tbss`. Make `.tdata` the LAST file-backed RW section and
    // `.tbss` the FIRST nobits section so they abut across the progbits→nobits
    // boundary, placing `.tbss`'s address inside the PT_TLS range. This runs
    // after all synthetic RW sections (`.got`, `.dynamic`, …) have been added.
    rw_pb.sort_by_key(|b| b.kind == SectionKind::Tdata); // false<true ⇒ .tdata last
    rw_bss.sort_by_key(|b| b.kind != SectionKind::Tbss); // false<true ⇒ .tbss first

    let mut rw_seg: Option<(u64, u64, u64)> = None; // (start, file_end, mem_end)
    if has_rw {
        va = align_up(va, page);
        let start = va;
        place(
            &rw_pb,
            base,
            &mut va,
            &mut sections,
            &mut addresses,
            &mut sec_to_out,
        );
        let file_end = va;
        place(
            &rw_bss,
            base,
            &mut va,
            &mut sections,
            &mut addresses,
            &mut sec_to_out,
        );
        rw_seg = Some((start, file_end, va));
    }

    // End of file-backed allocatable content (excludes .bss).
    let file_content_end = match (rw_seg, rx_seg) {
        (Some((_, file_end, _)), _) => file_end - base,
        (None, Some((_, end))) => end - base,
        (None, None) => ro_seg_end - base,
    };

    // ── Symbol table plan (.symtab / .strtab) ────────────────────────────────
    // `shndx == position + 1` for every alloc section (the null header is 0 and
    // meta sections are appended later), so it is final even before assignment.
    let (symtab, strtab, num_locals) = build_symtab_plan(symbols, objects, &addresses, &sec_to_out);

    // ── Place non-allocatable meta sections ──────────────────────────────────
    // `.symtab`/`.strtab` are omitted under `--strip-all`; `.shstrtab` is always
    // emitted (the section headers reference it).
    let mut foff = align_up(file_content_end, 8);
    let (symtab_emitted, symtab) = if config.strip {
        (false, Vec::new())
    } else {
        (true, symtab)
    };

    let mut symtab_pos = None;
    let mut strtab_pos = None;
    if symtab_emitted {
        let symtab_off = foff;
        let symtab_size = ((symtab.len() + 1) as u64) * elf::SYM_SIZE;
        foff += symtab_size;
        let strtab_off = foff;
        let strtab_size = strtab.len() as u64;
        foff += strtab_size;

        symtab_pos = Some(sections.len());
        sections.push(OutputSection {
            name: ".symtab".to_string(),
            kind: SectionKind::Other,
            sh_type: elf::SHT_SYMTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: symtab_off,
            sh_size: symtab_size,
            sh_link: 0,                       // → strtab shndx, set below
            sh_info: (num_locals + 1) as u32, // index of the first global symbol
            sh_addralign: 8,
            sh_entsize: elf::SYM_SIZE,
            sh_name: 0,
            shndx: 0,
            contributions: Vec::new(),
            source: SecSource::SymTab,
        });
        strtab_pos = Some(sections.len());
        sections.push(OutputSection {
            name: ".strtab".to_string(),
            kind: SectionKind::Other,
            sh_type: elf::SHT_STRTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: strtab_off,
            sh_size: strtab_size,
            sh_link: 0,
            sh_info: 0,
            sh_addralign: 1,
            sh_entsize: 0,
            sh_name: 0,
            shndx: 0,
            contributions: Vec::new(),
            source: SecSource::StrTab,
        });
    }

    let shstrtab_off = foff;
    let shstrtab_pos = sections.len();
    sections.push(OutputSection {
        name: ".shstrtab".to_string(),
        kind: SectionKind::Other,
        sh_type: elf::SHT_STRTAB,
        sh_flags: 0,
        sh_addr: 0,
        sh_offset: shstrtab_off,
        sh_size: 0, // set after building shstrtab bytes
        sh_link: 0,
        sh_info: 0,
        sh_addralign: 1,
        sh_entsize: 0,
        sh_name: 0,
        shndx: 0,
        contributions: Vec::new(),
        source: SecSource::ShStrTab,
    });

    // Assign shndx to all sections (= position + 1; index 0 is the null entry).
    for (i, s) in sections.iter_mut().enumerate() {
        s.shndx = (i + 1) as u32;
    }
    if let (Some(sp), Some(tp)) = (symtab_pos, strtab_pos) {
        sections[sp].sh_link = sections[tp].shndx;
    }
    // Dynamic-section header links (so readelf/tools parse them; `ld.so` uses the
    // DT_* tags at runtime).
    if dynamic.is_some() {
        let shndx_of = |src: SecSource| {
            sections
                .iter()
                .find(|s| s.source == src)
                .map_or(0, |s| s.shndx)
        };
        let dynstr_shndx = shndx_of(SecSource::DynStr);
        let dynsym_shndx = shndx_of(SecSource::DynSym);
        for s in sections.iter_mut() {
            match s.source {
                SecSource::DynSym => {
                    s.sh_link = dynstr_shndx;
                    s.sh_info = 1; // first global dynsym index
                }
                SecSource::Hash | SecSource::GnuHash | SecSource::RelaDyn | SecSource::RelaPlt => {
                    s.sh_link = dynsym_shndx
                }
                SecSource::Dynamic => s.sh_link = dynstr_shndx,
                // .gnu.version → links to .dynsym (one versym per dynsym entry).
                SecSource::GnuVersion => s.sh_link = dynsym_shndx,
                // .gnu.version_r → links to .dynstr; sh_info = number of verneed.
                SecSource::GnuVersionR => {
                    s.sh_link = dynstr_shndx;
                    s.sh_info = dyn_blobs.verneed_num as u32;
                }
                _ => {}
            }
        }
    }

    // Build .shstrtab from every section name and fill sh_name.
    let mut shstrtab: Vec<u8> = vec![0];
    for s in sections.iter_mut() {
        s.sh_name = shstrtab.len() as u32;
        shstrtab.extend_from_slice(s.name.as_bytes());
        shstrtab.push(0);
    }
    let shstrtab_size = shstrtab.len() as u64;
    sections[shstrtab_pos].sh_size = shstrtab_size;

    // ── Section header table + file size ─────────────────────────────────────
    let shoff = align_up(shstrtab_off + shstrtab_size, 8);
    let shnum = (sections.len() + 1) as u64; // + null section
    let shstrndx = sections[shstrtab_pos].shndx as u64;
    let file_size = shoff + shnum * elf::SHDR_SIZE;

    // ── GOT base ─────────────────────────────────────────────────────────────
    let got_base = sections
        .iter()
        .find(|s| s.source == SecSource::Got)
        .map(|s| s.sh_addr)
        .unwrap_or(0);

    // ── TLS layout: assign offsets within the static TLS block (.tdata, .tbss) ─
    let mut tls_offsets: FxHashMap<(usize, usize), u64> = FxHashMap::default();
    let mut tls_off = 0u64;
    let mut tls_align = 1u64;
    let mut tls_vaddr = 0u64;
    let mut tls_file_off = 0u64;
    let mut tls_filesz = 0u64;
    let mut tls_first = true;
    for want_tdata in [true, false] {
        for s in &sections {
            let is_tls = match s.kind {
                SectionKind::Tdata => want_tdata,
                SectionKind::Tbss => !want_tdata,
                _ => false,
            };
            if !is_tls {
                continue;
            }
            let align = s.sh_addralign.max(1);
            tls_off = align_up(tls_off, align);
            tls_align = tls_align.max(align);
            if tls_first {
                tls_vaddr = s.sh_addr;
                tls_file_off = s.sh_offset;
                tls_first = false;
            }
            for c in &s.contributions {
                tls_offsets.insert((c.object_id, c.section_index), tls_off + c.offset);
            }
            if s.kind == SectionKind::Tdata {
                tls_filesz += s.sh_size;
            }
            tls_off += s.sh_size;
        }
    }
    let tls_size = align_up(tls_off, tls_align);
    tracing::debug!(
        tls_size,
        tls_filesz,
        tls_align,
        tls_vaddr = format_args!("{tls_vaddr:#x}"),
        tls_blocks = tls_offsets.len(),
        "TLS layout: static block sized"
    );

    // ── Entry point ──────────────────────────────────────────────────────────
    let entry = resolve_entry(symbols, &addresses, &config.entry_symbol)?;

    // ── Dynamic blobs (.rela.dyn, .dynamic) — now that section VAs are known ──
    let mut plt_base = 0u64;
    if let Some(dynj) = dynamic {
        let va_of = |src: SecSource| {
            sections
                .iter()
                .find(|s| s.source == src)
                .map(|s| s.sh_addr)
                .unwrap_or(0)
        };

        // `.rela.dyn` layout: R_X86_64_RELATIVE entries MUST come first, because
        // glibc's `elf_machine_rela_relative` fast path processes the leading
        // DT_RELACOUNT entries assuming they are all RELATIVE (and asserts it).
        // The GLOB_DATs are built here and stashed; `append_relative_relocs`
        // (post-layout) prepends the actual relatives and appends these, with no
        // type-0 gap in between (which `eu-elflint`/BFD reject).
        let mut glob = Vec::with_capacity(n_glob_dat * elf::RELA_SIZE as usize);
        for (i, id) in got_syms.iter().enumerate() {
            let Some(name) = symbols.name_by_id(*id) else {
                continue;
            };
            let is_import = symbols.lookup(name).is_some_and(|r| r.import);
            if !is_import {
                continue;
            }
            let dynidx = *import_dynsym.get(name).unwrap_or(&0);
            let r_offset = got_base + (i as u64) * 8;
            let r_info = ((dynidx as u64) << 32) | (elf::R_X86_64_GLOB_DAT as u64);
            glob.extend_from_slice(&r_offset.to_le_bytes());
            glob.extend_from_slice(&r_info.to_le_bytes());
            glob.extend_from_slice(&0i64.to_le_bytes()); // r_addend
        }
        // Reserve the full section (relatives + glob_dats); the actual content is
        // assembled post-layout. Section size stays `(n_relative+n_glob_dat)*ent`.
        let rela_sz = ((dynj.n_relative + n_glob_dat) as u64) * elf::RELA_SIZE;
        dyn_blobs.rela_glob_dat = glob;
        dyn_blobs.rela_dyn = Vec::new();

        // PLT: `jmp *slot(%rip)` stubs, `.got.plt` slots, JUMP_SLOT relocs.
        let gotplt_base = va_of(SecSource::GotPlt);
        plt_base = va_of(SecSource::Plt);
        let dynamic_va = va_of(SecSource::Dynamic);
        if !plt_syms.is_empty() {
            let mut plt = Vec::with_capacity(plt_syms.len() * 16);
            let mut gotplt = Vec::with_capacity((plt_syms.len() + 3) * 8);
            let mut relaplt = Vec::with_capacity(plt_syms.len() * elf::RELA_SIZE as usize);
            // .got.plt[0] = &.dynamic; [1],[2] reserved for the loader.
            gotplt.extend_from_slice(&dynamic_va.to_le_bytes());
            gotplt.extend_from_slice(&0u64.to_le_bytes());
            gotplt.extend_from_slice(&0u64.to_le_bytes());
            for (i, id) in plt_syms.iter().enumerate() {
                let slot_va = gotplt_base + ((i + 3) as u64) * 8;
                let stub_va = plt_base + (i as u64) * 16;
                // jmp *slot(%rip): FF 25 <disp32>, disp relative to end of the insn.
                let disp = (slot_va as i64 - (stub_va as i64 + 6)) as i32;
                let mut stub = [0x90u8; 16]; // nop padding
                stub[0] = 0xff;
                stub[1] = 0x25;
                stub[2..6].copy_from_slice(&disp.to_le_bytes());
                plt.extend_from_slice(&stub);
                gotplt.extend_from_slice(&0u64.to_le_bytes()); // filled eagerly by ld.so
                let dynidx = symbols
                    .name_by_id(*id)
                    .and_then(|n| import_dynsym.get(n))
                    .copied()
                    .unwrap_or(0);
                let r_info = ((dynidx as u64) << 32) | (elf::R_X86_64_JUMP_SLOT as u64);
                relaplt.extend_from_slice(&slot_va.to_le_bytes());
                relaplt.extend_from_slice(&r_info.to_le_bytes());
                relaplt.extend_from_slice(&0i64.to_le_bytes());
            }
            dyn_blobs.plt = plt;
            dyn_blobs.got_plt = gotplt;
            dyn_blobs.rela_plt = relaplt;
        }

        let mut dyn_bytes = Vec::with_capacity(dyn_entries * 16);
        let mut push_dyn = |tag: i64, val: u64| {
            dyn_bytes.extend_from_slice(&tag.to_le_bytes());
            dyn_bytes.extend_from_slice(&val.to_le_bytes());
        };
        for &off in &needed_off {
            push_dyn(elf::DT_NEEDED, off as u64);
        }
        push_dyn(elf::DT_HASH, va_of(SecSource::Hash));
        push_dyn(elf::DT_GNU_HASH, va_of(SecSource::GnuHash));
        push_dyn(elf::DT_STRTAB, va_of(SecSource::DynStr));
        push_dyn(elf::DT_SYMTAB, va_of(SecSource::DynSym));
        push_dyn(elf::DT_STRSZ, dyn_blobs.dynstr.len() as u64);
        push_dyn(elf::DT_SYMENT, 24);
        // RELA group covers GLOB_DAT (imports) + RELATIVE (PIE base) entries.
        let total_rela = (n_glob_dat + dynj.n_relative) as u64 * elf::RELA_SIZE;
        if total_rela > 0 {
            push_dyn(elf::DT_RELA, va_of(SecSource::RelaDyn));
            push_dyn(elf::DT_RELASZ, total_rela);
            push_dyn(elf::DT_RELAENT, elf::RELA_SIZE);
        }
        // DT_RELACOUNT: number of leading R_X86_64_RELATIVE entries. peony emits
        // GLOB_DAT first then RELATIVE, but glibc only uses RELACOUNT as a fast
        // path hint; the relatives are still applied in full, so reporting the
        // count is safe and standard.
        if dynj.n_relative > 0 {
            push_dyn(elf::DT_RELACOUNT, dynj.n_relative as u64);
        }
        if !plt_syms.is_empty() {
            push_dyn(elf::DT_PLTGOT, va_of(SecSource::GotPlt));
            push_dyn(elf::DT_PLTRELSZ, (plt_syms.len() as u64) * elf::RELA_SIZE);
            push_dyn(elf::DT_PLTREL, elf::DT_RELA as u64);
            push_dyn(elf::DT_JMPREL, va_of(SecSource::RelaPlt));
        }
        // Symbol versioning tables.
        if dyn_blobs.verneed_num > 0 {
            push_dyn(elf::DT_VERSYM, va_of(SecSource::GnuVersion));
            push_dyn(elf::DT_VERNEED, va_of(SecSource::GnuVersionR));
            push_dyn(elf::DT_VERNEEDNUM, dyn_blobs.verneed_num);
        }
        // PIE / eager-binding flags.
        if dynj.pie {
            push_dyn(elf::DT_FLAGS, elf::DF_BIND_NOW);
            push_dyn(elf::DT_FLAGS_1, elf::DF_1_NOW | elf::DF_1_PIE);
        } else if !plt_syms.is_empty() {
            push_dyn(elf::DT_FLAGS, elf::DF_BIND_NOW); // eager binding (no lazy resolver)
        }
        // DT_INIT / DT_FINI point at the legacy `_init`/`_fini` routines (from
        // crti.o/crtn.o); glibc's __libc_start_main calls DT_INIT before main.
        if let Some(s) = sections.iter().find(|s| s.name == ".init") {
            push_dyn(elf::DT_INIT, s.sh_addr);
        }
        if let Some(s) = sections.iter().find(|s| s.name == ".fini") {
            push_dyn(elf::DT_FINI, s.sh_addr);
        }
        // Init/fini arrays so the C runtime / ld.so runs global constructors.
        for (name, tag_arr, tag_sz) in [
            (".init_array", elf::DT_INIT_ARRAY, elf::DT_INIT_ARRAYSZ),
            (".fini_array", elf::DT_FINI_ARRAY, elf::DT_FINI_ARRAYSZ),
        ] {
            if let Some(s) = sections.iter().find(|s| s.name == name) {
                push_dyn(tag_arr, s.sh_addr);
                push_dyn(tag_sz, s.sh_size);
            }
        }
        push_dyn(elf::DT_NULL, 0);
        dyn_blobs.dynamic = dyn_bytes;
    }

    // ── Program headers ──────────────────────────────────────────────────────
    let mut segments = Vec::with_capacity(phnum as usize);
    segments.push(ProgramHeader {
        p_type: elf::PT_PHDR,
        p_flags: elf::PF_R,
        p_offset: elf::EHDR_SIZE,
        p_vaddr: base + elf::EHDR_SIZE,
        p_paddr: base + elf::EHDR_SIZE,
        p_filesz: phnum * elf::PHDR_SIZE,
        p_memsz: phnum * elf::PHDR_SIZE,
        p_align: 8,
    });
    // RO load (always present; covers ehdr + phdrs + read-only sections).
    segments.push(ProgramHeader {
        p_type: elf::PT_LOAD,
        p_flags: elf::PF_R,
        p_offset: 0,
        p_vaddr: ro_seg_start,
        p_paddr: ro_seg_start,
        p_filesz: ro_seg_end - ro_seg_start,
        p_memsz: ro_seg_end - ro_seg_start,
        p_align: page,
    });
    if let Some((start, end)) = rx_seg {
        segments.push(ProgramHeader {
            p_type: elf::PT_LOAD,
            p_flags: elf::PF_R | elf::PF_X,
            p_offset: start - base,
            p_vaddr: start,
            p_paddr: start,
            p_filesz: end - start,
            p_memsz: end - start,
            p_align: page,
        });
    }
    if let Some((start, file_end, mem_end)) = rw_seg {
        segments.push(ProgramHeader {
            p_type: elf::PT_LOAD,
            p_flags: elf::PF_R | elf::PF_W,
            p_offset: start - base,
            p_vaddr: start,
            p_paddr: start,
            p_filesz: file_end - start,
            p_memsz: mem_end - start,
            p_align: page,
        });
    }
    // PT_NOTE for the build-id (after PT_PHDR, before the loads).
    if config.build_id {
        if let Some(s) = sections.iter().find(|s| s.source == SecSource::NoteBuildId) {
            segments.insert(
                1,
                ProgramHeader {
                    p_type: elf::PT_NOTE,
                    p_flags: elf::PF_R,
                    p_offset: s.sh_offset,
                    p_vaddr: s.sh_addr,
                    p_paddr: s.sh_addr,
                    p_filesz: s.sh_size,
                    p_memsz: s.sh_size,
                    p_align: 4,
                },
            );
        }
    }
    // PT_INTERP (early) and PT_DYNAMIC for a dynamic executable.
    if dynamic.is_some() {
        if let Some(s) = sections.iter().find(|s| s.source == SecSource::Interp) {
            segments.insert(
                1,
                ProgramHeader {
                    p_type: elf::PT_INTERP,
                    p_flags: elf::PF_R,
                    p_offset: s.sh_offset,
                    p_vaddr: s.sh_addr,
                    p_paddr: s.sh_addr,
                    p_filesz: s.sh_size,
                    p_memsz: s.sh_size,
                    p_align: 1,
                },
            );
        }
        if let Some(s) = sections.iter().find(|s| s.source == SecSource::Dynamic) {
            segments.push(ProgramHeader {
                p_type: elf::PT_DYNAMIC,
                p_flags: elf::PF_R | elf::PF_W,
                p_offset: s.sh_offset,
                p_vaddr: s.sh_addr,
                p_paddr: s.sh_addr,
                p_filesz: s.sh_size,
                p_memsz: s.sh_size,
                p_align: 8,
            });
        }
    }
    // PT_TLS describes the static TLS template for the loader.
    if tls_size > 0 {
        segments.push(ProgramHeader {
            p_type: elf::PT_TLS,
            p_flags: elf::PF_R,
            p_offset: tls_file_off,
            p_vaddr: tls_vaddr,
            p_paddr: tls_vaddr,
            p_filesz: tls_filesz,
            p_memsz: tls_size,
            p_align: tls_align,
        });
    }
    // PT_GNU_EH_FRAME points at `.eh_frame_hdr` so the unwinder can binary-search
    // FDEs via `dl_iterate_phdr`.
    if let Some(eh) = sections.iter().find(|s| s.source == SecSource::EhFrameHdr) {
        segments.push(ProgramHeader {
            p_type: elf::PT_GNU_EH_FRAME,
            p_flags: elf::PF_R,
            p_offset: eh.sh_offset,
            p_vaddr: eh.sh_addr,
            p_paddr: eh.sh_addr,
            p_filesz: eh.sh_size,
            p_memsz: eh.sh_size,
            p_align: 4,
        });
    }
    // PT_GNU_RELRO: the sub-region of the RW segment that holds relocation
    // targets resolved at load time (`.init_array`, `.fini_array`,
    // `.data.rel.ro`, `.dynamic`, `.got`) and can be made read-only afterwards.
    // With BIND_NOW (which peony sets) the whole region is eagerly resolved, so
    // the loader can mprotect it read-only — a standard hardening.
    //
    // CRITICAL INVARIANT: this push fires iff `has_relro` is true, the same gate
    // that reserved a program-header slot above. We therefore push exactly one
    // RELRO header whenever `has_relro`, computing the span from the relro member
    // sections and falling back to `.dynamic` (always present in a dynamic link)
    // so the segment is never silently dropped.
    if has_relro {
        const RELRO_NAMES: [&str; 5] = [
            ".init_array",
            ".fini_array",
            ".data.rel.ro",
            ".dynamic",
            ".got",
        ];
        let relro_secs: Vec<&OutputSection> = sections
            .iter()
            .filter(|s| RELRO_NAMES.contains(&s.name.as_str()))
            .collect();
        let span = relro_secs
            .iter()
            .map(|s| (s.sh_addr, s.sh_addr + s.sh_size, s.sh_offset))
            .fold(
                None,
                |acc: Option<(u64, u64, u64)>, (lo, hi, off)| match acc {
                    None => Some((lo, hi, off)),
                    Some((alo, ahi, aoff)) => {
                        Some((alo.min(lo), ahi.max(hi), if lo < alo { off } else { aoff }))
                    }
                },
            )
            .or_else(|| {
                // Defensive fallback: anchor on `.dynamic`, which a dynamic link
                // always has. If even that is missing, anchor on the RW segment.
                sections
                    .iter()
                    .find(|s| s.source == SecSource::Dynamic)
                    .map(|s| (s.sh_addr, s.sh_addr + s.sh_size, s.sh_offset))
            });
        let (lo, hi, lo_off) = span.unwrap_or((base, base, 0));
        let size = hi.saturating_sub(lo);
        tracing::debug!(
            relro_lo = format_args!("{lo:#x}"),
            relro_hi = format_args!("{hi:#x}"),
            size,
            members = relro_secs.len(),
            "PT_GNU_RELRO span"
        );
        if size == 0 {
            tracing::warn!("PT_GNU_RELRO span is empty; emitting a zero-size RELRO header");
        }
        segments.push(ProgramHeader {
            p_type: elf::PT_GNU_RELRO,
            p_flags: elf::PF_R,
            p_offset: lo_off,
            p_vaddr: lo,
            p_paddr: lo,
            p_filesz: size,
            p_memsz: size,
            p_align: 1,
        });
    }
    // Non-executable stack marker.
    segments.push(ProgramHeader {
        p_type: elf::PT_GNU_STACK,
        p_flags: elf::PF_R | elf::PF_W,
        p_offset: 0,
        p_vaddr: 0,
        p_paddr: 0,
        p_filesz: 0,
        p_memsz: 0,
        p_align: 0,
    });
    tracing::debug!(
        predicted_phnum = phnum,
        actual_segments = segments.len(),
        has_tls,
        has_eh_frame_hdr,
        has_relro,
        num_load,
        build_id = config.build_id,
        dynamic = dynamic.is_some(),
        segment_types = ?segments.iter().map(|s| s.p_type).collect::<Vec<_>>(),
        "segment count check"
    );
    // Hard invariant: the reserved program-header count MUST equal the number of
    // segments emitted, or e_phnum disagrees with the actual headers and the
    // loader reads garbage. Enforce in all builds (not just debug), since a
    // mismatch silently corrupts the binary.
    if segments.len() as u64 != phnum {
        return Err(LayoutError::PhdrCountMismatch {
            predicted: phnum,
            actual: segments.len() as u64,
        });
    }

    // ── Linker-defined symbol addresses ──────────────────────────────────────
    let mut image_end = base;
    let mut edata = base;
    let mut bss_lo: Option<u64> = None;
    for s in &sections {
        if s.sh_flags & elf::SHF_ALLOC == 0 {
            continue;
        }
        let s_end = s.sh_addr + s.sh_size;
        image_end = image_end.max(s_end);
        if s.sh_type == elf::SHT_NOBITS {
            bss_lo = Some(bss_lo.map_or(s.sh_addr, |b| b.min(s.sh_addr)));
        } else {
            edata = edata.max(s_end);
        }
    }
    let bss_start = bss_lo.unwrap_or(edata);

    // Final VAs for common symbols (the synthetic `.bss` with no contributions).
    let common_base = sections
        .iter()
        .find(|s| s.source == SecSource::Bss && s.contributions.is_empty())
        .map(|s| s.sh_addr)
        .unwrap_or(0);
    let common: Vec<(SymbolId, u64)> = common_offsets
        .iter()
        .map(|(id, off)| (*id, common_base + off))
        .collect();

    Ok(Layout {
        output_sections: sections,
        addresses,
        segments,
        entry,
        e_type: if config.pie {
            elf::ET_DYN
        } else {
            elf::ET_EXEC
        },
        phoff: elf::EHDR_SIZE,
        phnum,
        shoff,
        shnum,
        shstrndx,
        file_size,
        got_slots: got_syms.to_vec(),
        got_base,
        strtab,
        shstrtab,
        symtab,
        image_base: base,
        bss_start,
        edata,
        end: image_end,
        common,
        dyn_blobs,
        plt_slots: plt_syms.to_vec(),
        plt_base,
        tls_size,
        tls_offsets,
    })
}

/// Compute the set of input sections reachable from the GC roots
/// (`--gc-sections`). Roots are the entry symbol's section plus init/fini arrays;
/// edges run from a section to the sections defining the symbols its relocations
/// reference. Returns the live `(object_id, input_section_index)` set.
/// S3-GC: Optimal Level-Synchronous Parallel Section Garbage Collection.
///
/// Implements QUAD Algorithm 3.1 (Tithi, Fogel, Chowdhury 2022) using
/// `ws-deque`'s Chase-Lev work-stealing deque for parallel edge expansion:
///
/// - Adaptive thread count `Pₗ = min(P, |frontier|)` — never wastes threads
///   on sparse BFS levels.
/// - Level-synchronous: barrier between levels via thread join.
/// - Lock-free hot path: each thread owns a `Worker` deque; thieves steal
///   across them during imbalanced levels.
///
/// Complexity: O((m+n)/P + D·log P) matching the theoretical lower bound.
pub fn gc_sections(
    objects: &[InputObject],
    symbols: &SymbolTable,
    entry_symbol: &str,
) -> FxHashSet<(usize, usize)> {
    let resolve_target =
        |obj_id: usize, sym: &peony_object::InputSymbol| -> Option<(usize, usize)> {
            if sym.binding == peony_object::Binding::Local {
                sym.section.map(|s| (obj_id, s.0))
            } else {
                let res = symbols.lookup(&sym.name)?;
                let def = res.defined_in?;
                res.section_index.map(|si| (def.0 as usize, si))
            }
        };

    // Build root set.
    let mut frontier: Vec<(usize, usize)> = Vec::new();
    let mut live: FxHashSet<(usize, usize)> = FxHashSet::default();

    if let Some(res) = symbols.lookup(entry_symbol.as_bytes()) {
        if let (Some(def), Some(si)) = (res.defined_in, res.section_index) {
            let key = (def.0 as usize, si);
            if live.insert(key) {
                frontier.push(key);
            }
        }
    }
    for (obj_id, obj) in objects.iter().enumerate() {
        for sec in &obj.sections {
            let keep = sec.name.starts_with(b".init")
                || sec.name.starts_with(b".fini")
                || sec.name.starts_with(b".preinit_array")
                || (sec.flags & 0x0020_0000) != 0;
            if keep && sec.flags & peony_object::elf::SHF_ALLOC != 0 {
                let key = (obj_id, sec.index.0);
                if live.insert(key) {
                    frontier.push(key);
                }
            }
        }
    }

    // S3-GC BFS using ws-deque Chase-Lev workers.
    // Number of threads: adaptive per level (Pₗ = min(P, |frontier|)).
    let max_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    while !frontier.is_empty() {
        // Adaptive: use at most as many threads as frontier items, up to max_threads.
        let pl = max_threads
            .min(frontier.len())
            .min(frontier.len().div_ceil(S3GC_GRAIN_SIZE))
            .max(1);

        if pl == 1 {
            // Serial fast path for sparse levels (avoids thread spawn overhead).
            let mut next: Vec<(usize, usize)> = Vec::new();
            for &(obj_id, sec_idx) in &frontier {
                let Some(obj) = objects.get(obj_id) else {
                    continue;
                };
                let Some(&pos) = obj.section_map.get(&sec_idx) else {
                    continue;
                };
                let Some(sec) = obj.sections.get(pos) else {
                    continue;
                };
                for reloc in &sec.relocs {
                    let Some(sym) = obj
                        .symbols
                        .get(*obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX))
                    else {
                        continue;
                    };
                    if let Some(key) = resolve_target(obj_id, sym) {
                        if live.insert(key) {
                            next.push(key);
                        }
                    }
                }
            }
            frontier = next;
            continue;
        }

        // Parallel path: distribute frontier across pl Chase-Lev workers.
        // Each thread owns one Worker and may steal from others via Stealers.

        // One Worker per thread; seed round-robin, then move each into its thread.
        let workers: Vec<Worker<(usize, usize)>> = (0..pl).map(|_| Worker::new()).collect();
        // Build stealers before consuming workers.
        let stealers: Vec<_> = workers.iter().map(|w| w.stealer()).collect();

        for (i, &item) in frontier.iter().enumerate() {
            workers[i % pl].push(item);
        }
        frontier.clear();

        let results: Arc<std::sync::Mutex<Vec<(usize, usize)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        // Quiescence counter: incremented when a thread goes idle, decremented when
        // it finds work again. When idle_count == pl and a full steal round finds
        // nothing, the level is exhausted. This is the standard Chase-Lev drain.
        let idle_count = Arc::new(AtomicUsize::new(0));

        // Move each Worker (owner) into its thread; thieves use cloned Stealers.
        std::thread::scope(|scope| {
            for (t, worker) in workers.into_iter().enumerate() {
                let all_stealers: Vec<_> = stealers.iter().map(|s| s.clone()).collect();
                let results = Arc::clone(&results);
                let idle_count = Arc::clone(&idle_count);

                scope.spawn(move || {
                    let mut local_out: Vec<(usize, usize)> = Vec::new();
                    let mut is_idle = false;

                    loop {
                        // Try own deque first.
                        if let Some(item) = worker.pop() {
                            if is_idle {
                                idle_count.fetch_sub(1, Ordering::Release);
                                is_idle = false;
                            }
                            let (obj_id, sec_idx) = item;
                            let Some(obj) = objects.get(obj_id) else {
                                continue;
                            };
                            let Some(&pos) = obj.section_map.get(&sec_idx) else {
                                continue;
                            };
                            let Some(sec) = obj.sections.get(pos) else {
                                continue;
                            };
                            for reloc in &sec.relocs {
                                let Some(sym) = obj.symbols.get(
                                    *obj.symbol_map.get(&reloc.symbol.0).unwrap_or(&usize::MAX),
                                ) else {
                                    continue;
                                };
                                if let Some(key) = resolve_target(obj_id, sym) {
                                    local_out.push(key);
                                }
                            }
                            continue;
                        }

                        // Own deque empty: try stealing from all siblings (including self
                        // via Retry-safe index skipping).
                        let mut found = false;
                        for (i, s) in all_stealers.iter().enumerate() {
                            if i == t {
                                continue;
                            }
                            match s.steal() {
                                Steal::Success(item) => {
                                    if is_idle {
                                        idle_count.fetch_sub(1, Ordering::Release);
                                        is_idle = false;
                                    }
                                    let (obj_id, sec_idx) = item;
                                    let Some(obj) = objects.get(obj_id) else {
                                        continue;
                                    };
                                    let Some(&pos) = obj.section_map.get(&sec_idx) else {
                                        continue;
                                    };
                                    let Some(sec) = obj.sections.get(pos) else {
                                        continue;
                                    };
                                    for reloc in &sec.relocs {
                                        let Some(sym) = obj.symbols.get(
                                            *obj.symbol_map
                                                .get(&reloc.symbol.0)
                                                .unwrap_or(&usize::MAX),
                                        ) else {
                                            continue;
                                        };
                                        if let Some(key) = resolve_target(obj_id, sym) {
                                            local_out.push(key);
                                        }
                                    }
                                    found = true;
                                    break;
                                }
                                // Retry = victim non-empty but we lost the CAS; loop again.
                                Steal::Retry => {
                                    found = true;
                                    break;
                                }
                                Steal::Empty => {}
                            }
                        }

                        if found {
                            continue;
                        }

                        // Nothing found — mark idle if not already.
                        if !is_idle {
                            idle_count.fetch_add(1, Ordering::Release);
                            is_idle = true;
                        }

                        // Quiescence: all threads idle → level is exhausted.
                        if idle_count.load(Ordering::Acquire) >= pl {
                            break;
                        }
                        std::hint::spin_loop();
                    }

                    if !local_out.is_empty() {
                        results.lock().unwrap().extend(local_out);
                    }
                });
            }
            // No done.store() needed — threads self-terminate via quiescence.
        });

        // Dedup and build next frontier (serial — fast for typical sizes).
        let candidates = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
        for key in candidates {
            if live.insert(key) {
                frontier.push(key);
            }
        }
    }

    live
}

/// Place a list of builders contiguously starting at the current `va`, building
/// [`OutputSection`]s and recording per-input-section addresses.
fn place(
    builders: &[Builder],
    base: u64,
    va: &mut u64,
    sections: &mut Vec<OutputSection>,
    addresses: &mut FxHashMap<(usize, usize), u64>,
    sec_to_out: &mut FxHashMap<(usize, usize), usize>,
) {
    for b in builders {
        *va = align_up(*va, b.align);
        let sh_addr = *va;
        let sh_offset = sh_addr - base;
        let out_idx = sections.len();
        for c in &b.contributions {
            let key = (c.object_id, c.section_index);
            addresses.insert(key, sh_addr + c.offset);
            sec_to_out.insert(key, out_idx);
        }
        sections.push(OutputSection {
            name: b.name.clone(),
            kind: b.kind,
            sh_type: b.sh_type,
            sh_flags: b.sh_flags,
            sh_addr,
            sh_offset,
            sh_size: b.size,
            sh_link: 0,
            sh_info: 0,
            sh_addralign: b.align,
            sh_entsize: match b.sh_type {
                elf::SHT_SYMTAB | elf::SHT_DYNSYM => elf::SYM_SIZE,
                elf::SHT_RELA => elf::RELA_SIZE,
                elf::SHT_DYNAMIC => 16,
                elf::SHT_HASH => 4,
                _ => 0,
            },
            sh_name: 0,
            shndx: 0,
            contributions: b.contributions.clone(),
            source: match b.name.as_str() {
                ".got" => SecSource::Got,
                ".note.gnu.build-id" => SecSource::NoteBuildId,
                ".interp" => SecSource::Interp,
                ".hash" => SecSource::Hash,
                ".gnu.hash" => SecSource::GnuHash,
                ".dynsym" => SecSource::DynSym,
                ".dynstr" => SecSource::DynStr,
                ".rela.dyn" => SecSource::RelaDyn,
                ".dynamic" => SecSource::Dynamic,
                ".plt" => SecSource::Plt,
                ".got.plt" => SecSource::GotPlt,
                ".rela.plt" => SecSource::RelaPlt,
                ".gnu.version" => SecSource::GnuVersion,
                ".gnu.version_r" => SecSource::GnuVersionR,
                ".eh_frame_hdr" => SecSource::EhFrameHdr,
                _ if b.is_nobits => SecSource::Bss,
                _ => SecSource::Input,
            },
        });
        *va += b.size;
    }
}

/// Build the `.symtab` plan and `.strtab` bytes from the defined global symbols.
/// Build the `.symtab` plan: local symbols first (precomputed addresses), then
/// defined globals. Returns `(entries, strtab, num_locals)`; `num_locals + 1` is
/// the symtab's `sh_info` (index of the first global).
fn build_symtab_plan(
    symbols: &SymbolTable,
    objects: &[InputObject],
    addresses: &FxHashMap<(usize, usize), u64>,
    sec_to_out: &FxHashMap<(usize, usize), usize>,
) -> (Vec<SymEntry>, Vec<u8>, usize) {
    use peony_object::Binding;

    let mut strtab: Vec<u8> = vec![0];
    let mut locals: Vec<SymEntry> = Vec::new();

    // Local symbols, in object then symbol order (deterministic).
    for (obj_id, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.binding != Binding::Local || sym.is_undefined || sym.is_common {
                continue;
            }
            if sym.name.is_empty() {
                continue; // section/file symbols carry no useful name here
            }
            let Some(si) = sym.section else { continue };
            let Some(&idx) = sec_to_out.get(&(obj_id, si.0)) else {
                continue; // dropped (gc/comdat) or non-allocatable section
            };
            let Some(&sec_va) = addresses.get(&(obj_id, si.0)) else {
                continue;
            };
            let name_off = strtab.len() as u32;
            strtab.extend_from_slice(&sym.name);
            strtab.push(0);
            locals.push(SymEntry {
                name: sym.name.clone(),
                name_off,
                shndx: (idx + 1) as u16,
                info: elf::st_info(elf::STB_LOCAL, elf::STT_NOTYPE),
                local: Some((sec_va + sym.value, sym.size)),
            });
        }
    }
    let num_locals = locals.len();

    // Defined globals, sorted by name.
    let mut defined: Vec<(&[u8], peony_symbols::SymbolResolution)> = symbols
        .iter()
        .filter(|(_, r)| r.is_defined())
        .map(|(n, r)| (n, r.clone()))
        .collect();
    defined.sort_by(|a, b| a.0.cmp(b.0));

    let mut plan = locals;
    for (name, res) in defined {
        let shndx = match res.section_index {
            None => elf::SHN_ABS,
            Some(si) => match sec_to_out.get(&(res.defined_in.unwrap().0 as usize, si)) {
                Some(&idx) => (idx + 1) as u16, // shndx = position + 1
                None => continue,               // defined in a dropped (non-allocatable) section
            },
        };
        let bind = match res.binding {
            Binding::Weak => elf::STB_WEAK,
            _ => elf::STB_GLOBAL,
        };
        let name_off = strtab.len() as u32;
        strtab.extend_from_slice(name);
        strtab.push(0);
        plan.push(SymEntry {
            name: name.to_vec(),
            name_off,
            shndx,
            info: elf::st_info(bind, elf::STT_NOTYPE),
            local: None,
        });
    }
    (plan, strtab, num_locals)
}

fn resolve_entry(
    symbols: &SymbolTable,
    addresses: &FxHashMap<(usize, usize), u64>,
    entry_symbol: &str,
) -> Result<u64> {
    let res = symbols
        .lookup(entry_symbol.as_bytes())
        .filter(|r| r.is_defined())
        .ok_or_else(|| LayoutError::NoEntry(entry_symbol.to_string()))?;
    match res.section_index {
        Some(si) => addresses
            .get(&(res.defined_in.unwrap().0 as usize, si))
            .map(|&va| va + res.value)
            .ok_or_else(|| LayoutError::NoEntry(entry_symbol.to_string())),
        None => Ok(res.value),
    }
}

// ── Post-layout symbol finalisation ─────────────────────────────────────────

/// Write each defined symbol's virtual address (`section_addr + value`) and GOT
/// slot address back into the symbol table. Must run after [`compute_layout`]
/// and before relocations are applied.
pub fn finalize_symbols(symbols: &mut SymbolTable, layout: &Layout) {
    for res in symbols.values_mut() {
        if res.defined_in.is_none() || res.common.is_some() {
            continue; // undefined, or a common handled below
        }
        match res.section_index {
            Some(si) => {
                let key = (res.defined_in.unwrap().0 as usize, si);
                if let Some(&va) = layout.addresses.get(&key) {
                    res.virtual_address = va + res.value;
                }
            }
            None => res.virtual_address = res.value, // SHN_ABS
        }
    }

    // Common symbols: address comes from the synthetic `.bss` allocation.
    let common_updates: Vec<(Vec<u8>, u64)> = layout
        .common
        .iter()
        .filter_map(|&(id, va)| symbols.name_by_id(id).map(|n| (n.to_vec(), va)))
        .collect();
    for (name, va) in common_updates {
        if let Some(r) = symbols.lookup_mut(&name) {
            r.virtual_address = va;
        }
    }

    let updates: Vec<(Vec<u8>, u64)> = layout
        .got_slots
        .iter()
        .enumerate()
        .filter_map(|(i, &id)| {
            symbols
                .name_by_id(id)
                .map(|n| (n.to_vec(), layout.got_base + (i as u64) * 8))
        })
        .collect();
    for (name, addr) in updates {
        if let Some(r) = symbols.lookup_mut(&name) {
            r.got_address = addr;
        }
    }

    // PLT stub addresses (so `call foo@PLT` resolves to the stub).
    let plt_updates: Vec<(Vec<u8>, u64)> = layout
        .plt_slots
        .iter()
        .enumerate()
        .filter_map(|(i, &id)| {
            symbols
                .name_by_id(id)
                .map(|n| (n.to_vec(), layout.plt_base + (i as u64) * 16))
        })
        .collect();
    for (name, addr) in plt_updates {
        if let Some(r) = symbols.lookup_mut(&name) {
            r.plt_address = addr;
        }
    }
}

/// Error if any *strong* (non-weak) referenced symbol remained undefined. Weak
/// undefined references are permitted and resolve to zero.
pub fn check_undefined(symbols: &SymbolTable) -> Result<()> {
    for (name, res) in symbols.iter() {
        if !res.is_defined() && res.binding != peony_object::Binding::Weak {
            return Err(LayoutError::Undefined(
                String::from_utf8_lossy(name).into_owned(),
            ));
        }
    }
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Whether an input section participates in the loadable image.
fn is_allocatable(name: &[u8], kind: SectionKind, flags: u64) -> bool {
    if flags & elf::SHF_ALLOC == 0 {
        return false;
    }
    if kind == SectionKind::Debug {
        return false;
    }
    // `.note` needs synthetic handling we don't do yet; skip it. `.eh_frame` is
    // kept (passed through) so the unwinder can find FDEs; `.eh_frame_hdr` is
    // synthesised separately.
    if name.starts_with(b".note") {
        return false;
    }
    true
}

fn perm_of(flags: u64) -> Perm {
    if flags & elf::SHF_EXECINSTR != 0 {
        Perm::Rx
    } else if flags & elf::SHF_WRITE != 0 {
        Perm::Rw
    } else {
        Perm::Ro
    }
}

/// Map an input section name to its output section name
/// (`.text._ZN…` → `.text`, `.rodata..L__unnamed` → `.rodata`).
fn output_section_name(name: &[u8]) -> String {
    let s = std::str::from_utf8(name).unwrap_or(".unknown");
    if let Some(dot2) = s.get(1..).and_then(|rest| rest.find('.')) {
        s[..dot2 + 1].to_string()
    } else {
        s.to_string()
    }
}

#[inline]
fn align_up(val: u64, align: u64) -> u64 {
    let align = align.max(1);
    (val + align - 1) & !(align - 1)
}
