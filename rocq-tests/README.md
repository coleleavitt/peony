# peony — Rocq/Coq Formalization

Machine-checked proofs of the theorems in [`../research/QUAD.md`](../research/QUAD.md),
plus two **novel** results for byte-level incremental linking that a prior-art
survey (see `../research/`) found absent from the published literature.

## Build

```sh
make            # compile every proof; non-zero exit on any unproved goal
make clean
```

Requires the **Rocq Prover ≥ 9.0** (or Coq ≥ 8.18); developed against Rocq 9.1.0.
`coqc` exits non-zero on any incomplete proof, so a green `make` *is* the
correctness certificate. There are **no `Admitted`, `admit`, or custom `Axiom`s**
anywhere; the only axiom used is standard **functional extensionality** (for the
`addr → byte` memory model), confirmed via `Print Assumptions`. The
permutation-invariance / determinism results for symbol resolution are *closed
under the global context* (zero axioms).

## Files and what they prove

| File | QUAD.md § | Headline results |
|---|---|---|
| `SymbolLattice.v` | §2 | `sjoin_assoc`, `sjoin_comm`, idempotence (non-strong); `resolve_perm_invariant` / `parallel_resolution_deterministic` (order-independent merge); `pick_assoc` + `pick_is_input` (argmax resolution under provenance tie-break) |
| `RelocDisjoint.v` | §5 | `apply_comm` (disjoint relocations commute — the frame property); `apply_all_perm_invariant` / `parallel_reloc_deterministic` (parallel apply = sequential) |
| `RelocMonoid.v` | §5 (new framing) | **NOVEL** — relocation patches form a *partial commutative monoid*; `act_oplus` (action is a homomorphism), `act_comm`, `reloc_pcm_summary` |
| `SectionGC.v` | §3 | `gc_sound` (every GC-live section is reachable), `gc_contains_roots`, `bfs_superset_*` (level-synchronous BFS = reachability) |
| `Layout.v` | §4 | `layout_all_aligned` (page alignment), `layout_page_congruent` (vaddr ≡ fileoff mod page), `layout_addr_lower_bound` + `layout_no_overlap` (disjoint windows) |
| `IncrementalSoundness.v` | §6, §10.2 | **NOVEL** — `incremental_relink_sound` family + `green_is_byte_stable`, `red_is_forced`, `minimal_cut_characterization` (byte-level incremental soundness + minimal recomputation cut) |
| `IncrementalCostBound.v` | §6 (new) | **NOVEL** — `incremental_beats_fromscratch`: a single-section edit of an n-section link costs incremental-work 1 vs from-scratch n (`single_edit_cost_is_one`, `fromscratch_grows_unboundedly`); `capacity_stable_cost_bounded` (incremental ≤ from-scratch always). The Ω(n) edit-rebuild separation from mold/lld. |
| `ParallelSchedule.v` | §7 (new) | **NOVEL framing** — work–span (Brent) bounds for the disjoint-footprint section-copy antichain: `span_lower_bound`, `work_lower_bound` (no schedule beats max(T∞, T₁/P)), `greedy_within_2x_opt` (work-stealing is a 2-approximation), `linear_speedup_regime` |
| `ICFSoundness.v` | §9 (new) | **NOVEL** — Identical Code Folding soundness as bisimulation: `icf_call_preserved`, `icf_addr_change_iff_folded` (the address-significance hazard, characterised), `icf_observationally_equivalent` (sound iff no folded fn is address-significant), `icf_rel_refines_content` (Hopcroft partition refinement is a valid quotient) |

## The novel theorems

The deep-research prior-art survey (`../research/`, papers downloaded and
`pdftotext`-extracted under `../research/txt/`) confirmed that while the
*general* meta-theory of incremental computation exists —
Acar/Blume/Donham's *from-scratch consistency* (`acar2011-…`), Cai et al.'s
*incremental λ-calculus* derivative law `f(a ⊕ da) = f(a) ⊕ D⟦f⟧ a da`
(`cai2013-…`), Mokhov et al.'s build-system *minimality* (`mokhov2018-…`) — none
of it is specialized to **byte-level linking / relocation**. CompCert-line work
(Kang et al. POPL'16) verifies *separate compilation* at program-semantics level,
explicitly leaving the byte-level link step in the trusted base.

1. **Incremental-Relink Soundness** (`IncrementalSoundness.v`, Theorem A).
   Under the `capacity_stable` invariant (same identity/offset/capacity window,
   content still fits), the in-place patched image is **byte-identical** to a
   full deterministic relink. This is the linker analogue of from-scratch
   consistency, proved at output-byte granularity.

2. **Minimal Recomputation Cut** (`IncrementalSoundness.v`, Theorem B =
   `minimal_cut_characterization`). The red/green partition by content-change is
   the *unique minimal correct cut*: every green region is byte-stable
   (`green_is_byte_stable`, so it is sound to skip) and every red region is
   *forced* (`red_is_forced` exhibits a witnessing base memory at which the
   rendered byte differs). This is Mokhov-style minimality lifted from file
   granularity to byte granularity for a linker.

A third novel contribution, **relocation-as-PCM** (`RelocMonoid.v`), gives the
algebraic structure the council flagged as missing: relocation patches form a
partial commutative monoid whose action on the output buffer is a homomorphism,
from which parallel determinism falls out as commutativity. This connects peony's
emit phase to the resource algebras (PCMs / Iris cameras) of separation logic.

Three further results (added after a second deep-research pass on incremental/
parallel-linking theory — Acar self-adjusting computation arXiv:1106.0478 /
arXiv:2105.06712, Brent/Graham work–span, Hopcroft partition refinement):

4. **Incremental Cost Bound** (`IncrementalCostBound.v`). The *quantitative*
   companion to soundness: under capacity-stability the incremental relink work
   equals the affected-set size (`incremental_cost_eq_num_changed`), so a
   single-file edit of an n-object program costs **1** while a from-scratch
   linker (mold, lld — which have no incremental mode) pays **n**
   (`incremental_beats_fromscratch`). This is the formal statement of *why*
   peony can beat mold on the edit–rebuild loop: same output bytes (Theorem A),
   asymptotically less work.

5. **Parallel Schedule Optimality** (`ParallelSchedule.v`). Because section
   copies write disjoint output ranges (`RelocDisjoint.v`), their dependence DAG
   is an antichain; the work–span model gives matching lower bounds
   (`span_lower_bound`, `work_lower_bound`) and a 2-approximation guarantee for
   the greedy ws-deque scheduler (`greedy_within_2x_opt`), with exact linear
   speedup when sections are uniform. This bounds the *best any linker can do* on
   the copy phase — the ceiling peony's parallel emit provably approaches.

6. **ICF Soundness** (`ICFSoundness.v`). Identical Code Folding is behaviour-
   preserving exactly on non-address-significant functions: calls are always
   safe to redirect (`icf_call_preserved`), and the only observable hazard is an
   address comparison between folded functions (`icf_addr_change_iff_folded`),
   which the address-safe side-condition excludes (`icf_observationally_equivalent`).
   The ICF equivalence refines content-equality (`icf_rel_refines_content`),
   validating Hopcroft-style partition refinement as the folding algorithm.

## Relationship to the implementation

These proofs are about the *model* of peony's algorithms, connected to the Rust
code by the following correspondences (the model-council's recommended
"prove the model, argue refinement separately" discipline):

- `SymbolLattice.sjoin` ↔ `peony-symbols::merge_symbol` (`⊕` resolution)
- `RelocDisjoint.apply1` / `footprint` ↔ `peony-reloc::apply_reloc` writing
  `width` bytes at `offset`; `pairwise_disjoint` ↔ the disjoint section file
  ranges that make `peony-emit::write_section_data_parallel` sound
- `SectionGC.bfs` ↔ `peony-layout::gc_sections` (S3-GC level-synchronous BFS)
- `Layout.layout_assign` ↔ `peony-layout::compute_layout` address assignment
- `IncrementalSoundness.capacity_stable` ↔ `peony-cache` red-green coloring +
  capacity padding; `render` ↔ the emit of section contents into the mmap
- `RelocMonoid.oplus` / `act` ↔ composing relocation writes into the output image
