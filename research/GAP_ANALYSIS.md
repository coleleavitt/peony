# peony — Engineering Gap Report

> Generated 2026-06-07 from a codegraph-indexed audit: 8 parallel crate auditors
> (compared against the vendored `mold/` reference, indexed in the same graph),
> followed by adversarial verification of every blocker/major claim against the
> real source. 81 findings checked → 78 confirmed/partial, 3 refuted.

## 1. Verdict

peony is an **architecturally complete skeleton, not a working linker**. The 9-pass
pipeline is wired end-to-end and compiles clean, but the final pass writes no ELF
header, no program headers, and no section headers — `emit_full`
(peony-emit/src/lib.rs:79-137) only truncates, mmaps, copies section bytes starting
at file offset `0x1000`, applies relocations, and flushes. **The output is a flat
blob with a zero-filled first page; offset 0 lacks the `7f 45 4c 46` magic, so the
kernel rejects it with ENOEXEC and no ELF tool can parse it.** Even with headers it
would be wrong: symbol VAs, GOT addresses, and PLT addresses are all hardwired to 0.
**Incremental linking — the entire thesis of the project — is ~0% functional**:
`--incremental` opens the cache into an unused `_cache`, logs "full diff NYI, falling
through" (peony/src/main.rs:92-96), and always runs a full link.

## 2. What's actually implemented

- **Object parsing (pass 2)**: ELF-64 `.o` parse via the `object` crate, section
  classification, reloc + symbol collection, mmap zero-copy, O(1) index maps
  (peony-object/src/lib.rs:188-306). Reloc-section→target linkage and `SHN_XINDEX`
  are handled transparently by `object` (refuted as gaps).
- **Symbol resolution (pass 2, serial)**: global table, weak/strong discrimination,
  undef placeholder + satisfaction, weak→global upgrade, duplicate-strong detection
  (peony-symbols/src/lib.rs:137-259). The serial path is correct and race-free.
- **Layout of input sections (passes 5/8)**: grouping by output-section name,
  per-contribution offset + alignment, VA/file-offset assignment, canonical segment
  ordering, 20% incremental capacity padding (peony-layout/src/lib.rs:155-236).
- **Relocation engine (pass 6/9)**: parallel per-object scan (the only real
  `par_iter`, peony-reloc/src/lib.rs:146), GOT/PLT slot identification by reloc type,
  and apply for the common core: R64, PC64, R32, R32S, PC32, PLT32, GOTPCREL,
  REX_GOTPCRELX, with 32-bit overflow checks (peony-reloc/src/lib.rs:258-294).
- **Cache scaffolding**: `IndexFile`/`CachedSymbol`/`SectionDiff` types, atomic
  write, bincode index load/save, version constant (peony-cache/src/lib.rs).
  Structures only — no logic consumes them.

## 3. Blockers (P0) — output is not a loadable ELF

| # | Missing | Location | One-line fix |
|---|---------|----------|--------------|
| P0-1 | **ELF file header** (`e_ident` magic, class, machine, `e_type`, `e_entry`) never written | peony-emit/src/lib.rs:79-137 (entire emit) | Define `Elf64_Ehdr`, serialize to bytes `0..64` at offset 0. |
| P0-2 | **Program headers** (PT_LOAD/PT_PHDR/…) never written | peony-emit + peony-layout (no phdr anywhere) | Build phdr table, write at `e_phoff`; kernel needs PT_LOAD to map. |
| P0-3 | **Layout never allocates ehdr/phdr region** — `file_off` just starts at `0x1000`, nothing fills `0..0xfff` | peony-layout/src/lib.rs:206 | Reserve and assign offsets/VAs to ehdr+phdr chunks. |
| P0-4 | **Segments never created** — `segment_order()` is only a sort key, not PT_LOAD grouping | peony-layout/src/lib.rs:255-266 | Group output sections into PT_LOAD by flags; compute `p_vaddr/p_filesz/p_memsz/p_flags`. |
| P0-5 | **Entry point `e_entry` / ET_EXEC vs ET_DYN never determined** — `Layout` has no entry/type field | peony-layout/src/lib.rs:113-120; peony-emit | Resolve `_start` VA, set `e_entry`, choose `e_type`. |

These five are the wall between "compiles" and "produces a file the kernel will load."

## 4. Full-link correctness (P1) — wrong even once headers exist

- **Symbol VAs never written back.** `compute_layout` takes `_symbols: &SymbolTable`
  (immutable, unused — peony-layout/src/lib.rs:157); `SymbolTable::lookup_mut`
  (peony-symbols/src/lib.rs:213) has zero callers; `virtual_address` is only ever set
  to 0 (peony-symbols/src/lib.rs:187,201). `apply_reloc` reads
  `s = resolution.virtual_address` = 0 (peony-reloc/src/lib.rs:236), so **every `S+A`
  / `S+A-P` reloc resolves as if all symbols sit at address 0.** `SymbolResolution`
  also lacks `section_index` and `value` fields (peony-symbols/src/lib.rs:67-78) —
  both must be added (carried from `InputSymbol`, peony-object/src/lib.rs:135,137) so
  VA = section VA + value can be computed.
- **GOT/PLT never wired.** Scan result is computed then discarded
  (peony/src/main.rs:136-137); `compute_layout` takes no scan param and creates no
  `.got`/`.plt` sections; emit builds a **fresh empty** `RelocScanResult::new()`
  (peony-emit/src/lib.rs:121,172); `got_address`/`plt_address` init to 0 and are
  never assigned (read at peony-reloc/src/lib.rs:239-240). Result: **PLT32 (every
  external call) and GOTPCREL/REX_GOTPCRELX compute against base 0.** `slot_set` is
  documented "filled by layout" (peony-reloc/src/lib.rs:99) but layout never fills it.
- **Section header table + `.shstrtab` never written** (peony-emit/src/lib.rs:79-137).
  Output is not re-linkable and unusable with `readelf`/`objdump`/`strip`.
  `OutputSection` also lacks `sh_type`/`sh_flags`/`sh_name`
  (peony-layout/src/lib.rs:66-84) — add them via a `SectionKind`→`SHT_*`/`SHF_*` map.
- **Undefined symbols silently pass.** Placeholders make `lookup` return `Some` for
  undefined globals, so the `UndefinedSymbol` arm is dead (peony-reloc/src/lib.rs:223-233);
  a link with unresolved refs exits 0 with relocs pointed at 0. Add a finalization
  pass that errors on `defined_in.is_none()`.
- **Archives / `.rlib` / DSOs unsupported.** `load_objects` calls `parse_object` on
  every path (peony/src/main.rs:113-122); ar magic isn't ELF so any `.a`/`.rlib`
  fails to parse. `iter_archive_members` (peony-object/src/lib.rs:321) has no caller;
  there are no `-l`/`-L` flags and no `ET_DYN`/`.dynsym` parsing. **Cannot link
  against std or any system library** — the multi-pass `resolve→mark_live→extract→
  re-resolve` model (mold passes.cc:308-397) is absent.
- **Reloc coverage holes** (all fall to the warn-and-skip arm at
  peony-reloc/src/lib.rs:290, leaving bytes unpatched, link "succeeds"):
  - **TLS entirely unimplemented** (no TLSGD/TLSLD/DTPOFF/TPOFF/GOTTPOFF/TLSDESC) →
    **any `thread_local!()` silently breaks**; `.tdata`/`.tbss` are dropped as
    `SectionKind::Other` at peony-layout/src/lib.rs:166, and there's no PT_TLS.
  - **GOT32** is scanned (peony-reloc:181) but has no `patch_buf` arm.
  - Small-int (8/16/PC8/PC16), SIZE32/64, GOTOFF64/PLTOFF64/GOTPC32 missing.
  - GOTPCRELX instruction relaxation absent (optional perf, not correctness).
- **COMDAT groups + common symbols** unhandled: two strong defs of one name
  hard-error `DuplicateSymbol` (peony-symbols/src/lib.rs:251) — exactly the C++
  inline/template and C tentative-definition cases that must be deduped/merged. No
  `SHT_GROUP` parsing.
- **`SHF_COMPRESSED` sections written raw.** `section.data()` returns compressed
  bytes (peony-object/src/lib.rs:201,213); emit copies them verbatim and relocates at
  uncompressed offsets → corruption. Common for `.debug_*`.
- **Page-congruence / bss**: PT_LOAD must satisfy `p_vaddr ≡ p_offset (mod page)` and
  `.bss` (NOBITS) must contribute `p_memsz` but zero `p_filesz` — both depend on the
  missing segment-creation step (P0-4).

## 5. The incremental thesis (P2) — ~0% implemented

This is the whole point of the project and **none of it runs.** Every function below
either has zero callers or is a stub:

- **Driver doesn't use the cache.** `--incremental` binds `IncrCache::open` to
  `_cache`, logs "full diff NYI, falling through", and always calls `emit_full`
  (peony/src/main.rs:92-98). The only thing the flag does is set
  `incremental_padding = 1.2` (main.rs:151).
- **`compute_diff` is a size-only stub, never called** (peony-cache/src/lib.rs:218-248):
  keys by object path instead of `(object, section_name)`; same-size = unchanged with
  no byte/hash compare; **never emits `Removed`**; no timestamps, no `.rlib` member
  byte-compare, no section-name matching.
- **Symbol persistence absent.** No `load_symbols`/`save_symbols`;
  `symtable.bin`/`symnames.bin` exist only in doc rows; `CachedSymbol` is orphaned;
  `SymbolTable` isn't serializable. odht persistent variant is doc-only "NYI".
- **Reloc reverse-index persistence absent.** `reloc_heads.bin`/`reloc_next.bin` are
  doc rows; no reader/writer; scan always runs full (no diff hook).
- **Red-green invalidation absent.** No code computes red/green regions;
  `emit_incremental` expects `red_sections: &HashSet<String>`
  (peony-emit/src/lib.rs:143) but has **zero callers**.
- **`emit_incremental` unsafe and dead.** `copy_sections_filtered` guards only against
  total file length, not `out_sec.capacity` (peony-emit/src/lib.rs:229-230) — a grown
  red section would silently overwrite its neighbor. No capacity-fit check, no
  full-relink fallback.

## 6. Parallelism & perf (P3) — "parallel linker" is mostly serial

- **Only `scan_relocations` is parallel** (`objects.par_iter()`,
  peony-reloc/src/lib.rs:146); rayon is imported only there.
- `load_objects` is serial `.iter().map()` despite the "in parallel" docstring
  (peony/src/main.rs:115).
- `copy_sections` / `copy_sections_filtered` are serial nested loops despite the
  "Parallel copy … via rayon" doc (peony-emit/src/lib.rs:192-234).
- `apply_all_relocations` is a serial triple loop (peony-emit/src/lib.rs:238-269).
- `resolve_symbols` is a serial `for` loop; peony-symbols advertises "parallel scan
  via rayon" (lib.rs:13) but has **no rayon usage at all**. (The current serial code
  is correct, so this is a perf gap, not correctness.)

## 7. Testing & validation (P4) — zero tests

There are no tests anywhere in the workspace. Minimum ladder, in order:

1. **Parse unit tests** — feed a known `.o`, assert section kinds, reloc tuples,
   symbol bindings.
2. **Reloc apply unit tests** — synthetic buffers, verify each `patch_buf` arm
   byte-for-byte against hand-computed `S+A`, `S+A-P`, overflow rejection.
3. **Golden ELF structure** — link a freestanding `_start.o` (no libc), then
   `readelf -h`/`-l` to assert magic, `e_entry`, PT_LOAD; `execve` it and assert exit
   code. **This is the "it links and runs" gate.**
4. **Cross-object link** — two `.o`s with a PLT32 call + GOTPCREL data ref across the
   boundary; execute, assert correct result (validates VA write-back + GOT/PLT).
5. **Incremental round-trip** — link, touch one `.o`, relink with `--incremental`,
   assert (a) output still executes correctly and (b) only the changed region's bytes
   differ from the prior output.
6. **Differential** — `readelf`/`objdump` structural diff of peony output vs
   `ld`/`mold` on the same inputs.

## 8. Suggested build order

From "compiles" → "links & runs hello-world" → "incremental works":

1. **Add ELF structs + writer** (`Elf64_Ehdr/Phdr/Shdr`) in peony-emit; write the
   header at offset 0 in `emit_full`. *(P0-1)*
2. **Allocate ehdr/phdr in layout** — reserve the page properly instead of the bare
   `0x1000` constant (peony-layout/src/lib.rs:206). *(P0-3)*
3. **Carry `section_index` + `value` into `SymbolResolution`; write back symbol VAs**
   after layout (give `compute_layout` `&mut` symbols or a finalize pass; call
   `lookup_mut`). *(P1)*
4. **Create synthetic `.got`/`.plt` OutputSections** sized from the scan result; stop
   discarding the scan (main.rs:137) and stop using empty `RelocScanResult::new()` in
   emit (peony-emit:121,172). *(P1)*
5. **Assign GOT/PLT slot addresses** into `slot_set` and into each symbol's
   `got_address`/`plt_address`. *(P1)*
6. **Build PT_LOAD segments** grouped by flags with `p_vaddr ≡ p_offset (mod page)`
   and bss `p_memsz`; set `e_entry` from `_start`; choose `e_type`. *(P0-2/4/5)*
7. **Write section header table + `.shstrtab`**; add `sh_type/sh_flags/sh_name` to
   `OutputSection`. *(P1)*
8. **Add undefined-symbol finalization** that errors before emit. *(P1)*
   → **Milestone: link & execute a freestanding hello-world; gate with test ladder 3-4.**
9. **Archive/`.rlib` support**: detect ar magic, drive `iter_archive_members`,
   multi-pass `mark_live`/extract resolution; add `-l`/`-L`. *(P1)*
10. **Broaden reloc coverage as real Rust objects demand** — TLS + `.tdata/.tbss` +
    PT_TLS first (thread_local), then GOT32/SIZE/GOTOFF/small-int; add COMDAT dedup +
    common-symbol merge; handle `SHF_COMPRESSED`. *(P1)*
11. **Parallelize** `load_objects`, `copy_sections`, `apply_all_relocations` with
    rayon. *(P3)*
12. **Implement incremental**: real `compute_diff` keyed on `(object, section)` with
    hash/byte compare + `Removed` detection; persistence (`symtable.bin`/
    `symnames.bin`, reloc reverse-index); red-green set derivation; wire
    `emit_incremental` with a capacity-fit check and full-relink fallback. *(P2)*
    → **Milestone: incremental round-trip test (ladder step 5).**
