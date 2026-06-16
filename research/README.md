# peony — research library

All papers, specs, and primary sources relevant to building an incremental
parallel ELF linker targeting Rust builds.

---

## Generated analyses (start here)

These three documents were synthesized from the corpus below + codegraph-mining
of the indexed reference linkers (mold, vendored under `mold/`; lld, indexed at
`~/CLionProjects/forks/llvm-project/lld`). Read them in this order:

| Doc | What it answers |
|---|---|
| `GAP_ANALYSIS.md` | **What is missing** — verified per-crate gap analysis of peony today (compiling skeleton; no loadable ELF; incremental ~0%); prioritized P0→P4 build order. |
| `REFERENCE_BLUEPRINT.md` | **How the real linkers do it** — for each gap, the lld vs mold *code pattern* (with file:line), which to follow (→ mold), the concrete Rust types to add, and a ranked reading list. |
| `SPEC_AND_LITERATURE_DIGEST.md` | **The authoritative values + literature** — ELF64 emission cheat-sheet (exact Ehdr/Phdr/Shdr field values), the full x86-64 relocation calculation table, TLS/PLT/GOT/`_start` ABI, the parallelism papers mapped to components, and the incremental playbook (Wild + rustc). |

> All PDFs below have been `pdftotext`-converted to `.txt` in this directory (the
> `.txt` is what the digest cites; 2-column papers in reading-order mode, specs
> with `-layout` to preserve tables).

---

## Academic papers (PDF)

| File | Reference | Why relevant |
|---|---|---|
| `smits2020-hybrid-incremental-compilers.pdf` | Smits, Konat, Visser — *Constructing Hybrid Incremental Compilers* (arXiv:2002.06183, 2020) | Core framework: dependency tracking, caching, cache invalidation for incremental builds |
| `maier2016-concurrent-hash-tables.pdf` | Maier, Sanders, Dementiev — *Concurrent Hash Tables: Fast and General?!* (arXiv:1601.04017, 2016) | Lock-free linear-probing hash table design — informs `peony-symbols` hot path |
| `tithi2022-optimal-parallel-bfs.pdf` | Tithi, Fogel, Chowdhury — *Optimal Level-Synchronous Parallel BFS* (arXiv:2209.08764, 2022) | Parallel GC mark-sweep algorithm (no locks, per-thread queues, level-sync) |
| `lyu2024-detecting-build-dependency-errors.pdf` | Lyu et al. — *Detecting Build Dependency Errors in Incremental Builds* (arXiv:2404.13295, 2024) | Inferring which outputs are stale without a full rebuild — maps to peony's diff phase |
| `huang2020-taskflow-parallel-task-graph.pdf` | Huang et al. — *Taskflow: Lightweight Parallel and Heterogeneous Task Graph* (arXiv:2004.10908, 2020) | Task-graph parallelism model; informs pipeline stage scheduling |

---

## ELF / ABI specifications (PDF)

| File | Reference |
|---|---|
| `x86-64-sysv-abi.pdf` | AMD64 SysV ABI v0.99 — authoritative x86-64 relocation type table (S, A, P, G, L formulas) |
| `elf-spec-tis.pdf` | TIS ELF-1.2 specification — original ELF format reference |
| `elf-64-spec.pdf` | ELF-64 Object File Format (SCO/UCL) — 64-bit extensions |

---

## mold linker (Rui Ueyama)

| File | Notes |
|---|---|
| `mold-design.md` | Official mold design document — parallel algorithm, preloading, string interning, Merkle build-id, ICF |

---

## Wild linker (David Lattimore)

| File | Notes |
|---|---|
| `wild-incremental-design.html` | **Primary source** — Wild's full incremental linking design: object diffing, persistent state files, relocation reverse index, red-green invalidation |
| `wild-speeding-up-edit-build-run.html` | Feb 2024 — motivation: target <10 ms rebuild for Rust |
| `wild-march-update-2024.html` | March 2024 — early Wild status |
| `wild-speeding-up-rustc-lazy.html` | June 2024 — lazy evaluation tricks in rustc/Wild interaction |
| `wild-testing-a-linker.html` | July 2024 — test strategy for a linker (differential testing vs GNU ld) |
| `wild-dylib-rabbit-holes.html` | Aug 2024 — dynamic library edge cases |
| `wild-update-0.6.0.html` | Sep 2025 — benchmark: Wild 1.47s vs mold 2.75s vs lld 4.04s (clang debug) |
| `wild-performance-tricks.html` | Sep 2025 — low-level performance techniques used in Wild |
| `wild-graph-algorithms-rayon.html` | Nov 2025 — graph algorithms in rayon (parallel GC, ICF) |
| `wild-update-0.9.0.html` | May 2026 — latest update |

---

## MaskRay / lld analysis

| File | Notes |
|---|---|
| `maskray-why-isnt-lld-faster.html` | **Primary source** — 9-pass linker model, lld vs mold analysis, bottlenecks |
| `maskray-recent-lld-improvements.html` | 2026 — parallel GC, parallel input loading; benchmark table (lld/mold/wild) |
| `maskray-implement-elf-linker.md` | Gist — step-by-step guide to implementing an ELF linker |
| `maskray-lld-gnu-incompatibilities.html` | lld vs GNU ld differences (compatibility swamp) |
| `maskray-relocatable-linking.html` | `-r` partial linking internals |
| `maskray-all-about-plt.html` | PLT/GOT internals, GOTPCREL, PLT32 relocation formulas |
| `maskray-stack-unwinding-eh-frame.html` | `.eh_frame` / `.eh_frame_hdr` format — needed for incremental FDE updates |
| `maskray-relocation-generation-assemblers.html` | How assemblers generate relocations — upstream of the linker |

---

## Rust project sources

| File | Notes |
|---|---|
| `rust-blog-enabling-rust-lld-2024.html` | May 2024 — rust-lld enabled on nightly; 7× link speedup, 40% end-to-end improvement for ripgrep debug |
| `rust-blog-lld-1.90-stable.html` | Sep 2025 — rust-lld on 1.90 stable |
| `rustc-dev-guide-parallel-rustc.html` | Codegen units, parallel compilation architecture |
| `rustc-dev-guide-incremental.html` | rustc red-green dep-graph, query system, fingerprinting |
| `odht-readme.md` | `odht` crate — on-disk hash table used by rustc incremental; planned for `peony-cache` symbol name map |

---

## Key numbers to remember

| Metric | Value | Source |
|---|---|---|
| lld link speedup over GNU ld (ripgrep debug) | 7× | Rust blog 2024 |
| End-to-end improvement | 40% | Rust blog 2024 |
| Wild vs lld (clang debug, 8 threads) | 2.5–2.8× faster | maskray 2026 |
| Wild vs mold (clang debug) | ~1.9× faster | wild 0.6.0 |
| mold Chrome input: object files | 30,723 | mold design.md |
| mold Chrome input: relocations | 62,024,719 | mold design.md |
| RDR rebuild speedup (David Barsky) | 4s → 0.81s | Zulip t-compiler/workspace-rebuild-perf |
