<!-- Generated 2026-06-07 by codegraph-mining of lld + mold (8 subsystems, adversarially sourced). Companion to GAP_ANALYSIS.md (what is missing) and SPEC_AND_LITERATURE_DIGEST.md (authoritative spec values + literature). -->

# peony Build Blueprint: Reference-Grounded Roadmap to a Loadable Binary

## 1. Overview

This document is a concrete, reference-grounded build plan for **peony**, an incremental+parallel ELF linker (x86-64 Linux) that today is a compiling skeleton producing no loadable binary (see `/home/cole/RustProjects/active/peony/research/GAP_ANALYSIS.md`). It is synthesized from mined source data on how **lld** (LLVM) and **mold** implement each missing subsystem, with `file:line` citations throughout.

**Single most important meta-finding:** peony should adopt **mold's model** as its primary template for nearly every subsystem. mold's unified `Chunk` abstraction (headers are first-class layout objects), tagged-pointer `Symbol` (no vtable subclasses), index-first/VA-later GOT-PLT assignment, eager archive loading + explicit multi-pass resolve loop, and TBB per-thread-then-merge parallelism all map cleanly onto idiomatic Rust (enums, parallel arrays, rayon) and require far fewer coordinating data structures than lld's dual hierarchies. lld remains the better reference for two specific things: **RelExpr** (an explicit reloc-type→class abstraction worth understanding even if not copied) and **ICF** (mold has none). **Critically: neither lld nor mold is incremental** — both always do a full link. peony's incremental thesis is therefore its own invention; the references only supply the full-link foundation plus the in-memory data structures that peony must make *persistent and red-green-aware*. Treat every "incremental note" below as net-new engineering.

---

## 2. Per-Subsystem Blueprints

### 2.1 ELF Output: File / Program / Section Headers + Output Chunk Model

**peony gap & priority (P0-1,2,3,4 — top blocker).** emit writes no `Elf64_Ehdr`, no program headers, no section header table, no `.shstrtab`; output sections lack `sh_type`/`sh_flags`/`sh_name`; no synthetic header sections; segments never constructed.

**lld approach.** Separates data (`OutputSection` hierarchy) from segments (`PhdrEntry`/`Partition`, precomputed). `Writer::writeHeader` (`ELF/Writer.cpp:2877`) calls `writeEhdr` (`ELF/SyntheticSections.cpp:4402`, fills `e_ident`/machine/`e_phoff`/`e_phnum`/`e_shentsize`), then `writePhdrs` (`:4425`, serializes the precomputed `PhdrEntry` vector), then writes section headers last via `OutputSection::writeHeaderTo` for each `ctx.outputSections` (`Writer.cpp:2914-2915`).

**mold approach.** Unifies everything via a `Chunk` trait: `OutputEhdr`/`OutputPhdr`/`OutputShdr` are first-class chunks in `ctx.chunks` (`mold/src/passes.cc:123-135`). Two-phase write: `update_shdr` (post-layout) then `copy_buf` (final bytes). `OutputEhdr::copy_buf` (`mold/src/output-chunks.cc:63-109`) fills magic/`e_type`/`e_entry` (via `get_entry_addr:49`)/`e_phoff`/`e_shoff`/`e_shstrndx`. `OutputPhdr::update_shdr` (`:373`) calls `create_phdr` (`:153-361`) to group ALLOC sections; `copy_buf` (`:388-390`) serializes via `write_vector`. `OutputShdr::copy_buf` (`:112-126`) writes `Elf_Shdr` at `hdr[chunk->shndx]`. `ShstrtabSection::update_shdr` (`:496-510`) builds a dedup'd offset map; `copy_buf` (`:513-520`) writes null-terminated strings. `create_synthetic_sections` (`mold/src/passes.cc:108`).

**Which to follow & why.** **mold.** One data structure (`Chunk` in one `ctx.chunks` vector), one layout pass; headers flow through the same offset-assignment pipeline as data. lld's dual hierarchy is easier to debug but adds bookkeeping a young linker doesn't need. The cost is careful `shndx` tracking (`output-chunks.cc:124-125`).

**What peony should build.**
```rust
struct OutputChunk { name: String, shdr: Elf64_Shdr, sh_offset: u64, sh_addr: u64,
                     shndx: u32, contributions: Vec<(ObjectId, SectionIndex)> }
struct OutputEhdr { shdr: Elf64_Shdr }                       // phdr offset/size precomputed
struct OutputPhdr { shdr: Elf64_Shdr, phdrs: Vec<Elf64_Phdr> } // computed in update_shdr
struct ShStrTab  { shdr: Elf64_Shdr, /* dedup offset map */ }
```
Order: (1) `create_synthetic_sections` — instantiate ehdr/phdr/shdr/shstrtab chunks, **reserve page 0 for ehdr+phdr** (peony currently hardcodes `file_offset = 0x1000` at `peony-layout:206`); (2) `compute_layout` — assign `sh_offset`/`sh_addr` to all chunks incl. headers; (3) phase-assign `chunk.shndx`; (4) build `.shstrtab` offsets; (5) `finalize_phdr` (group by `sh_flags`, compute `PT_LOAD`/`PT_TLS`, set `p_vaddr`/`p_filesz`/`p_memsz`/`p_align`); (6) emit — `copy_buf` on ehdr, phdr, shstrtab, shdr. Cite `mold/src/passes.cc:108-214`, `output-chunks.cc:63-126,153-390,496-520`.

**Incremental implications.** ehdr/phdr/shdr are almost always **red** (every link changes `e_entry`/`e_shoff`/segment grouping). `.shstrtab` is green only if section names+counts are identical. Persist `ctx.phdr->phdrs` and diff vs prior; if segment structure changes, fall back to full relink. peony currently discards scan results (`main.rs:137`) and rebuilds phdr/shdr every time.

**Gotchas.** SHN_XINDEX (`shndx >= 65280` → `e_shstrndx = 65535`, true index in `shdr[0].sh_link`; `mold:80-83`, `lld:2906-2912`). `e_shnum` overflow >65535 → `e_shnum=0`, count in `shdr[0].sh_size` (`mold:119-121`). Page congruence `p_vaddr ≡ p_offset (mod page)` (`mold:167,250-251`). NOBITS contributes `p_memsz` but zero `p_filesz` (`mold:162-171,185-186`). `sh_name` are `.shstrtab` indices, dedup them (`mold:498-505`). Phdr order is fixed: PT_PHDR, PT_INTERP, PT_NOTE, PT_LOAD, PT_TLS, PT_DYNAMIC, PT_GNU_EH_FRAME, PT_GNU_PROPERTY, PT_GNU_STACK, PT_GNU_RELRO (`mold:215-305`). Segment contiguity check (`mold:250-251`). `.eh_frame` carries addresses needing relocation in `copy_buf` (`mold:2468,2495`) — don't double-apply.

---

### 2.2 Segments / PT_LOAD Creation: Address Assignment, Page Congruence, BSS, e_entry

**peony gap & priority (P0-2/4/5 — blockers).** No program headers; `segment_order()` sorts but never groups; page congruence unimplemented; `.bss` NOBITS unhandled; `e_entry`/`e_type` never set.

**lld approach.** Two-phase. `createPhdrs` (`ELF/Writer.cpp:2312-2510`) dynamically creates `PT_LOAD` via `addHdr`, appends compatible sections via `PhdrEntry::add` (`Writer.cpp:820`); `PhdrEntry` struct at `SyntheticSections.h:1460`. Addresses assigned separately in `LinkerScript::assignAddresses`. `setPhdrs` (`Writer.cpp:2711-2750`) finalizes `p_filesz = last->offset - first->offset + last->size` (only if non-NOBITS, `:2735-2736`), `p_memsz`, `p_offset`, `p_vaddr`. Entry via `getEntryAddr` (`:2852-2866`); `ET_EXEC`/`ET_DYN` via `getELFType` (`:2869`).

**mold approach.** Integrated single-pass-to-convergence. `set_osec_offsets` (`passes.cc:2986`) loops until phdr size stabilizes (`:2989-3020`), calling `set_virtual_addresses_regular` (`:2668-2777`, page-aligns on flag change `:2723-2741`, TBSS overlap `:2760-2770`), then `set_file_offsets` (`:2853-2923`) using **`align_with_skew(val, align, skew) = val + ((skew - val) & (align - 1))`** (`:2847`) to enforce `p_vaddr ≡ p_offset (mod page)` (`:2878`), NOBITS sets offset but doesn't advance fileoff (`:2869-2872`). `create_phdr` (`output-chunks.cc:153-370`) groups contiguous same-flag chunks via `define`/`append` lambdas (`:181-187`).

**Which to follow & why.** **mold.** Single-pass convergence eliminates lld's two-step finalization and separate linker-script layer; `align_with_skew` enforces congruence inline rather than bolted-on; segment creation is a simple contiguity loop, not a flag-transition state machine.

**What peony should build.**
```rust
pub struct ProgramHeader { p_type: u32, p_flags: u32, p_offset: u64, p_vaddr: u64,
    p_paddr: u64, p_filesz: u64, p_memsz: u64, p_align: u64,
    first_section: usize, last_section: usize }
pub struct ElfHeader { e_type: u16, e_entry: u64, e_machine: u16, e_flags: u32 }
```
Order: (1) add `sh_type`/`sh_flags`/`sh_offset` to `OutputSection`; (2) in `compute_layout` add a second pass assigning `sh_offset` via `align_with_skew` (CRITICAL — `mold/src/passes.cc:2847-2849,2875-2878`); (3) `create_program_headers(&[OutputSection], page_size) -> Vec<ProgramHeader>` grouping by flags (`mold/src/output-chunks.cc:153-305`, append logic `:181-187`); (4) resolve `e_entry` from `_start` (add `SymbolTable::lookup`), `e_type = ET_EXEC` for now; (5) `emit_full` encodes ehdr (64B @ off 0) + phdrs (56B each @ `e_phoff=64`) before `copy_sections`. Section-kind → flags map: Text→`SHF_ALLOC|SHF_EXECINSTR`, ReadOnly→`SHF_ALLOC`, Data→`SHF_ALLOC|SHF_WRITE`, Bss→`SHT_NOBITS|SHF_ALLOC|SHF_WRITE`. Files: `peony-layout/src/lib.rs` (struct+layout), `peony-emit/src/lib.rs` (header write).

**Incremental implications.** Full-link only in both refs. To make incremental: persist `Vec<ProgramHeader>` + per-section PT_LOAD membership; recompute phdrs only for red sections' bounding regions; if a phdr grew/shrank (e.g. new `.got` entry pushed bss), trigger full re-emit (can't patch in place without shifting later sections). Design `create_program_headers` to accept a `&mut cache` now for P2.

**Gotchas.** Page congruence via `align_with_skew` (`mold:2847-2849`) — independent alignment of vaddr/offset breaks loading. NOBITS `p_filesz < p_memsz` (`lld:2735-2736`). Flag-boundary → new PT_LOAD + page boundary (`lld:2407-2418`, `mold:2723-2741`). PT_TLS/TBSS overlaps following sections (`mold:2760-2770`). Phdr-size convergence loop (`mold:3011-3016`). Missing `_start` → default 0, don't crash (`lld:2859-2860`). `ET_EXEC` vs `ET_DYN` is P1. Zero-size sections excluded (peony `layout:170` already correct).

---

### 2.3 Synthetic Sections: GOT / PLT / GOTPLT / .symtab / .strtab — Sizing + Slot Assignment

**peony gap & priority (P1).** Reloc scan result computed then discarded (`main.rs:136-137`); emit makes a fresh empty `RelocScanResult` (`peony-emit:121,172`); `slot_set` never filled; `virtual_address`/`got_address`/`plt_address` hardwired 0 (`peony-symbols:187,201`).

**lld approach.** Two-pass: scan reserves slots via `sym->setGotIdx`/`setPltIdx`; synthetic sections hold `vector<Symbol*>` with `getSize() = entries * entry_size`. `GotSection::addEntry` (`SyntheticSections.cpp:677`), `PltSection::addEntry` (`:2624`), `GotPltSection::addEntry` (`:1220`). VAs computed lazily: `Symbol::getGotVA` (`Symbols.cpp:153-157`) = `got->getVA() + getGotIdx() * gotEntrySize`; `getPltVA` (`:176-188`). Indices live in `auxIdx → ctx.symAux[]` (parallel array). **Slot indices assigned at scan; VAs lazy at apply.**

**mold approach.** Three-level indexing. Scan flags symbols; `create_synthetic_sections` (`passes.cc:108-140`) instantiates `ctx.got`/`ctx.plt`/`ctx.pltgot`; `add_got_symbol`/`add_symbol` (`output-chunks.cc:1341,1679-1684,1757-1764`) append + assign `set_got_idx(ctx, size-1)` into `ctx.symbol_aux[sym->aux_idx]`. `Symbol::get_got_addr` (`mold.h:3247-3249`) = `ctx.got->shdr.sh_addr + get_got_idx() * sizeof(Word)`; `get_plt_addr` (`:3307-3311`). `get_got_entries` (`output-chunks.cc:1427-1523`) post-layout computes `(index, address, reloc_type)` tuples. `populate_symtab` (`:1718-1754`) writes `st_name`/`st_value`/`st_shndx`.

**Which to follow & why.** **mold.** Explicit scan→create+index→layout→address separation maps to Rust split-data/indexed-array patterns and to incremental persistence (indices stable across runs, VAs derived). lld is equivalent but buries indices in `auxIdx` indirection.

**What peony should build.**
```rust
pub struct SymbolResolution { id: SymbolId, binding: Binding, defined_in: Option<ObjectId>,
    section_index: usize /*ADD*/, value: u64 /*ADD*/,
    virtual_address: u64, got_address: u64, plt_address: u64 }
pub struct SymbolAux { got_idx: i32, plt_idx: i32 }  // -1 = not needed
pub struct SymbolTable { /* ... */ symbol_aux: Vec<SymbolAux> /*ADD parallel array*/ }
```
Order: (1) carry `section_index`+`value` through resolution; (2) in `compute_layout(scan)` (stop ignoring `scan`) build `.got`/`.plt`/`.got.plt` as synthetic `OutputSection`s, assign `got_idx`/`plt_idx` from `scan.slots` into `SymbolAux`; (3) post-VA pass: `got_address = got_section.va + got_idx*8`, same for plt; (4) post-layout symbol-VA fill: `VA = section_va + value`; (5) emit `.symtab`/`.strtab`: write `(st_name, st_info, st_shndx, st_value, st_size)`, strtab as null-separated blob.

**Incremental implications.** `got_idx`/`plt_idx` are **stable per symbol name** (deterministic scan order) — persist `SymbolAux` in cache. `file_offset`/`virtual_address` are NOT stable — recompute every link in the post-layout VA pass. Mark a symbol red if its `got_idx`/`plt_idx` or `virtual_address` changed. GOT/PLT sections are always red (entries depend on possibly-changed symbol VAs).

**Gotchas.** Distinct section names `.got`/`.got.plt`/`.plt` for independent alignment. Preallocate exactly `scan.slots.len()` — never add slots post-scan. Imported (DSO) symbols need `GLOB_DAT` relocs not just addresses (skip for static MVP). IFUNC may need two GOT slots (`mold:1438-1440`; treat as regular for MVP). `.symtab` locals first, then global/weak; set `sh_info` to first-global index. NOBITS consumes no file space (peony `layout:170` skips empty — extend for size>0 bss). `R_X86_64_GOTPCREL` is 32-bit PC-relative (peony `reloc:282`) but GOT *content* is 64-bit. `.got` align 8, `.plt` align 16. Use `got_idx = -1` sentinel.

---

### 2.4 Symbol Resolution + Final VA Write-Back + Undefined Handling

**peony gap & priority (P1).** Never writes symbol VAs back; `SymbolResolution` lacks `section_index`+`value`; undefined symbols silently resolve to 0; COMDAT hard-errors instead of deduping.

**lld approach.** Subclass polymorphism: `Symbol` base → `Defined`/`Undefined`/`CommonSymbol`/`SharedSymbol`. `Defined` holds `section*`/`value`/`size` (`Symbols.h:389-391`). Resolution rewrites in place via `Symbol::overwrite` (`Symbols.h:253-261`). VAs lazy via `getSymVA` (`Symbols.cpp:66`) / `getVA` (`:149`). `reportDuplicate` (`:526`). Undefined errors deferred to `handleUndefined` (`Driver.cpp:2345`). COMDAT: leader kept, others `discard()`. `computeBinding` (`:261`).

**mold approach.** Unified `Symbol<E>` with **tagged origin pointer** (`mold.h:2762-2768`: TAG_ABS/ISEC/OSEC/FRAG) — no subclasses. `resolve_symbols` (`input-files.cc:995-1022`) compares `get_rank` (`:929,948`) under mutex, writes `file`+`set_input_section(isec)`+`value`+`sym_idx`. `get_addr` (`mold.h:2684`, impl `input-files.cc:3171`) reads origin tag → base + value. `convert_common_symbols` (`:1103-1138`) synthesizes `.bss`. COMDAT via `mark_live` (`is_alive`). Archive extraction loop in `passes.cc:308-397`.

**Which to follow & why.** **mold.** Tagged origin (optional section index + value) is idiomatic Rust (enum + match, no downcasting); no vtable, no per-subclass allocation, local per-symbol locking.

**What peony should build.** Extend `SymbolResolution` with `section_index: Option<SectionIndex>` + `value: u64` (capture in `merge_symbol`, `peony-symbols:137-194`). Add after layout:
```rust
pub fn finalize_symbols(symbols: &mut SymbolTable, layout: &Layout) -> Result<()> {
    for (_, res) in symbols.iter_mut() {
        match res.section_index {
            Some(si) => if let Some(osec) = layout.output_sections.iter()
                          .find(|s| s.input_sections.contains(&si)) {
                res.virtual_address = osec.virtual_address + res.value; },
            None => res.virtual_address = res.value, // SHN_ABS
        }
    } Ok(())
}
pub fn check_undefined_symbols(symbols: &SymbolTable) -> Result<()> { /* err if defined_in.is_none() */ }
```
Pipeline: resolve → layout → `finalize_symbols` → `check_undefined` → apply relocations (read `virtual_address`) → emit. Reference `lld/ELF/Symbols.cpp:149`, `mold/src/mold.h:2684`.

**Incremental implications.** Persist symbol table with `virtual_address` to `peony-cache` (`CachedSymbol` exists but unused). If an input section is unchanged, its symbol VAs stay valid — skip recompute; if changed, mark its symbols red and recompute. Red-green split happens at emit: copy green sections, re-apply only red. This is mold-style full-relink-with-caching, not classic offline incremental.

**Gotchas.** Page-congruence gap means `VA = base + offset` needs a `segment_va` per `OutputSection`. NOBITS symbols: don't read from file. `S + A` uses `value` not pre-added VA; addends are small SHF_MERGE disambiguators. COMDAT/common: dead-mark losing sections before reloc scan (peony currently hard-errors at `peony-symbols:251` — switch to mark-dead). `STT_SECTION` → `section_base + addend`, not `+ symbol.value`. `SHN_ABS` has no section. Enforce visibility/binding (`lld computeBinding`) before writing symtab. Reloc scan happens before finalize and is discarded (`main.rs:136-137`) — after finalize, assign `got_address`/`plt_address`, do NOT re-scan.

---

### 2.5 x86-64 Relocation Scan + Apply: Type Coverage, TLS (GD/LD/IE/LE), GOTPCRELX Relaxation

**peony gap & priority (P1 — blocks libc + all thread-local code).** No TLS at all (`thread_local` breaks); only ~7 reloc types; GOT32 scanned but no apply arm; no PT_TLS, no `.tdata`/`.tbss`; no GOTPCRELX relaxation.

**lld approach.** Two-stage with explicit `RelExpr`. `getRelExpr` (`Arch/X86_64.cpp:364-422`) maps each `R_X86_64_*` → abstract `R_ABS`/`R_PC`/`R_GOT_PC`/`R_TPREL`/`R_TLSGD_PC`/etc. `relocate` (`:859-955`) dispatches on `rel.expr`. Four TLS relaxers: `relaxTlsGdToLe` (`:477-523`), `relaxTlsGdToIe` (`:526-565`), `relaxTlsIeToLe` (`:571-671`), `relaxTlsLdToLe` (`:674-712`), each byte-pattern-matching preceding instruction bytes. `getTlsGdRelaxSkip` (`:101-108`). `relaxGot` (`:1072`). Constructor sets `tlsDescRel`/`tlsGotRel`/`tlsModuleIndexRel`/`tlsOffsetRel` (`:77-99`).

**mold approach.** Unified scan+apply, no intermediate RelExpr. `scan_relocations` (`arch-x86-64.cc:753-859`) marks `NEEDS_GOT`/`NEEDS_GOTTP`/`NEEDS_TLSGD`/`NEEDS_TLSDESC`; for TLS checks `is_tprel_linktime_const`/`is_tprel_runtime_const`. `apply_reloc_alloc` (`:435-643`) computes S/A/P/G/GOT, big switch on `rel.r_type`: TLSGD (`:536-543`), GOTTPOFF (`:562-576`), GOTPCRELX (`:520-535`). Standalone relaxers: `relax_gotpcrelx` (`:131-159`), `relax_gottpoff` (`:162-203`), `relax_tlsdesc_to_ie/le` (`:206-291`), `relax_gd_to_le/ie` (`:297-368`), `relax_ld_to_le` (`:374-429`). Pre-computed `ctx.tp_addr`/`ctx.dtp_addr`/`ctx.tls_begin`.

**Which to follow & why.** **mold.** Flag-based design (`NEEDS_GOTTP`, `NEEDS_TLSGD`) maps to Rust enums; `relax_*` as stateless functions are easy unit tests and parallelize cleanly; peony already computes `SyntheticSlot::Got/Plt` in scan — extending to TLS is natural. lld's RelExpr would require a parallel intermediate type system peony lacks.

**What peony should build.** (1) Add reloc constants: GOTPCRELX/REX_GOTPCRELX/CODE_4_GOTPCRELX, GOTTPOFF/CODE_4/6_GOTTPOFF, TLSGD, TLSLD, TLSDESC, TLSDESC_CALL, GOTPC32_TLSDESC, DTPOFF32/64, TPOFF32/64, GOT32/64. (2) Extend `SyntheticSlot` (`peony-reloc:88-92`): `Gottp(SymbolId)`, `Tlsgd(SymbolId)`, `Tlsld`, `Tlsdesc(SymbolId)`, plus `TlsStart(usize)`. (3) `ApplyCtx` add `tp_addr`/`dtp_addr`/`tls_begin: u64`. (4) Add `relax_*` helpers mirroring `mold/src/arch-x86-64.cc:131-429` (memcpy hardcoded templates). (5) Extend `patch_buf` (`peony-reloc:258-294`) with full match incl. guards: `GOTTPOFF => if has_gottp { G+A-P } else { relax_gottpoff + (S-tp_addr) }`; TLSGD → `has_tlsgd`/`has_gottp`(`relax_gd_to_ie`)/else `relax_gd_to_le`; DTPOFF=`S+A-dtp_addr`, TPOFF=`S+A-tp_addr`. (6) Layout computes `.tdata`/`.tbss`, allocates PT_TLS, sets `tp_addr = .tdata_va + .tdata_size`, `dtp_addr = .tdata_va`. Emit order: scan (keep result) → layout TLS → assign slot addresses → apply → relaxers read `ctx.tp_addr`/`dtp_addr` (`mold:435-643`).

**Incremental implications.** Build a persistent reloc reverse-index (`reloc_heads.bin`/`reloc_next.bin`, symbol→reloc offsets) so round-2 can diff without rescanning. Red-green delta: recompute changed object's scan, compare `SyntheticSlot` sets — equal=green (reuse slot addrs), superset/subset=red. Persist `slot_set` (`peony-reloc:99`) to `slots.bin`. `emit_incremental` (currently dead, GAP item 5, `:229-230`) visits only red sections, falls back to full relink on capacity overflow. `tp_addr`/`dtp_addr` stable across rounds (depend only on total `.tdata` size); cache and re-derive — if size changes, TLS-referencing sections become red.

**Gotchas.** TLSGD/TLSLD are **two-reloc sequences** — TLSGD followed by PLT32/GOTPCREL (`mold:769-779`); apply must `++i` to skip the pair (`:540,542,548`). Instruction matching is byte-order sensitive: `0x488b05` via `(loc[-3]<<16)|(loc[-2]<<8)|loc[-1]` (`mold:164-180`). **TP points past end of `.tdata` on x86-64**: `TPOFF = S - (tdata_va + tdata_size)` (`mold:21-25`) — negating inverts all TPOFFs. DTPOFF (`S - dtp_addr` = start) vs TPOFF (`S - tp_addr` = end) (`mold:551-560`). GOT32=`G+A` (rare) vs GOTPCREL=`G+GOT+A-P`. GOTPCRELX relaxes only if `is_pcrel_linktime_const` (non-imported, `mold:526`) — track `is_imported`. APX CODE_4/CODE_6 variants have different prefixes (`mold:183-203`). PT_TLS must exist with VA/offset set before reloc apply (P0-2 dependency). Debug DTPOFF tombstones (`mold:712-722`) — deferred.

---

### 2.6 Archives (.a / .rlib) — Lazy Extraction, Mark-Live GC, ICF

**peony gap & priority (P1 — "cannot link against std or any system library").** Cannot load `.a`/`.rlib`; `iter_archive_members` has no caller; no multi-pass resolve→mark_live→extract loop; `--gc-sections` unimplemented; no ICF.

**lld approach.** Three symbol kinds incl. `LazySymbol` (`Symbols.h:514`) wrapping `Archive::Symbol`. `ArchiveFile::addLazySymbols` (`InputFiles.cpp:2190`); on resolution win, `ArchiveFile::fetch` (`:2278`) parses member into `ctx.objectFiles`, symbol replaced via placement-new. Multi-pass re-resolution after each extraction. `markLive` (`MarkLive.cpp:517`): roots = entry + exports + forced-keep; BFS over relocations; live = `partition=1`. ICF `ICF::run` (`ICF.cpp:100`): two-buffer equivalence classes, `equalsConstant`/`equalsVariable`, fuse to canonical.

**mold approach.** **Eager** — reads all members upfront. `read_archive_members` (`archive-file.cc:166`) detects `!<arch>` (fat, `read_fat_archive_members:128`) vs `!<thin>` (`:79`); all parsed as ObjectFile immediately, no lazy state. Explicit multi-pass loop (`passes.cc:308-345`): `resolve_symbols` (`input-files.cc:995`, rank-compare in place) → `mark_live_objects` (`passes.cc:216`, sets `is_reachable`) → redo if changed. GC (`gc-sections.cc:230`): `collect_root_set` (`:60`) + `build_start_stop_map` (`:43`) → `mark` BFS via `visit_section` (`:127`, depth cap 3 at `:136-139`) → `sweep` (`:190`). **No ICF.**

**Which to follow & why.** **mold.** No lazy-symbol wrapper (peony's `iter_archive_members` is already written for eager loading); explicit loop with clear states; no placement-new (un-idiomatic in Rust); `is_reachable`/`is_alive` flags + BFS are simple and incremental-compatible. Use lld's feeder/lazy model only if peony later adds `--as-needed`.

**What peony should build.** (1) Add `archive_name: Option<String>` to `InputObject`; in `load_objects` (`main.rs:113`) detect ar magic, call `iter_archive_members`, parse each, dedup by path via `HashMap<PathBuf, ObjectId>`. (2) Add `is_reachable: AtomicBool` (object) + `gc_root: bool` (symbol); wrap `resolve_symbols` + `mark_live_objects` in a fixed-point loop. (3) GC: add `is_alive: bool` to `InputSection`, `start_stop_sections: HashMap<String, Vec<SectionId>>` to `Layout`; `collect_root_set` → `mark` (BFS over relocs, handle `__start_`/`__stop_`) → `sweep`. (4) Skip dead sections/objects in `compute_layout`. (5) ICF deferred to P2 (`--fold-identical`, `replacement: Option<SectionId>`). Order: `load_objects` (with members) → resolve+mark_live loop → optional `gc_sections` → `compute_layout` → `emit_full`. Reference `mold/src/passes.cc:308-345`, `gc-sections.cc:60-230`.

**Incremental implications.** Persist `object_id → (archive_path, member_name, is_reachable_prior)` and `section_id → is_alive_prior`. Re-parse only changed archives (mtime/hash); thin archives need external-file mtime+hash tracking. Any object flipping reachable/dead or section flipping alive/dead triggers re-layout (or full relink on overflow). A new undefined ref appearing in a reachable object can pull new members — conservatively full-relink. Design `is_reachable`/`is_alive` as persistent fields now. For MVP keep archive+GC full-link only.

**Gotchas.** Dead NOBITS still counts toward `p_memsz` — set `is_alive=false` but don't drop from segment calc. Parse relocs before GC but resolve symbols first. Dedup identical `.o` across archives by file identity. Preserve symbol version through member parsing. `__start_`/`__stop_` only for referenced C-identifier sections (peony has no start-stop tracking). COMDAT dedup must run **after** archive extraction (`mold:355-390`) — peony's hard-error at `peony-symbols:251` is wrong, defer + dedup by group. SHF_MERGE fragments marked independently (peony drops them as `SectionKind::Other` at `layout:166`). Circular archive deps need the fixed-point loop (`mold:315-345`). ICF must run after resolution, before reloc assignment.

---

### 2.7 String Merging (SHF_MERGE) + .eh_frame / .eh_frame_hdr

**peony gap & priority (P1).** Classifies `MergeString`/`MergeConst`/`EhFrame` but does nothing: no merge map, no offset remapping, no CIE/FDE parsing, no `.eh_frame_hdr`. Blocks correct exceptions, smaller output, pass-6 reloc correctness.

**lld approach.** Two-phase. Parse: `.eh_frame` lazy-parsed into CIE/FDE pieces (`InputSection.cpp:1533-1550`). Merge: `EhFrameSection::addRecords` (`SyntheticSections.cpp:448`) parallel-iterates; `addCie` (`:404`) dedups by `(data, personality_symbol)` in `DenseMap`; dead FDEs skipped via `isFdeLive` (`:425`). `.eh_frame_hdr`: `EhFrameHeader::writeTo` (`:658`) builds sorted `(PC, offset)` binary-search table. Strings: `MergeInputSection` (`InputSection.cpp:1534`) + `MergeSyntheticSection` (`SyntheticSections.h:1083`). `EhReader::getFdeEncoding` (`EhFrame.cpp:166`), `hasLSDA` (`:188`).

**mold approach.** Lazy merging. Strings: `convert_mergeable_sections` (`input-files.cc:738`) wraps into `MergeableSection`; `split_contents` (`input-sections.cc:399`) finds null terminators / fixed-width pieces, hashes via `hash_string`; `MergedSection::insert` (`output-chunks.cc:2223`) uses `ConcurrentMap` (`lib/lib.h:413`) returning `SectionFragment` (`mold.h:269`); `compute_section_size` (`output-chunks.cc:2289`) assigns offsets in parallel. `resolve` (`:2264`). eh_frame: `parse_ehframe` (`input-files.cc:550`) reads records by size, id=0→CIE; `construct` (`output-chunks.cc:2383`) byte-compares CIEs (`cie_equals`), reuses leaders; dead FDEs `erase_if`. `EhFrameHdrSection` (`mold.h:1258`) 12B header + `num_fdes*8` sorted pairs.

**Which to follow & why.** **mold.** Single wrapping pattern (`InputSection → MergeableSection`), unified sequential flow (`split_contents → resolve_contents → compute_section_size`) easy to parallelize, proven sharded `ConcurrentMap` with no LLVM deps, lightweight `SectionFragment`, simpler byte-by-byte CIE iteration.

**What peony should build.**
```rust
struct SectionFragment { output_section_id: usize, offset: u64, p2align: u8, is_alive: bool, hash: u64 }
struct MergeableSection { parent_idx: usize, input_section: InputSection, p2align: u8,
    frag_offsets: Vec<u32>, hashes: Vec<u64>, fragments: Vec<usize> }
struct MergedSection { name: String, sh_flags: u64, sh_type: u32, sh_entsize: u64,
    members: Vec<usize>, frag_map: FxHashMap<Vec<u8>, usize>, /* HLL estimator */ size: u64, ... }
struct CieRecord { input_offset: usize, input_section_id: usize, is_leader: bool,
    output_offset: u64, contents: Vec<u8>, rels: Vec<(u32, usize)> }
struct FdeRecord { input_offset: usize, input_section_id: usize, is_alive: bool,
    output_offset: u64, cie_idx: usize, reloc_idx: usize }
```
Passes: load → `convert_mergeable_sections`; parse `.eh_frame` → `parse_ehframe`; layout 5a → `split_contents` (parallel, accumulate cardinality) → `resolve_contents` (size frag_map from estimator, insert) → `eh_frame_resolve` (linear leader byte-compare); addr-assign 8 → `merge_compute_size` (sorted-by-hash offsets, align) → `eh_frame_layout` → `eh_frame_hdr_layout` (12 + n*8); emit 9 → `merge_copy_buf`, `eh_frame_copy_buf` (apply relocs), `eh_frame_hdr_copy_buf` (sort by PC). Reloc fixup: in scan, when reloc targets a mergeable section, set addend to `frag->offset + intra_frag_offset`; in apply `S = frag.output_section.va + frag.offset + addend`. Reference `mold/src/output-chunks.cc:2178-2289,2383`, `input-sections.cc:399-450`, `input-files.cc:550-634`.

**Incremental implications.** Persist `MergedSection.frag_map` + fragment offsets keyed by hash (or `(file, section, local_offset)`). On relink: green fragment reuses prior offset if unchanged and non-overlapping; red fragments packed first, green at prior offsets if they fit; overflow → full relink. Persist `CieRecord.is_leader`+`output_offset`, reuse on byte-match. `.eh_frame_hdr` always recomputed (depends on possibly-shifted FDE PCs).

**Gotchas.** Every reloc/symbol referencing a fragment must be remapped to the dedup'd offset. CIE dedup key includes personality symbol id — never merge identical bytes with different personalities. Exclude NOBITS from merging. Include null terminator in `frag_offsets`/hashes/contents for SHF_STRINGS (no trailing null needed in output). Dead fragments filtered at `compute_section_size`. `.eh_frame` relocs re-applied to dedup'd offsets — don't assume prior values valid. FDE alive iff function not GC'd/folded/in live partition. CIE uniqueness is **global** across all files (`O(n²)` linear search, deterministic). Fragment alignment via `align_to(offset, 1<<p2align)`. **`.eh_frame_hdr` MUST be sorted by PC** (compute from FDE first-reloc symbol VA+addend) — unsorted breaks `dl_iterate_phdr` binary search. Decompress SHF_COMPRESSED before splitting.

---

### 2.8 Parallelism + Concurrency Model

**peony gap & priority (P3 parallelism, P2 incremental structures).** No parallelism in `load_objects`, `copy_sections`, `apply_all_relocations` (only `scan_relocations` uses rayon); no concurrent output-section map; no per-thread merge infra; no persistent symbol/reloc indices for red-green.

**lld approach.** Task-group barrier. `Writer::writeSections` (`Writer.cpp:2998-3023`) creates `parallel::TaskGroup`, enqueues each `OutputSection::writeTo` (two groups: reloc sections first, then rest); destructor waits. `computeHash` (`:3029-3044`) splits 1MB shards, `parallelFor` hashes each, then sequential reduce. No shared mutable state within tasks (disjoint buffer ranges).

**mold approach.** Two patterns. **Per-thread-then-merge** for symbol tables: `create_output_sections` (`passes.cc:621-686`) uses `tbb::enumerable_thread_specific<Map>` (`:629`), `cache.local()` (`:635`), `shared_mutex` (`:627`) for final merge. **Direct `parallel_for_each`** for stateless passes: `resolve_symbols` (`:319-321`), `mark_live_objects` (`:216-254`) each touch only their own file. **Chunked parallel hashing**: `write_build_id` (`:3350-3360`) `tbb::parallel_for` over shards + `madvise(MADV_DONTNEED)`, sequential reduce.

**Which to follow & why.** **mold + rayon.** rayon is the Rust analog of TBB with scope safety. For peony's incremental constraint specifically, rayon scope (per-thread collection) + a persistent global `odht` (stable ID assignment across rebuilds) wins — lld's manual task model has no per-thread state to persist.

**What peony should build** (in execution order). (1) Parallelize `load_objects` — `.par_iter().map()` + rayon scope for per-thread parse buffers (`main.rs:115-121`). (2) Symbol-table per-thread merge — `Vec<FxHashMap>` collected from `par_iter`, merged serially; pre-size by `objects.len()/num_threads`. (3) Collect per-object slot sets into `Vec<Vec<SyntheticSlot>>` (scan already parallel, `peony-reloc:146`). (4) Parallel section copy — replace nested loops (`peony-emit:192-210`) with `output_sections.par_iter().flat_map(contributions).for_each(...)` over disjoint buf ranges (use `IndexedParallelIterator`/`par_bridge` over `(section_idx, contrib_idx)`). (5) Parallel reloc apply — same shape. (6) Parallel build-id — 1MB chunks via `par_iter`, sequential reduce.

**Incremental implications.** The per-thread merge pattern **breaks** for persistent incremental: each full rebuild could assign different symbol IDs per thread. Solution: a global on-disk hash table (`odht`/sled) keyed by symbol name bytes — bump ID only for new symbols, reuse for existing. Persist: (1) symbol ID↔name bimap (currently `SymbolId(self.names.len())`, `peony-symbols:179`); (2) reloc reverse-index (`reloc_heads.bin`/`reloc_next.bin`, NYI `peony-cache:121-122`); (3) section→offset+capacity map; (4) GOT/PLT slot assignments (`slot_set`, `peony-reloc:99`). mold's design doesn't address any of this (always full link).

**Gotchas.** Page congruence + disjoint-range assertions before parallelizing writes. NOBITS: parallel copy skips them, parallel apply still processes their relocs. Symbol-VA write-back race — layout must barrier before emit (currently serial; incremental needs explicit sync). The scan-discard bug (`main.rs:136-137`): pass scan result to `emit_full`, don't recreate. Pre-allocate `FxHashMap` capacity to avoid collisions. Build-id chunks must align to hash output size (32 BLAKE3 / 64 SHA-256). rayon `scope` borrows `&mut` — verify no deadlock in merge path.

---

## 3. Cross-Cutting Recommendations

Four patterns recur across subsystems. peony lacks all four; all four are best taken from **mold**.

1. **The output "Chunk/Section" model (peony's biggest structural hole).** mold makes *everything* — ehdr, phdr, shdr, shstrtab, GOT, PLT, symtab, merged strings, eh_frame — a `Chunk` with `shdr`/`sh_offset`/`sh_addr`/`shndx` flowing through one layout pipeline (`output-chunks.cc:63-126`, `passes.cc:108-214`). peony has only ad-hoc `OutputSection`s and writes no headers. **Build the `OutputChunk` hierarchy first** (§2.1); every other subsystem hangs off it.

2. **The symbol `getVA()` / `get_addr()` abstraction.** Both refs compute addresses *lazily* from `section_base + value` (lld `Symbols.cpp:149`, mold `mold.h:2684`) rather than storing them. peony hardwires `virtual_address = 0`. Adopt mold's tagged-origin `get_addr` so VA, `got_addr`, `plt_addr` are all derived from one stored `(section_index, value)` pair + layout (§2.3, §2.4).

3. **Synthetic-section reservation-during-scan.** Slot indices (`got_idx`/`plt_idx`) are assigned at scan time and frozen; addresses derived post-layout (mold `mold.h:3247-3311`, lld `Symbols.cpp:153-188`). peony scans then *discards* (`main.rs:136-137`). Thread the `RelocScanResult` through to layout and emit.

4. **Per-thread-then-merge parallelism.** mold's `enumerable_thread_specific<Map>` + `shared_mutex` merge (`passes.cc:629-686`) is the shape for symbol-table construction; stateless passes use direct `parallel_for_each` (`:319-321`). In Rust: rayon `par_iter` + `Vec<FxHashMap>` collect + serial merge.

**Primary model to imitate: mold.** Justification — (a) one unified `Chunk`/`Symbol` representation instead of lld's dual `OutputSection`+`PhdrEntry` and four-subclass `Symbol` hierarchies, fewer data structures for a young codebase; (b) tagged enums + parallel index arrays are idiomatic Rust (lld's placement-new symbol rewriting and `auxIdx` indirection are not); (c) single-pass-to-convergence layout (`passes.cc:2986-3020`) avoids lld's multi-phase coordination; (d) stateless `relax_*` / `parallel_for_each` passes port directly to rayon. **Borrow from lld only:** the `RelExpr` concept (understand reloc-type→class mapping even without copying it) and **ICF** (`ICF.cpp:100-158`), which mold lacks entirely — defer to P2.

---

## 4. What to Study First (Ranked Reading List, in GAP_ANALYSIS Build Order)

Read these reference functions *before* writing each peony piece:

**Stage 1 — ELF header + chunk model:**
1. `mold/src/passes.cc:108-214` — `create_synthetic_sections` (the whole chunk-instantiation skeleton).
2. `mold/src/output-chunks.cc:63-126` — `OutputEhdr::copy_buf` + `OutputShdr::copy_buf` (exact header byte layout, `shndx` indexing).
3. `mold/src/output-chunks.cc:496-520` — `ShstrtabSection` (string dedup + write).

**Stage 2 — segments / PT_LOAD:**
4. `mold/src/passes.cc:2847-2849` — `align_with_skew` (the page-congruence formula — the single most load-bearing line).
5. `mold/src/passes.cc:2853-2923` — `set_file_offsets` (NOBITS handling, congruence application).
6. `mold/src/output-chunks.cc:153-305` — `create_phdr` (segment grouping, phdr order).
7. `lld/ELF/Writer.cpp:2734-2740` — `setPhdrs` (clearest `p_filesz`/`p_memsz` computation, NOBITS check).

**Stage 3 — symbol VA write-back:**
8. `mold/src/input-files.cc:995-1022` — `resolve_symbols` (rank compare + in-place write).
9. `mold/src/mold.h:2684` + `input-files.cc:3171` — `get_addr` (tagged-origin VA computation).

**Stage 4 — GOT/PLT:**
10. `mold/src/mold.h:3247-3311` — `get_got_addr`/`get_plt_addr` (index→address derivation).
11. `mold/src/output-chunks.cc:1427-1523` — `get_got_entries` (post-layout GOT content).

**Stage 5 — relocations + TLS:**
12. `mold/src/arch-x86-64.cc:435-643` — `apply_reloc_alloc` (the master dispatch switch, paired-reloc skip).
13. `mold/src/arch-x86-64.cc:131-203` — `relax_gotpcrelx` + `relax_gottpoff` (instruction-rewrite templates).
14. `mold/src/arch-x86-64.cc:297-429` — `relax_gd_to_le/ie`, `relax_ld_to_le` (TLS relaxation; note `tp_addr` past-end-of-`.tdata` comment at `:21-25`).

**Stage 6 — archives + GC:**
15. `mold/src/passes.cc:308-345` — the resolve→mark_live→redo fixed-point loop.
16. `mold/src/archive-file.cc:79-175` — `read_archive_members` (fat/thin detection).
17. `mold/src/gc-sections.cc:60-230` — `collect_root_set` → `mark` → `sweep`.

**Stage 6b — merge / eh_frame:**
18. `mold/src/input-sections.cc:399-450` — `split_contents`/`resolve_contents`.
19. `mold/src/input-files.cc:550-634` — `parse_ehframe`; `mold/src/output-chunks.cc:2383` — `construct` (CIE dedup).

**Stage 7 — incremental (no reference exists):** re-read mold's persistence-free structures above and design their on-disk forms (see §5). For parallelism shape: `mold/src/passes.cc:621-686` (`create_output_sections` per-thread merge).

---

## 5. Incremental-Specific Note (peony's Thesis — Net-New Engineering)

**Neither lld nor mold is incremental** — both rebuild everything every link. peony's differentiator therefore has no reference implementation; the refs only supply the full-link foundation and the *in-memory* structures peony must make **persistent + red-green-aware**. The concrete list, with the closest reference starting point for each:

| Structure to persist | Why | Red-green rule | Closest ref starting point |
|---|---|---|---|
| **Symbol ID↔name bimap** (`peony-symbols` `resolutions`+`names`) | IDs must stay stable across rebuilds to reuse in red sections | new symbol → new ID; existing → reuse | mold `Symbol`/`symbol_aux` (`mold.h:2756-2818`); replace incremental `SymbolId(names.len())` (`peony-symbols:179`) with global `odht` |
| **Symbol VAs** (`virtual_address`) | avoid recomputing for unchanged sections | section unchanged → VA green; changed → symbols red | `CachedSymbol` (peony-cache, unused) ← fill via `finalize_symbols` |
| **GOT/PLT slot map** (`SymbolAux.got_idx/plt_idx`, `slot_set` `peony-reloc:99`) | indices are deterministic/stable; addresses are not | slot changed → all referencing relocs red; GOT/PLT sections always red | `slots.bin` (NYI); mold `symbol_aux` |
| **Reloc reverse-index** (symbol→reloc offsets) | re-apply to red sections without rescanning | reloc set superset/subset → red | `reloc_heads.bin`/`reloc_next.bin` (NYI, `peony-cache:121-122`) — mold never stores this |
| **Program headers** (`Vec<ProgramHeader>`) | detect if segment structure changed | any flag/addr/size change → red, possibly full relink | mold `OutputPhdr.phdrs` (`output-chunks.cc:373`) |
| **Section→offset+capacity map** (`Layout`, in-memory only) | detect green sections, avoid re-copy; capacity = overflow guard | fits capacity → patch in place; overflow → full relink | mold `Chunk.shdr.sh_offset`/`sh_addr` |
| **Merge fragment map** (`MergedSection.frag_map` + offsets) | reuse string offsets across rebuilds | byte-identical fragment, non-overlapping → green | mold `ConcurrentMap`/`SectionFragment` (`output-chunks.cc:2223`, `mold.h:269`) |
| **CIE leaders** (`is_leader`+`output_offset`) | reuse eh_frame leader offsets | byte-identical CIE → green | mold `construct` (`output-chunks.cc:2383`) |
| **Archive membership + reachability** (`object→(archive,member,is_reachable)`; `section→is_alive`) | re-parse only changed archives; detect liveness flips | reachable/alive flip → re-layout; new undef ref → conservative full relink | mold `ObjectFile::archive_name`, `is_reachable`/`is_alive` (`passes.cc:216`) — design as persistent **fields** now |
| **TLS anchors** (`tp_addr`/`dtp_addr`) | stable if total `.tdata` size unchanged | size change → TLS-referencing sections red | mold `ctx.tp_addr`/`dtp_addr` (`arch-x86-64.cc:435-643`) |

**Mechanism.** Every "always red" item (ehdr/phdr/shdr, GOT/PLT) is recomputed each link; everything else is diffed against its persisted prior version. `emit_incremental` (currently dead/unsafe, GAP item 5, `peony-emit:229-230`) must: copy green sections in place, patch red sections that fit within their reserved capacity (the `incremental_padding` margin at `main.rs:151`, e.g. 1.2× sizing), and **fall back to `emit_full` on any capacity overflow or segment-structure change**. The honest framing: because the references are full-link, peony's "incremental" is best understood as *mold-style full-relink-with-aggressive-caching* — reuse cached VAs, slot indices, fragment offsets, and section bytes whenever layout hasn't moved, and full-relink whenever it has. The mold data structures above are the closest starting points; making them persistent and red-green-aware is the original contribution.
