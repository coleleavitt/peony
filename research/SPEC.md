# SPEC: peony Implementation Specification

**Version 0.1 — Derived from QUAD.md Theoretical Foundations**

> Every section below maps to a theorem or definition in `QUAD.md`.
> Every data structure has formal invariants. Every phase has pre/postconditions.

---

## 1. Crate Architecture

```
peony/                  # CLI driver binary
peony-object/           # ELF parse, archive iteration     (QUAD §1.1–1.2)
peony-symbols/          # Global symbol table              (QUAD §2)
peony-layout/           # Section grouping + addresses     (QUAD §4)
peony-reloc/            # GOT/PLT scan + reloc apply       (QUAD §5)
peony-cache/            # Incremental state persistence    (QUAD §6, §9)
peony-emit/             # Binary emission                  (QUAD §7)
```

---

## 2. Data Formats

### 2.1 ELF Input Representation

**Invariants on `InputSection`** (maps to Definition 1.2):

| Field | Type | Invariant |
|-------|------|-----------|
| `index` | `SectionIndex` | unique within the parent object |
| `name` | `Vec<u8>` | valid ELF section name bytes; may not be UTF-8 |
| `align` | `u64` | must be a power of two; minimum 1 |
| `size` | `u64` | equals `data.len()` for non-BSS; 0 ≤ data.len() for BSS |
| `flags` | `u64` | raw `sh_flags` bitmask from ELF header |
| `relocs` | `Vec<InputReloc>` | each `offset + width ≤ size` |

**Invariants on `InputSymbol`** (maps to Definition 1.3):

| Field | Type | Invariant |
|-------|------|-----------|
| `name` | `Vec<u8>` | raw bytes; may be empty for locals |
| `binding` | `Binding` | one of `{Local, Weak, Global}` |
| `is_undefined` | `bool` | `section.is_none() && value == 0` for true undefined |
| `value` | `u64` | offset within `section` if defined; 0 if undefined |

### 2.2 Global Symbol Table

**Invariants on `SymbolResolution`** (maps to Definition 2.1–2.2):

| Field | Type | Invariant |
|-------|------|-----------|
| `id` | `SymbolId(u32)` | unique across the table; dense 0..n |
| `binding` | `Binding` | reflects the winning definition's binding |
| `defined_in` | `Option<ObjectId>` | `None` iff unresolved (referenced but not defined) |
| `virtual_address` | `u64` | set by layout pass; 0 until then |
| `got_address` | `u64` | set by reloc-scan pass; 0 if not GOT-referenced |
| `plt_address` | `u64` | set by reloc-scan pass; 0 if not PLT-referenced |

**Table invariants:**

- `resolutions.len() == names.len()` at all times
- For each `(name, res)` in `resolutions`: `res.id.0 as usize < names.len()`
- Duplicate Global–Global definition → `SymbolError::DuplicateSymbol` (never stored)

### 2.3 Layout

**Invariants on `OutputSection`** (maps to Definition 4.1):

| Field | Invariant |
|-------|-----------|
| `align` | `align >= 1 && align.is_power_of_two()` |
| `size` | equals sum of `contrib.size` for all contributions, plus alignment padding between them |
| `capacity` | `capacity >= size`, `capacity` is a multiple of `align` |
| `virtual_address` | `virtual_address % align == 0`, `virtual_address >= base_address` |
| `file_offset` | `file_offset % align == 0`, `file_offset % page_size == virtual_address % page_size` (page congruence) |

**`addresses` map invariant:** For each `(object_id, section_index)` key, the value is `output_section.virtual_address + contribution.offset`.

### 2.4 Incremental Cache (on-disk format)

All files live under `<output>.incr/`:

```
<output>.incr/
├── index.bin          # bincode-encoded IndexFile
├── symnames.bin       # flat concatenation of symbol name bytes
├── symtable.bin       # array of CachedSymbol (fixed-size, mmap-able)
├── reloc_heads.bin    # u32 array indexed by SymbolId → first reloc index
├── reloc_next.bin     # u32 array indexed by reloc index → next reloc index
└── obj_copies/        # hard-linked copies of previous input files
    ├── <hash>/...
    └── ...
```

**`IndexFile` invariants:**

| Field | Invariant |
|-------|-----------|
| `version` | must equal `CACHE_VERSION` constant; otherwise full relink |
| `output_file_size` | must equal `stat(output).size`; otherwise full relink |
| `sections[i].capacity` | must be `>= sections[i].size` |
| `args_hash` | SHA-256 of the sorted, normalized linker argument list |

**`CachedSymbol` layout** (C-repr, fixed 28 bytes):
```
u32 name_offset    // byte offset into symnames.bin
u16 name_len       // byte length
u16 _padding       // reserved, must be 0
u64 virtual_address
u64 got_address
u64 plt_address    // (wait for plt support)
```

**Relocation reverse index invariant:** For symbol `sid`, the list
`reloc_heads[sid] → reloc_next[r0] → reloc_next[r1] → ...` terminates with `u32::MAX` and enumerates exactly the set of relocations in the current link that reference `sid`.

---

## 3. Pipeline Phases

The pipeline follows MaskRay's 9-pass model (QUAD §1) extended with the incremental path.

### Phase 0: Argument Parsing

**Pre:** raw `argv` from OS  
**Post:** `Args` struct with validated and normalized fields

```
Args {
  inputs:          Vec<PathBuf>,         // object files, archives, shared libs
  output:          PathBuf,              // -o
  entry:           String,               // -e / --entry
  base_address:    u64,                  // --image-base
  incremental:     bool,                 // --incremental
  gc_sections:     bool,                 // --gc-sections
  pie:             bool,                 // --pie / -shared
  build_id:        bool,                 // --build-id
  threads:         usize,               // --threads (0 = all)
  defsyms:         Vec<(String, u64)>,   // --defsym
  search_paths:    Vec<PathBuf>,         // -L
  libs:            Vec<String>,          // -l
}
```

### Phase 1: Input File Loading (parallel)

**Pre:** `Args.inputs` validated  
**Post:** `Vec<InputObject>` with all sections and symbols parsed; archive members extracted

**Algorithm:**
```rust
// Parallel mmap + parse (by QUAD Theorem 5.1)
let objects: Vec<InputObject> = args.inputs
    .par_iter()
    .flat_map(|path| load_input(path))   // handles .o, .a, .rlib, .so
    .collect();
```

**Correctness:** Archive members are iterated by byte-comparison, not timestamp (QUAD §6.1, Wild design).

**Complexity:** $O(n_{\text{bytes}} / P)$ wall time where $P$ = rayon thread count.

### Phase 2: Symbol Resolution

**Pre:** `Vec<InputObject>`, `Args`  
**Post:** `SymbolTable` satisfying all invariants in §2.2; GOT/PLT slot set

**Sub-phases:**
1. `add_object()` for each input in command-line order (preserves Unix archive semantics)
2. `process_object()` for bare objects (unconditional)
3. Archive member activation: fixpoint — process member if it defines an undefined symbol
4. `--defsym` symbols added last

**Key algorithm:** `merge_symbol()` implements $\oplus$ from QUAD Definition 2.2.

**Resolution ordering** (critical for archive semantics — must not parallelize):
```
for input in args.inputs (in order):
    match input:
        Object(o)   → process_object(obj_id, o)
        Archive(a)  → fixpoint {
            for member in a.members:
                if member defines any undefined symbol in Σ:
                    process_object(obj_id, member)
        }
        SharedLib(d) → register_exports(d)
```

**Complexity:** $O(n)$ expected (QUAD Theorem 2.1).

### Phase 3: Section GC (if `--gc-sections`)

**Pre:** `SymbolTable` complete, live entry symbol known  
**Post:** `live_sections: FxHashSet<(ObjectId, SectionIndex)>`

**Algorithm:** S3-GC (QUAD Algorithm 3.1) — level-synchronous parallel BFS with adaptive thread count.

**Root set construction:**
- Section containing `entry` symbol
- All sections in `.init_array`, `.fini_array`, `.ctors`, `.dtors`
- All sections with `SHF_GNU_RETAIN`
- All sections contributing to linker-defined symbols

**Complexity:** $O((m+n)/P + D \log P)$ (QUAD Theorem 3.1).

### Phase 4: Create Synthetic Sections

**Pre:** Live sections known, GOT/PLT slot set from Phase 2  
**Post:** Synthetic sections added: `.got`, `.got.plt`, `.plt`, `.rela.dyn`, `.rela.plt`, `.dynamic`, `.dynsym`, `.dynstr`, `.hash`, `.interp`, `.note.gnu.build-id`

**Sizing rules:**
- `.got` size: `8 * (|GOT_slots| + 3)` (3 reserved entries)
- `.plt` size: `16 * (1 + |PLT_slots|)` (1 header stub)
- `.got.plt` size: `8 * (3 + |PLT_slots|)` (3 reserved)
- `.rela.plt` size: `24 * |PLT_slots|` (one Elf64_Rela per PLT slot)

### Phase 5: Section Layout (passes 5 + 8 in MaskRay model)

**Pre:** All sections known (live + synthetic), all sizes computed  
**Post:** `Layout` with `addresses` map populated, `file_size` set

**Algorithm:**

1. **Grouping** (`output_section_name()`): strip suffix after second `.`
2. **Sort** output sections by segment type order (Text < RO < EhFrame < MergeStr < InitArray < Data < BSS < Debug)
3. **Address assignment** (sequential within each output section):
   ```
   va = base_address
   for out_sec in sorted_sections:
     va = align_up(va, out_sec.align.max(page_size))
     out_sec.virtual_address = va
     out_sec.file_offset = ...  // page-congruent
     for contrib in out_sec.contributions:
       contrib.offset = current_size_within_section
       addresses[(obj_id, sec_idx)] = va + contrib.offset
     out_sec.size = sum of contributions + alignment padding
     out_sec.capacity = ceil(out_sec.size * incremental_padding)
     va += out_sec.capacity
   ```

**Page congruence invariant:** maintained by setting `file_offset = (va - base_address) % file_alignment * page_size / file_alignment`. Concretely: each PT\_LOAD segment has `p_offset ≡ p_vaddr (mod page_size)`.

**Complexity:** $O(|\mathcal{S}|)$.

### Phase 6: Relocation Scan

**Pre:** `SymbolTable` complete, layout not yet needed  
**Post:** `RelocScanResult` with all GOT/PLT slots identified

**Algorithm:** QUAD Algorithm 5.1 — fully parallel, $O(m/P)$.

### Phase 7: Finalize Synthetic Sections

**Pre:** GOT/PLT slot set finalized  
**Post:** GOT/PLT/PLT.GOT bytes computed; symbol VMA table updated with GOT/PLT addresses

**GOT layout:**
```
got[0] = address of .dynamic section (or 0)
got[1] = 0 (reserved for ld.so)
got[2] = 0 (reserved for ld.so)
got[3..] = one entry per GOT_slot symbol
```

PLT stubs (x86-64):
```asm
; PLT header (16 bytes)
pushq  (%rip + got[1] - here - 6)
jmpq   *(%rip + got[2] - here - 6)
nop; nop; nop; nop

; Per-symbol PLT stub (16 bytes)
jmpq   *symbol_got_slot(%rip)
pushq  $reloc_index
jmpq   PLT[0]
```

### Phase 8: Binary Emission

**Pre:** `Layout` complete, GOT/PLT addresses set in `SymbolTable`  
**Post:** Valid ELF-64 binary at `Args.output`

**Strategy:**
1. Open output file with `O_RDWR | O_CREAT`; if exists and size matches, reuse (avoids TLB shootdowns per QUAD §7)
2. `ftruncate(file, layout.file_size)` if needed
3. `MmapMut::map_mut(&file)` — single mapping, no re-mmapping
4. Write ELF header + program headers at offset 0
5. **Parallel** section copy: `par_iter` over `output_sections`; each thread owns a disjoint slice of the mmap
6. **Parallel** relocation apply: `par_iter` over (section, reloc) pairs — disjoint writes (QUAD Theorem 5.1)
7. Write section header table at `layout.file_size - shoff`
8. Compute and write build-id (Merkle SHA-256 over 1MiB chunks, in parallel)
9. `mmap.flush()`

**Build-id algorithm (mold-style Merkle tree):**
```
chunks ← split output into 1MiB blocks
chunk_hashes ← chunks.par_iter().map(|c| sha256(c))
build_id ← sha256(concat(chunk_hashes))
write to .note.gnu.build-id
```

**Complexity:** $O(|B|/P)$ wall time (QUAD §11 summary table).

### Phase 9: Incremental Cache Update

**Pre:** Successful link completed  
**Post:** `<output>.incr/` updated with new `index.bin`, `symtable.bin`, etc.

**Algorithm:**
```
cache.save_index(IndexFile {
    version: CACHE_VERSION,
    output_file_size: layout.file_size,
    sections: layout.output_sections.iter().map(|s| CachedSectionMeta {
        name: s.name.clone(),
        virtual_address: s.virtual_address,
        file_offset: s.file_offset,
        size: s.size,
        capacity: s.capacity,
    }).collect(),
    args_hash: sha256(normalized_args),
})
```

---

## 4. Incremental Path

### 4.1 Cache Validation

```
fn try_reuse(output: &Path, cache_dir: &Path, args_hash: &[u8; 32]) -> Result<bool>:
  if !output.exists() || !cache_dir.join("index.bin").exists():
    return Ok(false)
  
  index = decode(read(cache_dir/"index.bin"))?
  
  if index.version != CACHE_VERSION:         return Ok(false)
  if index.args_hash != *args_hash:          return Ok(false)
  if stat(output).size != index.output_size: return Ok(false)
  
  return Ok(true)
```

### 4.2 Diff Computation

**Algorithm (maps to QUAD Definition 1.6):**

```
fn compute_diff(current: &[InputObject], cache_dir: &Path) -> IncrementalDiff:
  // Step 1: Compare modification timestamps
  changed_files ← current.filter(|o| mtime(o.path) != cached_mtime(o.path))
  
  // Step 2: For .rlib archives with changed mtime, byte-compare each member
  // (rustc doesn't set timestamps inside archives)
  changed_sections ← []
  for file in changed_files:
    if is_archive(file):
      prev_members ← load_from_obj_copies(cache_dir, file.path)
      for (prev, cur) in zip(prev_members, file.members):
        if prev.data != cur.data:
          for sec in cur.sections:
            changed_sections.push(SectionDiff { status: Changed, ... })
    else:
      // Changed bare object: all sections are changed
      for sec in file.sections:
        changed_sections.push(SectionDiff { status: Changed, ... })
  
  // Step 3: New files → Added; Missing files → Removed
  ...
  
  IncrementalDiff { section_diffs: changed_sections, ... }
```

### 4.3 Red-Green Coloring

**Algorithm (maps to QUAD Definition 6.1 + Theorem 6.1):**

```
fn color_regions(diff: &IncrementalDiff, layout: &Layout, Σ: &SymbolTable)
    -> (FxHashSet<String>, FxHashSet<SymbolId>):
  
  // Find symbols whose address changed (moved symbols)
  moved_symbols ← Σ.iter()
    .filter(|(_, res)| res.virtual_address != cache.symbol_table[res.id].virtual_address)
    .map(|(_, res)| res.id)
    .collect()
  
  // Red output sections: contain any changed section, or reference a moved symbol
  red_sections ← {}
  
  for diff in diff.section_diffs.filter(|d| d.status != Unchanged):
    red_sections.insert(layout.output_section_of(diff.section))
  
  for out_sec in layout.output_sections:
    if out_sec.references_any(&moved_symbols):
      red_sections.insert(out_sec.name.clone())
  
  (red_sections, moved_symbols)
```

**Key property (QUAD Theorem 6.1):** All output sections not in `red_sections` produce byte-identical output; their file ranges need not be rewritten.

---

## 5. Symbol Pre-Hashing API

All symbol name lookups use `PreHashed<&[u8]>` (Wild's `PassThroughHashMap` pattern, QUAD Lemma 2.3):

```rust
/// Wrapper that carries a pre-computed hash alongside the key.
/// The `Hash` impl returns the stored hash; no re-hashing occurs.
#[derive(Clone)]
pub struct PreHashed<K> {
    hash: u64,
    key: K,
}

impl<K: Hash> PreHashed<K> {
    pub fn new(key: K) -> Self {
        let hash = FxHasher::hash(&key);
        Self { hash, key }
    }
}

impl<K> Hash for PreHashed<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}
```

**Contract:** The `FxHashMap<PreHashed<Vec<u8>>, SymbolId>` in `SymbolTable` must use `raw_entry_mut()` with the pre-computed hash and a custom byte-comparison closure to avoid copying symbol name bytes.

---

## 6. x86-64 Relocation API

```rust
/// Apply a single relocation in-place.
/// 
/// # Preconditions
/// - `buf.len() >= reloc.offset + width(reloc.r_type)`
/// - `ctx.symbols.lookup(sym_name).virtual_address != 0` (layout done)
/// - `section_va = vma(output_section) + contribution.offset`
/// 
/// # Postconditions  
/// - `buf[reloc.offset..reloc.offset + width]` contains the correct relocation value
/// - No other bytes in `buf` are modified
/// 
/// # Errors
/// - `RelocError::Overflow` if the computed value doesn't fit in the relocation width
/// - `RelocError::UndefinedSymbol` if the referenced symbol has no definition
pub fn apply_reloc(
    ctx: &ApplyCtx<'_>,
    obj: &InputObject,
    reloc: &InputReloc,
    section_va: u64,
    buf: &mut [u8],
) -> Result<(), RelocError>
```

**Width by type:**

| r_type | Width | Formula | Overflow check |
|--------|-------|---------|----------------|
| R64, PC64 | 8 | $S+A$ / $S+A-P$ | none |
| PC32, R32S | 4 | $S+A-P$ / $S+A$ | $v$ fits in `i32` |
| R32 | 4 | $S+A$ | $v$ fits in `u32` |
| GOTPCREL, REX_GOTPCRELX, PLT32 | 4 | $G+A-P$ / $L+A-P$ | `i32` |

---

## 7. Persistent State Schema

### 7.1 `index.bin` (bincode, serde-encoded)

```rust
#[derive(Serialize, Deserialize)]
pub struct IndexFile {
    pub version: u32,              // must == CACHE_VERSION
    pub output_file_size: u64,
    pub sections: Vec<CachedSectionMeta>,
    pub args_hash: [u8; 32],
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CachedSectionMeta {
    pub name: String,
    pub virtual_address: u64,
    pub file_offset: u64,
    pub size: u64,
    pub capacity: u64,             // invariant: capacity >= size
}
```

### 7.2 `symtable.bin` (C-repr flat array, mmap-able)

```rust
#[repr(C)]
#[derive(Copy, Clone)]
pub struct CachedSymbol {
    pub name_offset: u32,          // byte offset into symnames.bin
    pub name_len: u16,             // byte length of name
    pub _pad: u16,                 // must be 0
    pub virtual_address: u64,
    pub got_address: u64,
    pub plt_address: u64,
}
```

**Invariant:** `symtable.bin` length = `sizeof(CachedSymbol) * symbol_count`. Symbol at index `i` corresponds to `SymbolId(i as u32)`.

### 7.3 Relocation Reverse Index

```
reloc_heads.bin: [u32; symbol_count]    // reloc_heads[symbol_id] = first reloc index, or u32::MAX
reloc_next.bin:  [u32; reloc_count]     // reloc_next[reloc_id] = next reloc index, or u32::MAX
```

**Construction invariant:** Built with lock-free atomic `compare_exchange`:
```rust
// Thread-safe insertion of reloc_id into the list for symbol_id
loop {
    let old_head = reloc_heads[symbol_id].load(Relaxed);
    reloc_next[reloc_id].store(old_head, Relaxed);
    if reloc_heads[symbol_id].compare_exchange(old_head, reloc_id, Release, Relaxed).is_ok() {
        break;
    }
}
```

---

## 8. Test Oracle Specification

### 8.1 Correctness Oracle

Every link-and-run test must verify (using `readelf`, `objdump`, or execution):

1. **ELF validity**: `readelf -h output` exits 0
2. **No undefined dynamic symbols** (static link): `readelf -s output | grep NOTYPE | grep GLOBAL` is empty
3. **Page congruence**: for each `LOAD` segment, `p_vaddr % p_align == p_offset % p_align`
4. **Entry reachable**: `readelf -h output | grep Entry` shows a valid address in the `.text` range
5. **Execution**: `./output; echo $?` matches expected exit code

### 8.2 Incremental Correctness Oracle

For each incremental test:

1. **Full link**: `peony <inputs> -o output.full`
2. **First incremental**: `peony --incremental <inputs> -o output.incr`
3. **Assert**: `cmp output.full output.incr` (byte-identical)
4. **Edit**: modify one source file, recompile to produce `inputs_v2`
5. **Second incremental**: `peony --incremental <inputs_v2> -o output.incr`
6. **Full re-link**: `peony <inputs_v2> -o output.full2`
7. **Assert**: `cmp output.full2 output.incr` (byte-identical)
8. **Assert incremental was faster**: wall time of step 5 < 50% of wall time of step 6

### 8.3 Determinism Oracle

```bash
# Run the link 10 times; all outputs must be identical
for i in {1..10}; do
  peony <inputs> -o output.$i
done
md5sum output.* | awk '{print $1}' | sort -u | wc -l  # must print "1"
```

---

## 9. Configuration Constants

| Constant | Value | Source |
|----------|-------|--------|
| `CACHE_VERSION` | `1` | bump on any format change |
| `INCREMENTAL_PADDING` | `1.2` | 20% extra; from Wild design |
| `PAGE_SIZE` | `0x1000` | Linux x86-64 default |
| `BASE_ADDRESS` | `0x400000` | standard Linux ELF load address |
| `GOT_RESERVED_ENTRIES` | `3` | GOT[0]=dynamic, GOT[1]=ld.so, GOT[2]=ld.so |
| `PLT_HEADER_SIZE` | `16` | x86-64 PLT stub size |
| `PLT_ENTRY_SIZE` | `16` | x86-64 per-symbol PLT entry |
| `BUILD_ID_CHUNK_SIZE` | `1 MiB` | Merkle tree chunk size (mold design) |
| `SYMBOL_TABLE_LOAD_FACTOR` | `0.75` | FxHashMap resize threshold |
| `S3GC_GRAIN_SIZE` | `256` | rayon divide-and-conquer threshold for BFS |

---

## 10. Phase Dependency Graph

```
Args (Phase 0)
  ↓
InputLoad (Phase 1) ──────────────────┐
  ↓                                    │
SymbolResolution (Phase 2)             │ (parallel within phase)
  ↓                                    │
SectionGC (Phase 3, if --gc-sections)  │
  ↓                                    │
SyntheticSections (Phase 4)            │
  ↓                                    │
Layout (Phase 5)                       │
  ↓                                    │
RelocScan (Phase 6) ←──────────────────┘
  ↓
FinalizeSynthetics (Phase 7)
  ↓
Emit (Phase 8) ── parallel section copy + reloc apply
  ↓
CacheUpdate (Phase 9)

──── Incremental fast path ────

Args → CacheValidation → Diff → RedGreenColor → IncrementalPatch → CacheUpdate
```

**Phases 1, 6, 8 are parallel.** Phases 2 (archive fixpoint), 5 (address assignment) are serial but fast ($O(n)$ or $O(|\mathcal{S}|)$).

---

## 11. Error Handling Contract

All public API functions return `Result<T, E>` where `E` is the crate-specific error type:

| Crate | Error type | Key variants |
|-------|-----------|--------------|
| `peony-object` | `ObjectError` | `Io`, `Parse`, `UnsupportedArch` |
| `peony-symbols` | `SymbolError` | `DuplicateSymbol` |
| `peony-layout` | `LayoutError` | `BadAlignment`, `AddressOverflow` |
| `peony-reloc` | `RelocError` | `UndefinedSymbol`, `Overflow`, `UnknownType` |
| `peony-cache` | `CacheError` | `Io`, `VersionMismatch`, `Decode` |
| `peony-emit` | `EmitError` | `Io`, `Reloc`, `TooLarge` |
| `peony` | `anyhow::Error` | wraps all of the above with context |

**Unhandled relocation types**: log at WARN level with `tracing::warn!` and skip (no `panic!`). This ensures that unknown relocation types in real-world objects degrade gracefully.

**Cache errors**: always fall back to full relink. A corrupted cache must never cause a link failure.

---

## 12. CLI Contract

Minimum subset for drop-in `ld` compatibility (to be usable via `cc -B`):

```
peony [OPTIONS] <inputs>...

Required:
  <inputs>...             Input .o, .a, .so files

Options:
  -o <file>               Output file (default: a.out)
  -e <sym>                Entry symbol (default: _start)
  -L <dir>                Add library search path
  -l <name>               Link library -l<name> → lib<name>.a / lib<name>.so
  -s, --strip-all         Strip .symtab and .strtab
  --pie                   Produce position-independent executable (ET_DYN)
  --gc-sections           Remove unreferenced sections
  --build-id              Emit .note.gnu.build-id (default: true)
  --incremental           Enable incremental caching
  --threads <N>           Rayon thread count (0 = all cores)
  --defsym SYM=VALUE      Define symbol with absolute value

Flags accepted and silently ignored (cc compatibility):
  -z <arg>                Various GNU ld -z flags
  -m <arg>                Target emulation
  --dynamic-linker <path>
  --hash-style <style>
  -plugin <path>
  --as-needed, --no-as-needed
  --eh-frame-hdr
  -soname <name>
  --rpath <path>
```

---

*This specification is derived from QUAD.md. Every data structure invariant maps to a theorem; every phase maps to an algorithm. Correctness arguments flow from the mathematical proofs in QUAD.md through to the oracle tests in §8.*
