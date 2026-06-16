<!-- Generated 2026-06-07 by digesting the research/ corpus (ELF + x86-64 ABI specs, parallelism + incremental papers, mold/MaskRay/Wild/rustc docs). Companion to REFERENCE_BLUEPRINT.md (lld/mold code patterns) and GAP_ANALYSIS.md (verified gaps). All PDFs were pdftotext-converted to .txt in this dir. -->

# peony — Authoritative Spec Values & Literature Reference

## 1. How to use this doc
This is the **values + literature** companion to `REFERENCE_BLUEPRINT.md` (which holds the lld/mold *code-shape* patterns): everything here is a normative constant, an exact relocation formula, an ABI contract, or a design algorithm from the research corpus — the things code-mining cannot give you. When the blueprint says "build the GOT like mold does," this doc tells you the exact byte values, slot reservations, and reloc arithmetic to put in it; cross-reference `GAP_ANALYSIS.md` for the verified per-crate gap each item closes.

---

## §A — ELF64 emission cheat-sheet (P0)
Closes GAP P0-1/P0-2/P0-3 (emit writes no Ehdr/Phdr/Shdr; first section hardcoded at file offset 0x1000). Sources: `elf-64-spec.txt`, `elf-spec-tis.txt`, `x86-64-sysv-abi.txt`.

### Elf64_Ehdr — 64 bytes total, at file offset 0
| field | offset | width | value for static x86-64 exec |
|---|---|---|---|
| `e_ident[EI_MAG0..3]` | 0 | 4 B | `0x7f 'E' 'L' 'F'` = `7f 45 4c 46` |
| `e_ident[EI_CLASS]` (idx 4) | 4 | 1 B | `ELFCLASS64 = 2` |
| `e_ident[EI_DATA]` (idx 5) | 5 | 1 B | `ELFDATA2LSB = 1` |
| `e_ident[EI_VERSION]` (idx 6) | 6 | 1 B | `EV_CURRENT = 1` |
| `e_ident[EI_OSABI]` (idx 7) | 7 | 1 B | `ELFOSABI_SYSV = 0` |
| `e_ident[EI_ABIVERSION]` (idx 8) | 8 | 1 B | `0` |
| `e_ident[EI_PAD..15]` (idx 9) | 9 | 7 B | `0x00` ×7 (`EI_NIDENT = 16`) |
| `e_type` | 16 | u16 LE | `ET_EXEC = 2` (static) or `ET_DYN = 3` (all-PIC/PIE) |
| `e_machine` | 18 | u16 LE | `EM_X86_64 = 62 (0x3e)` |
| `e_version` | 20 | u32 LE | `1` |
| `e_entry` | 24 | u64 LE | VA of `_start` |
| `e_phoff` | 32 | u64 LE | `64` (phdrs immediately follow ehdr) |
| `e_shoff` | 40 | u64 LE | file offset of shdr table |
| `e_flags` | 48 | u32 LE | `0` |
| `e_ehsize` | 52 | u16 LE | `64` (FIXED — hardcode) |
| `e_phentsize` | 54 | u16 LE | `56` (FIXED) |
| `e_phnum` | 56 | u16 LE | count of phdrs |
| `e_shentsize` | 58 | u16 LE | `64` (FIXED) |
| `e_shnum` | 60 | u16 LE | count of shdrs (incl. null entry 0) |
| `e_shstrndx` | 62 | u16 LE | index of `.shstrtab` shdr |

`ET_*`: NONE=0, REL=1, EXEC=2, DYN=3, CORE=4, LOOS=0xFE00, HIOS=0xFEFF, LOPROC=0xFF00, HIPROC=0xFFFF.

### Elf64_Phdr — 56 bytes each, at `e_phoff`
| field | offset | width |
|---|---|---|
| `p_type` | 0 | u32 LE |
| `p_flags` | 4 | u32 LE |
| `p_offset` | 8 | u64 LE |
| `p_vaddr` | 16 | u64 LE |
| `p_paddr` | 24 | u64 LE (= p_vaddr) |
| `p_filesz` | 32 | u64 LE |
| `p_memsz` | 40 | u64 LE |
| `p_align` | 48 | u64 LE (page = 0x1000) |

`PT_*`: NULL=0, LOAD=1, DYNAMIC=2, INTERP=3, NOTE=4, SHLIB=5, PHDR=6, LOOS=0x60000000, HIOS=0x6FFFFFFF, LOPROC=0x70000000, HIPROC=0x7FFFFFFF. `PT_TLS = 7` (standard gABI — value not stated in corpus, verify in spec). `PT_GNU_EH_FRAME = 0x6474e550` (verify in spec).
`PF_*` flags (bitwise OR): `PF_X=0x1`, `PF_W=0x2`, `PF_R=0x4`. Map section flags → segment: ALLOC→R, WRITE→W, EXECINSTR→X.

**Congruence rule (kernel-enforced; violation → SIGSEGV on mmap):** `(p_vaddr % p_align) == (p_offset % p_align)`, `p_align` a power of 2. Practically: every section in a PT_LOAD shares one `load_base = VA − file_offset`. `.bss` (NOBITS): `p_filesz` excludes it, `p_memsz` includes it. Image base: GNU ld uses `0x400000`, lld uses `0x200000` — **pick one and document** (GAP/maskray).

### Elf64_Shdr — 64 bytes each, at `e_shoff`; index 0 MUST be all-zero
| field | offset | width |
|---|---|---|
| `sh_name` | 0 | u32 LE (offset into .shstrtab) |
| `sh_type` | 4 | u32 LE |
| `sh_flags` | 8 | u64 LE |
| `sh_addr` | 16 | u64 LE (VA) |
| `sh_offset` | 24 | u64 LE |
| `sh_size` | 32 | u64 LE |
| `sh_link` | 40 | u32 LE |
| `sh_info` | 44 | u32 LE |
| `sh_addralign` | 48 | u64 LE |
| `sh_entsize` | 56 | u64 LE |

`SHT_*`: NULL=0, PROGBITS=1, SYMTAB=2, STRTAB=3, RELA=4, HASH=5, DYNAMIC=6, NOTE=7, NOBITS=8, REL=9, SHLIB=10, DYNSYM=11, LOOS=0x60000000, HIOS=0x6FFFFFFF, LOPROC=0x70000000, HIPROC=0x7FFFFFFF, `X86_64_UNWIND=0x70000001`.
`SHF_*` (OR): `WRITE=0x1`, `ALLOC=0x2`, `EXECINSTR=0x4`, MASKOS=0x0F000000, MASKPROC=0xF0000000, `X86_64_LARGE=0x10000000`.
**SectionKind → (sh_type, sh_flags):** Text→(PROGBITS, ALLOC|EXECINSTR=0x6); RoData→(PROGBITS, ALLOC=0x2); Data→(PROGBITS, ALLOC|WRITE=0x3); Bss→(NOBITS, ALLOC|WRITE=0x3).
**sh_link/sh_info:** SYMTAB/DYNSYM → `sh_link`=strtab idx, `sh_info`=index of first non-local symbol (= local count); RELA/REL → `sh_link`=symtab idx, `sh_info`=section being relocated; DYNAMIC/HASH → `sh_link`=strtab/symtab, `sh_info`=0.

### Special section indices (SHN_*)
`UNDEF=0`, `LORESERVE=LOPROC=0xff00`, `HIPROC=0xff1f`, `LOOS=0xff20`, `HIOS=0xff3f`, `ABS=0xfff1`, `COMMON=0xfff2`, `HIRESERVE=0xffff`. For `SHN_COMMON`: `st_value`=alignment, `st_size`=byte size. Reserved indices (0xff00–0xffff) must **not** index the shdr table.

**Layout prerequisites (GAP P0-3/P0-4):** reserve 64 B for Ehdr at 0, then `e_phnum×56` for phdrs at 64; first section offset follows; build PT_LOAD groups by RWX (currently only `segment_order()` sort exists, no grouping).

---

## §B — Symbol & relocation on-disk layouts
Sources: `elf-64-spec.txt`, `elf-spec-tis.txt`.

### Elf64_Sym — 24 bytes; index 0 (STN_UNDEF) all-zero; locals precede globals/weaks
| field | offset | width |
|---|---|---|
| `st_name` | 0 | u32 LE (strtab offset; 0 = no name) |
| `st_info` | 4 | u8 (bind<<4 \| type) |
| `st_other` | 5 | u8 (visibility in low 2 bits) |
| `st_shndx` | 6 | u16 LE |
| `st_value` | 8 | u64 LE (sec-relative in REL; VA in EXEC) |
| `st_size` | 16 | u64 LE |

`st_info` macros: `ST_BIND(i)=i>>4`, `ST_TYPE(i)=i&0xf`, `ST_INFO(b,t)=(b<<4)+(t&0xf)`.
**STB_*:** LOCAL=0, GLOBAL=1, WEAK=2, LOOS=10, HIOS=12, LOPROC=13, HIPROC=15.
**STT_*:** NOTYPE=0, OBJECT=1, FUNC=2, SECTION=3, FILE=4, `TLS=6`, `GNU_IFUNC=10` (=STT_LOOS), LOOS=10, HIOS=12, LOPROC=13, HIPROC=15.
**STV_* (visibility, st_other bits):** DEFAULT/LOCAL=0, INTERNAL/GLOBAL=1, WEAK/HIDDEN=2 (corpus labels these STV_LOCAL/GLOBAL/WEAK; the operational pair you need is DEFAULT=0 vs PROTECTED — see §I — verify PROTECTED numeric in spec). `STT_FILE` → STB_LOCAL + `st_shndx=SHN_ABS`, precedes other locals.

### Elf64_Rela — 24 bytes (x86-64 uses RELA **only**, never REL)
| field | offset | width |
|---|---|---|
| `r_offset` | 0 | u64 LE (sec offset, or VA in exec) |
| `r_info` | 8 | u64 LE |
| `r_addend` | 16 | i64 LE |

`r_info` macros (note 64-bit split, opposite of ELF32): `ELF64_R_SYM(i)=i>>32`, `ELF64_R_TYPE(i)=i & 0xffffffff`, `ELF64_R_INFO(s,t)=(s<<32)+(t & 0xffffffff)`. Elf64_Rel (16 B) exists but is **not** used on x86-64; addend always comes from `r_addend`, never the patch bytes.

**ELF hash (for .hash, dynamic linking):**
```
h=0; for c in name { h=(h<<4)+c; g=h&0xf0000000; if g!=0 { h^=g>>24; } h&=0x0fffffff; } return h
```
Layout: `nbucket(u32), nchain(u32), bucket[nbucket], chain[nchain]`; `nchain`=symbol count; probe `bucket[h%nbucket]`, follow `chain[]` until `STN_UNDEF(0)`.

---

## §C — x86-64 relocation calculation table (P1)
Variables (`x86-64-sysv-abi.txt §4.4.1`): **A**=`r_addend`; **S**=symbol VA; **P**=place (`r_offset`, becomes VA at load); **B**=load base (0 for ET_EXEC); **G**=GOT slot offset (index×8); **GOT**=GOT section base VA; **L**=PLT entry VA; **Z**=symbol `st_size`. PC-relative reference point is **end of field** (x86 RIP is post-fetch); offsets must fit signed 32-bit `[-2^31, 2^31-1]`.

Status per `GAP_ANALYSIS.md:36-37,88-93`: peony's `patch_buf` arms exist for the 8 marked **HAS**; `GOT32` is *scanned* but has **no patch arm**; all others **MISSING**.

| val | name | field | calculation | peony |
|---|---|---|---|---|
| 0 | NONE | none | — | n/a |
| 1 | **R_X86_64_64** | word64 | `S + A` | **HAS** |
| 2 | **PC32** | word32 | `S + A − P` | **HAS** (ovf-checked) |
| 3 | GOT32 | word32 | `G + A` | scanned, **no arm** |
| 4 | **PLT32** | word32 | `L + A − P` | **HAS** (A usually −4) |
| 5 | COPY | none | (dyn-linker only) | skip (static) |
| 6 | GLOB_DAT | word64 | `S` (dyn-linker writes) | linker *emits*, not patch |
| 7 | JUMP_SLOT | word64 | `S` (dyn-linker writes) | linker *emits*, not patch |
| 8 | RELATIVE | word64 | `B + A` | needed for PIE |
| 9 | **GOTPCREL** | word32 | `G + GOT + A − P` | **HAS** |
| 10 | **R_X86_64_32** | word32 | `S + A` (zero-extend, verify fits) | **HAS** |
| 11 | **R_X86_64_32S** | word32 | `S + A` (sign-extend, verify fits) | **HAS** |
| 12 | 16 | word16 | `S + A` | **non-conformant** — reject/warn |
| 13 | PC16 | word16 | `S + A − P` | **non-conformant** |
| 14 | 8 | word8 | `S + A` | **non-conformant** |
| 15 | PC8 | word8 | `S + A − P` | **non-conformant** |
| 16 | DTPMOD64 | word64 | TLS module index (dyn) | MISSING (TLS) |
| 17 | DTPOFF64 | word64 | offset in TLS block | MISSING (TLS) |
| 18 | TPOFF64 | word64 | offset from thread ptr | MISSING (TLS) |
| 19 | TLSGD | word32 | PC-rel → GOT pair (DTPMOD+DTPOFF) | MISSING (TLS) |
| 20 | TLSLD | word32 | PC-rel → DTPMOD GOT entry | MISSING (TLS) |
| 21 | DTPOFF32 | word32 | offset in TLS block | MISSING (TLS) |
| 22 | GOTTPOFF | word32 | PC-rel → GOT entry w/ TPOFF | MISSING (TLS/IE) |
| 23 | TPOFF32 | word32 | offset from thread ptr | MISSING (TLS/LE) |
| 24 | **PC64** | word64 | `S + A − P` | **HAS** |
| 25 | GOTOFF64 | word64 | `S + A − GOT` | MISSING |
| 26 | GOTPC32 | word32 | `GOT + A − P` | MISSING |
| 27 | GOT64 | word64 | `G + A` | MISSING (large) |
| 28 | GOTPCREL64 | word64 | `G + GOT − P + A` | MISSING (large) |
| 29 | GOTPC64 | word64 | `GOT − P + A` | MISSING (large) |
| 30 | GOTPLT64 | word64 | `G + A` | MISSING (large) |
| 31 | PLTOFF64 | word64 | `L − GOT + A` | MISSING (large) |
| 32 | SIZE32 | word32 | `Z + A` | MISSING (needs st_size) |
| 33 | SIZE64 | word64 | `Z + A` | MISSING |
| 34 | GOTPC32_TLSDESC | word32 | PC-rel → TLSDESC GOT pair | MISSING (TLS) |
| 35 | TLSDESC_CALL | none | relax marker (not patched) | MISSING (TLS) |
| 36 | TLSDESC | word64×2 | descriptor pair (fn ptr + arg) | MISSING (TLS) |
| 37 | IRELATIVE | word64 | `indirect(B + A)` (IFUNC) | MISSING |

`REX_GOTPCRELX` (**HAS** in peony) and `GOTPCRELX` are GNU/psABI relaxation extensions (values 42 and 41 respectively — **not in corpus, verify in spec**); they relax `mov sym@GOTPCREL(%rip),%reg` → `lea sym(%rip),%reg` for non-preemptible symbols (optional perf, not correctness — GAP:93).

**Two-phase GOT/PLT wiring (the central P1 break, GAP:65-70):** scan result is computed then *discarded*; emit builds a fresh empty `RelocScanResult::new()`; `got_address`/`plt_address` stay 0 → every PLT32/GOTPCREL computes against base 0. Fix: thread `slot_set: HashMap<SymbolIndex,(got_off,plt_off)>` through **scan → layout → emit → apply**; layout creates synthetic `.got`/`.plt` OutputSections sized from the scan and assigns slot offsets. **Symbol VA write-back** (GAP P1): after `compute_layout` assigns section VAs, run a finalize pass `final_va = section_va + symbol.value` into `SymbolTable` *before* `apply_reloc`. Ordering is strict: (1) scan, (2) layout, (3) finalize symbol VAs, (4) apply.

---

## §D — TLS, PLT/GOT, and process startup (P1)
Sources: `x86-64-sysv-abi.txt`, `maskray-all-about-plt.html`, `mold-design.md`.

### GOT / .got.plt
`.got`: SHT_PROGBITS, ALLOC|WRITE (0x3). `.plt`: SHT_PROGBITS, ALLOC|EXECINSTR (0x6). **Reserved GOT slots — never assign user symbols:** `GOT[0]=&_DYNAMIC`, `GOT[1]=link_map` (ld.so), `GOT[2]=resolver/_rtld_bind_start` (ld.so). **User symbol slots start at GOT[3]**, at byte offset `3*8 + N*8`. GOT offsets are 32-bit → max ~2^29 ≈ 536M entries.

### PLT stub format (small/medium model, exact bytes — maskray)
```
PLT0 (16 B): pushq  .got.plt+8(%rip)   ; [6B]
             jmpq  *.got.plt+16(%rip)  ; [6B]
             nop  (×4)                 ; pad to 16
PLT[n] (16 B): jmpq *.got.plt[n+3](%rip) ; [6B]
               pushq $n                  ; [6B]  (reloc index)
               jmp  PLT0                 ; [4B]
```
Each external call site: `call foo@plt` (5 B, `R_X86_64_PLT32`, addend −4). For each preemptible PLT symbol emit one `R_X86_64_JUMP_SLOT` in `.rela.plt`: `r_offset`=`.got.plt[n+3]` VA, `r_info`=JUMP_SLOT|symidx, `r_addend=0`. **Lazy binding:** ld.so resolves on first call; do **not** eagerly populate `.got.plt[n]` (loader's job) unless `-z now`. Large model PLT0 uses `%r15` (GOT ptr); PLT[n] is 21 B (`movabs $name@GOT,%r11; jmp *(%r11,%r15); pushq $idx; jmp PLT0`), switching to 27 B past entry 102261125.

### Four TLS models (psABI; all MISSING in peony, P1)
| model | reloc(s) | GOT entries | runtime sequence |
|---|---|---|---|
| **GD** (general dynamic) | TLSGD(19) → DTPMOD64(16)+DTPOFF64(17) | **2 adjacent** | `__tls_get_addr(GOT_pair)` |
| **LD** (local dynamic) | TLSLD(20) → DTPMOD64+module-offset | 2 adjacent | `__tls_get_addr` then add per-sym DTPOFF |
| **IE** (initial exec) | GOTTPOFF(22) → TPOFF64(18) | 1 | `mov sym@GOTTPOFF(%rip),%rax; add %fs:0,%rax` |
| **LE** (local exec) | TPOFF32(23)/TPOFF64(18) | 0 | immediate offset from `%fs:0`, no GOT |
| TLSDESC | GOTPC32_TLSDESC(34)+TLSDESC_CALL(35)→TLSDESC(36) | 2 (16 B) | lazy descriptor resolver |

Slot reservation in scan: TLSGD/TLSLD → **2×8 B consecutive**; GOTTPOFF/IE → 1×8 B; TLSDESC → 16 B. **Critical:** allocate the GD/LD *pair* together or the second value gets overwritten. TLS offsets (DTPOFF/TPOFF) are **not** symbol VAs — they come from `.tdata`/`.tbss` layout inside a `PT_TLS` segment; `PT_TLS p_memsz` = `.tdata + .tbss` size, `p_filesz` = `.tdata` size only. A symbol must use **one** model consistently across all CUs. Static-exec relaxations: IE→LE, GD→IE/LE when binding is local (verify exact rewrite sequences in spec).

### Process startup contract (`_start` / e_entry)
At `_start`, `%rsp` is **16-byte aligned** pointing at `argc` (u64); then `argv[0..argc]`, NULL, `envp[...]`, NULL, auxv `(a_type,a_val)` pairs ending `AT_NULL`. `%rbp` unspecified (set 0 to mark frame bottom); `%rdx` = atexit fn ptr (register if non-null); `DF=0`. Startup FPU state (Tables 3.3–3.5): x87 RC=0/PC=11/all-masked; MXCSR FZ=0/RC=0/all-masked/DAZ=0; rFLAGS all clear. Red zone = 128 B below `%rsp`. **Init/fini ordering:** `.init_array`/`.fini_array` are strictly ordered roots for GC; any change to ordered sections (`.init`,`.ctors`) forces full relayout (descope incremental for these — see §F).

---

## §E — Parallelism literature → peony
Maps the parallelism thesis (P3) onto concrete components.

### Concurrent hash table → `peony-symbols` symbol map (Maier 2016)
**Decision: lock-free linear probing with 2-word (128-bit) CAS.** Beats bucket-chaining/cuckoo/RCU by **7.7–64.5×** on multi-socket; finds hit **12.8×** @48 threads (reads parallelize perfectly, no atomics), inserts **9.6×**. Bucket-chaining is **non-negotiably out**. Concrete params:
- Cell = `{hash:u64, id:u32}` packed to 16 B; modify via 2-word CAS, **find = read-only scan** (no atomics).
- Capacity = `1 << (64 - (2*expected − 1).leading_zeros())` (power-of-2; map via `hash & (cap-1)`, never `%`).
- **Migrate at 60% load; double capacity; migration block = 4096 cells.** Clusters map to non-overlapping target ranges (Lemma 1) → lock-free parallel migration of disjoint blocks; one barrier at end.
- **Approx size via per-thread counters**, flush to global every `Θ(p)` (randomized `1..√p`) inserts; error bound `O(p²)` (fine when table ≫ p²). Never a single global atomic counter (serialization bottleneck).
- **Torn-read safety:** in `find`, read key → read value → re-read key == key; else retry. Updates CAS both. Memory: ~512 MB / 1M symbols at 37% fill; typical Rust binary 10k–1M symbols → preallocate `2×`.
- Skip Intel TSX initially (+28% only on non-growing, 0 during migration).

### Level-synchronous parallel BFS → GC mark phase (Tithi S3BFS)
**Decision: work-adaptive level-synchronous BFS, no locks/atomics**, for `--gc-sections` mark over the section→relocation graph (sections=vertices, relocs=edges). Bound `T_P = Ω((m+n)/P + D·log(min(P,W_l)))`. Per level: collect out-degrees → parallel-prefix-sum → spawn `P_l = min(P, W_l)` threads (**never more threads than work** — energy/cache win as incremental work shrinks) → explore into thread-local queues → **benign-race dedup** via non-atomic `Owner[v]` (no CAS) → prefix-sum linearize next frontier. Grain size `≤ log(n/P_l)`. Thread-local queues must be strictly isolated or the benign race becomes hazardous.

### Taskflow task-graph → pipeline scheduler (Huang 2020)
**Decision: work-stealing with per-domain queues + condition tasks** for the `link_batch → relocate → write` pipeline. `MAX_STEALS = 10 × num_workers`; wasteful-steal bound `O(MAX_STEALS·(|W| + |D|·E/es))`. Keep task structs **<300 B** (Taskflow 272 B, ~61 ns create + 54 ns/dep on 40-core). Separate `link_workers` from `reloc_workers` (one CTQ+GTQ pair per worker per domain) to kill false sharing; specialize reloc queues by type (PC32-near vs GLOB_DAT-far) so per-type arithmetic stays cache-hot. **Two-phase-commit sleep** (`prepare_wait(pred)` → recheck → `commit_wait`) so no task is lost during sparse incremental phases. **Condition task** after mark returns 0 (Δsections<threshold → fast reloc path) or 1 (full path) — branch without re-entering the executor; **the branch predicate must be deterministic** (same Δ → same path) for reproducible incremental state. Graph must stay a DAG (verify relocations never create new reachability paths, else work-stealing can deadlock).

---

## §F — The incremental playbook (P2 — the thesis)
peony's whole reason to exist; currently ~0% (GAP §5). Synthesizes Wild (`wild-incremental-design.html`), rustc red-green (`rustc-dev-guide-incremental.html`), ODHT (`odht-readme.md`), Smits, Lyu.

### The two/three-mode algorithm (Wild)
1. **non-incremental** — status quo full link.
2. **initial-incremental** — full link, but **over-allocate** each output section by a growth factor (`--incremental-space=10%`), then persist state.
3. **incremental-update** — diff → allocate new addresses in slack → update symbol resolutions → patch relocations (via reverse index) → update dynamic relocs + FDEs → rebuild `.eh_frame_hdr` → persist. Fall back to initial-incremental on slack exhaustion / version mismatch / any error.

### Object diffing (Wild)
Check `.o` mtimes first. **`.rlib` archives carry no rustc timestamps → byte-compare archive members.** Match code sections by mangled symbol name; match anonymous data (`.rodata..L__unnamed_75`) by **referrer-set** (which named sections reference it). Output = changed/added/removed section list. EChecker (Lyu) insight: build deps change *only* via tracked inputs — analog: if a section's relocation set + symbol-reference set is unchanged, it is stable; but also hash **linker-script/version-script/variant flags** into the fingerprint (their false positives all came from untracked inputs like symlinks).

### `peony-cache` on-disk structures (in `[output].incr/`, all mmap'd; rebuild on full link)
- `index.json` — input files, args, **peony version** (mismatch → discard, full relink).
- **symbol name→ID map** — start `sled`; long-term **ODHT** (open-addressing, external key storage, endianness/alignment-independent, zero deserialization, mmap, no rehash on incremental — exactly rustc's substrate). Keep an in-memory `FxHashMap` for the non-incremental path to avoid slowdown.
- **symbol resolution table** — mmapped `Vec` of `(binding, VA, section_index)`; allocate **contiguous ID ranges per input object** so `split_off_mut` gives each thread an exclusive slice (zero-copy parallel resolution; convert `Vec<T>↔Vec<AtomicT>` in place via `collect`/`into_inner`).
- **relocation reverse index** — two flat files: `head[symbol_id] = first_reloc_index`, `next[reloc_index] = next_reloc_for_same_symbol` (sentinel `0xFFFFFFFF`). Built in parallel via **atomic compare-exchange on `head[]`** (avoids a `Vec` per symbol). On symbol move, walk the list and patch reloc bytes.
- **dynamic-reloc index** — per input section `(first_dyn_reloc_index:u32, count:u32)` (GLOB_DAT/JUMP_SLOT adjacency) → efficient removal.
- **FDE index** — per input section `(first_fde_index:u32, fde_count:u32)`; section change → look up + rebuild `.eh_frame_hdr` (§G).
- **string-merge index** — `string → output_address` mmap'd hash table per output SHF_MERGE section.

### Red-green / try-mark-green (rustc, Salsa) → linker phases
Model each phase as a query: `read_object_symbols → resolve_extern_refs → assign_section_vas → compute_relocations → emit_elf`. Store `(phase, input_fingerprint, output_fingerprint)`. On rerun: recompute input fingerprint; all-green → return cache, no exec. Any red → re-exec, hash output; **output unchanged ⇒ recover green** (e.g., symbol reordered but VA stable, skip downstream GOT/PLT recompute). **Must visit `reads(Q)` in original execution order** — out-of-order replay re-executes a query under the wrong control-flow branch (ICE/stale cache). Use **SHA-256** for fingerprints (large reloc streams → collision risk with FNV/CRC). Weak-symbol redefinition: track `weak_name → [defining_lib_in_order]`; order change invalidates all users (batch per-library, not per-symbol, to avoid invalidation explosions).

### Targets (Smits/Lyu) & what Wild descoped
Incremental ≈ **5–15% of full-link time** when <30% changed (Smits: recompile ~10% of scratch; Lyu: 75–95% saved, up to 85× detection speedup; ≤1.4% false-positive tolerance acceptable if from user input not algorithm). **Wild explicitly descopes:** compiler-supplied diffs (future — object diffing carries overhead until rustc emits diffs directly); strictly-ordered sections (`.init`/`.ctors` → full relayout + diagnostic). **mold rejects incremental linking entirely** — this is peony's differentiator. State is fragile: use atomic file ops + checksums; any partial/interrupted write corrupts it.

---

## §G — eh_frame / .eh_frame_hdr
Sources: `maskray-stack-unwinding-eh-frame.html`, `wild-incremental-design.html`.

**Why it matters:** unwinders (panics, C++ EH, backtraces) binary-search `.eh_frame_hdr` to find the FDE for a PC; missing/unsorted → broken unwinding. It is also a major bottleneck in large debug builds — handle it in incremental mode via the FDE index (§F).

**CIE:** `u32 length, u32 CIE_id(=0), u8 version, augmentation string ('z'⇒aug data present), ULEB128 code_align, ULEB128 data_align, SLEB128 return_addr_column, [aug data], CFI`. **FDE:** `u32 length, u32 CIE_pointer (back-ref), initial_location, address_range, [aug data], CFI` (loc/range width per CIE augmentation). `GNU_ARGS_SIZE` DWARF op = 0x2e (uleb128 arg) for EH after unwinding.

**.eh_frame_hdr** (`PT_GNU_EH_FRAME` segment): `u8 version(=1), u8 eh_frame_ptr_enc, u8 fde_count_enc, u8 table_enc, i32 eh_frame_ptr (PC-rel), u32 fde_count, then fde_count × [i32 initial_location, i32 fde_address]`. Encodings (numeric values — **verify in spec**): `eh_frame_ptr_enc = DW_EH_PE_pcrel | DW_EH_PE_sdata4`; `fde_count_enc = DW_EH_PE_udata4`; `table_enc = DW_EH_PE_datarel | DW_EH_PE_sdata4` (relative to `.eh_frame_hdr` start). **Entries MUST be sorted by `initial_location`** or binary search breaks. Construction: collect all input `.eh_frame`, **deduplicate CIEs**, extract FDE `initial_location`s, sort, build index. GNU ld supports only 32-bit offsets; lld 23+ adds `sdata8` for huge binaries.

---

## §H — Testing strategy
Source: `wild-testing-a-linker.html`.

**Differential testing (core method):** link each test with peony, **GNU ld, lld** (and mold). Accept a property (instruction bytes, header fields, symbols) if peony matches **any** reference — GNU ld is sometimes suboptimal (e.g., wrong relaxation when a symbol is referenced by a `.so`). Build a `linker-diff` tool: disassemble functions, **normalize symbol names in relocations**, trace each reloc back to its input. Add `peony --write-layout` so trace logs correlate to output addresses for diagnosis. Inline test metadata as comments (`//#Object:exit.c`, `//#CompArgs:...`, `//#ExpectSym:_start .text`, `//#EnableLinker:lld`, `//#DiffIgnore:...`); cache compiled artifacts + reference-linker outputs keyed on source+args hash; run tests via `par_iter`. Support `PEONY_REFERENCE_LINKER=ld` to diff against a reference on real crates ad-hoc.

**Test ladder (GAP §test plan):** (1) emit valid Ehdr/Phdr/Shdr — `readelf -h/-l/-S` clean; (2) single-`.o` `_start` that exits — actually runs; (3) data section + R_X86_64_64/PC32 — correct value; (4) **cross-object link**: two `.o`s with a PLT32 call + GOTPCREL data ref across the boundary — executes with correct result (validates VA write-back + GOT/PLT wiring); (5) TLS thread_local; (6) incremental relink after one-file change matches full-link output bitwise (mod intended layout differences).

---

## §I — Pitfalls & GNU-compat traps (consolidated)
- **Page congruence:** `(p_vaddr % p_align)==(p_offset % p_align)` or SIGSEGV on mmap. `.bss` NOBITS → in PT_LOAD with `p_filesz=0, p_memsz=size`.
- **PC-relative reference = end of field** (P + width), not start; addend always from `r_addend`, never patch bytes; verify `S+A−P` fits signed i32.
- **Ordering is law:** scan → layout → finalize symbol VAs → apply relocs. Symbol VA = section VA + value, but section VA needs total symbol count — circular, so finalize *after* layout.
- **GOT[0..2] reserved**; user slots from GOT[3] (`3*8 + N*8`). Scan/emit slot assignment must be byte-identical or relocs point at wrong data.
- **GD/LD TLS needs 2 adjacent GOT entries**; allocate the pair atomically. TLS offsets ≠ symbol VAs (come from `.tdata`/`.tbss` + PT_TLS).
- **JUMP_SLOT addend = 0**; do not eagerly populate `.got.plt[n]` (ld.so's job) unless `-z now`.
- **Undefined-symbol check before emit** (GAP P1): placeholders make `lookup` return `Some(undefined)`, so the `UndefinedSymbol` branch in `apply_reloc` is dead — add an explicit pre-emit pass erroring on `defined_in.is_none()`.
- **Non-conformant relocs** (8/16/PC8/PC16) — reject or warn; indicate malformed compiler output.
- **SHF_COMPRESSED `.debug_*`** (GAP): copying compressed bytes then relocating at uncompressed offsets corrupts; decompress first or skip `.debug` (common in static linkers).
- **COMDAT/common symbols** unimplemented → duplicate strong symbols hard-error today; C++ headers / tentative C defs break. Keep first per `.group`; keep/discard whole group together.
- **Weak symbols:** strong overrides weak; unresolved weak ⇒ S=0 — must warn, not silently link. Weak in `.o` + strong in `.so` ⇒ resolves to `.so` (preemptible).
- **Determinism:** sorted containers / stable iteration everywhere (BTreeMap, sorted Vec) for reproducible builds and clean diffs.
- **GNU-compat traps:** image base GNU ld `0x400000` vs lld `0x200000` — pick & document. lld is **not** bug-for-bug GNU ld; ~20 crater crates needed `-Wl,-z,nostart-stop-gc`. **Copy relocations are ABI-breaking** (freeze size into ABI, force startup copy of large RO data) — avoid; use GOT-indirection. **Symbol visibility:** 300k default-visibility symbols → 150 ms load vs **protected → 5 ms** (GLOB_DAT cost); emit **protected** for Rust-mangled internals (binutils ≥2.40 allows direct relocs to protected, forbids copy relocs against them; <2.40 rejects → recommend lld). Respect rustc's version script (`global:{...}; local:*;`); debug builds export 300k+ symbols via `-Z share-generics`. `--gc-sections` roots: `_start`, `__init_array_start`, KEEP-marked, SHF_LINK_ORDER edges; mark-sweep must handle COMDAT cycles. ICF: fold only if content hash **and** relocation-target isomorphism match (bad ICF = silent alias bug).

---

## §J — Key numbers
**Linker performance (Wild/Rust blog):** lld on stable cuts linking **7×**, **40%** end-to-end (ripgrep). Full link of ripgrep-like debug binary: lld ~500 ms, mold ~290 ms, Wild ~180 ms → peony target **<400 ms**; incremental relink after one file → **<50 ms** (Wild ~20–50 ms). Wild links itself **48% faster than mold** (less so with heavy debug info). Symbol visibility: 300k symbols → **150 ms** (default) vs **5 ms** (protected).
**Concurrent hash table (Maier):** linear probing **7.7–64.5×** > chaining/cuckoo; find **12.8×**, insert **9.6×** @48 threads; migrate at **60%**, growth **2×**, block **4096**; ~**512 MB/1M** symbols @37% fill; TSX **+28%** (non-growing only).
**Parallel symbol resolution (mold):** 1 thread 1.3× *slower* than lld (atomics overhead), 8 threads **5.3×** faster, 16 threads **5.4×** (saturation).
**Build-id (mold):** SHA-1 ~**2 GiB/s** with x86 SHA insns; 2 GiB binary on 16 cores in **60–70 ms** (1 MiB chunks, Merkle). Overwrite-output trick saves **~300 ms** on a 2 GiB file.
**ICF (mold):** **5×** over lld's pessimistic algo (Chromium 5 s → 1 s).
**Incremental (Smits/Lyu):** recompile ~**10%** of scratch (0–9 s vs 29–50 s); EChecker saves **75–95%** of clean build (median 78.9% small, 95.1% large), **85×** detection speedup, F1 0.995 (FP 1.4%).
**Task overhead (Taskflow):** struct 272 B, **61 ns** create + **54 ns**/dep on 40-core; `MAX_STEALS = 10×workers`.
**Fixed ELF sizes:** Ehdr **64**, Phdr **56**, Shdr **64**, Sym **24**, Rela **24**, Rel **16**, Dyn **16**, PLT entry **16** (small/med), GOT max ~**2^29** entries.
