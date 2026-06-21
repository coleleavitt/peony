# peony — Rocq/Coq Formalization

Machine-checked model proofs for selected linker invariants in
[`../research/QUAD.md`](../research/QUAD.md). These files are useful as precise
specifications and proof sketches for implementation invariants; they are not a
whole-program verification of the Rust linker.

## Build

```sh
make            # compile every proof; non-zero exit on any unproved goal
make clean
```

Requires the **Rocq Prover >= 9.0** (or Coq >= 8.18); developed against Rocq
9.1.0. `coqc` exits non-zero on any incomplete proof, so a green `make`
certifies these Rocq models. It does **not** certify the Rust implementation
unless a theorem explicitly states and proves an implementation bridge. There
are **no `Admitted`, `admit`, or custom `Axiom`s** anywhere; the only axiom used
is standard **functional extensionality** (for the `addr -> byte` memory model),
confirmed via `Print Assumptions`. The permutation-invariance / determinism
results for symbol resolution are *closed under the global context* (zero
axioms).

## Files and what they prove

| File | QUAD.md § | Headline results |
|---|---|---|
| `SymbolLattice.v` | §2 | `sjoin_assoc`, `sjoin_comm`, idempotence (non-strong); `resolve_perm_invariant` / `parallel_resolution_deterministic` (order-independent merge); `pick_assoc` + `pick_is_input` (argmax resolution under provenance tie-break) |
| `RelocDisjoint.v` | §5 | `apply_comm` (disjoint relocations commute — the frame property); `apply_all_perm_invariant` / `parallel_reloc_deterministic` (parallel apply = sequential); `accepted_emit_ranges_batch_precondition` / `accepted_emit_ranges_parallel_batches_deterministic` (checked section output ranges imply cross-batch disjointness for the current emitter) |
| `RelocMonoid.v` | §5 (new framing) | Model algebra: relocation patches form a *partial commutative monoid*; `act_oplus` (action is a homomorphism), `act_comm`, `reloc_pcm_summary` |
| `SectionGC.v` | §3 | `gc_sound` (every GC-live section is reachable), `gc_contains_roots`, `bfs_superset_*` (level-synchronous BFS = reachability) |
| `Layout.v` | §4 | `layout_all_aligned` (page alignment), `layout_page_congruent` (vaddr ≡ fileoff mod page), `layout_addr_lower_bound`, `layout_no_overlap`, and `layout_pairwise_no_overlap` (disjoint windows) |
| `IncrementalSoundness.v` | §6, §10.2 | `accepted_patch_metadata_implies_layout_compatible`, `accepted_patch_plan_implies_capacity_stable`, and `incremental_relink_sound`: a checked bridge from `peony-cache::plan_partial_relink`'s accepted metadata shape to the model's `capacity_stable` invariant, plus model lemmas `green_is_byte_stable`, `red_is_forced`, and `minimal_cut_characterization` |
| `IncrementalCostBound.v` | §6 (new) | Model-level cost results: `incremental_beats_fromscratch`, `single_edit_cost_is_one`, `fromscratch_grows_unboundedly`, and `capacity_stable_cost_bounded`. These are abstract work-count results, not wall-clock performance proofs. |
| `ParallelSchedule.v` | §7 (new) | Model-level work-span (Brent) bounds for an ideal disjoint-footprint section-copy antichain: `span_lower_bound`, `work_lower_bound`, `greedy_within_2x_opt`, `linear_speedup_regime` |
| `ICFSoundness.v` | §9 (new) | Model-level ICF soundness lemmas: `icf_call_preserved`, `icf_addr_change_iff_folded`, `icf_observationally_equivalent`, `icf_rel_refines_content` |

## What Is Actually Verified

The deep-research prior-art survey (`../research/`, papers downloaded and
`pdftotext`-extracted under `../research/txt/`) confirmed that while the
*general* meta-theory of incremental computation exists —
Acar/Blume/Donham's *from-scratch consistency* (`acar2011-…`), Cai et al.'s
*incremental λ-calculus* derivative law `f(a ⊕ da) = f(a) ⊕ D⟦f⟧ a da`
(`cai2013-…`), Mokhov et al.'s build-system *minimality* (`mokhov2018-…`) — none
of it is specialized to **byte-level linking / relocation**. CompCert-line work
(Kang et al. POPL'16) verifies *separate compilation* at program-semantics level,
explicitly leaving the byte-level link step in the trusted base.

1. **Incremental planner gate bridge** (`IncrementalSoundness.v`, Theorem A).
   The checked theorem `incremental_relink_sound` now proves that the accepted
   metadata shape modeled after `peony-cache::plan_partial_relink` implies the
   Rocq model's `capacity_stable` invariant. The matching Rust tests live in
   `peony-cache/tests/partial_relink.rs` and exercise every planner fallback
   reason plus relocation-dependent red coloring.

2. **Parallel emit range bridge** (`RelocDisjoint.v`).
   `accepted_emit_ranges_batch_precondition` proves that accepted half-open
   section output ranges imply cross-batch relocation disjointness when every
   relocation footprint in a worker batch fits inside that worker's range. This
   matches the Rust shape: one work item per input-section contribution, with
   relocations inside that item applied sequentially by one worker. The Rust
   precondition is enforced by
   `peony-emit::input_work::validate_work_item_ranges` before
   `copy_input_sections` dispatches serial or parallel workers; unit tests cover
   adjacent ranges, overlap, overflow, out-of-bounds ranges, and zero-length
   ranges.

3. **Minimal Recomputation Cut** (`IncrementalSoundness.v`, Theorem B =
   `minimal_cut_characterization`). In this model, the red/green partition by
   content-change is the *unique minimal correct cut*: every green region is byte-stable
   (`green_is_byte_stable`, so it is sound to skip) and every red region is
   *forced* (`red_is_forced` exhibits a witnessing base memory at which the
   rendered byte differs). This is Mokhov-style minimality lifted from file
   granularity to byte granularity for a linker.

The **relocation-as-PCM** model (`RelocMonoid.v`) gives the
algebraic structure the council flagged as missing: relocation patches form a
partial commutative monoid whose action on the output buffer is a homomorphism,
from which parallel determinism falls out as commutativity.

Three further model results (added after a second deep-research pass on incremental/
parallel-linking theory — Acar self-adjusting computation arXiv:1106.0478 /
arXiv:2105.06712, Brent/Graham work–span, Hopcroft partition refinement):

4. **Incremental Cost Bound** (`IncrementalCostBound.v`). The *quantitative*
   companion to soundness: under capacity-stability the incremental relink work
   equals the affected-set size (`incremental_cost_eq_num_changed`), so a
   single-file edit of an n-object program costs **1** while a from-scratch
   linker (mold, lld — which have no incremental mode) pays **n**
   (`incremental_beats_fromscratch`). This is an abstract work-count result; it
   supports but does not replace empirical benchmarking.

5. **Parallel Schedule Bounds** (`ParallelSchedule.v`). Because section
   copies write disjoint output ranges (`RelocDisjoint.v`), their dependence DAG
   is an antichain; the work–span model gives matching lower bounds
   (`span_lower_bound`, `work_lower_bound`) and a 2-approximation guarantee for
   the greedy ws-deque scheduler model (`greedy_within_2x_opt`), with exact
   linear speedup when sections are uniform. This is not a proof of real runtime
   speed on hardware.

6. **ICF Soundness** (`ICFSoundness.v`). Identical Code Folding is behaviour-
   preserving exactly on non-address-significant functions: calls are always
   safe to redirect (`icf_call_preserved`), and the only observable hazard is an
   address comparison between folded functions (`icf_addr_change_iff_folded`),
   which the address-safe side-condition excludes (`icf_observationally_equivalent`).
   The ICF equivalence refines content-equality (`icf_rel_refines_content`),
   validating Hopcroft-style partition refinement as the folding algorithm.

## Relationship to the Implementation

The public claim boundary lives in
[`../docs/VERIFICATION_CLAIMS.md`](../docs/VERIFICATION_CLAIMS.md), with the
machine-readable table in
[`../docs/VERIFICATION_CLAIMS.json`](../docs/VERIFICATION_CLAIMS.json). That
boundary separates model-only results, bridge-tested correspondences, theorem
bridges, and narrow implementation-verified surfaces.

The theorem-to-Rust bridge map is
[`../docs/THEOREM_TO_RUST_BRIDGES.md`](../docs/THEOREM_TO_RUST_BRIDGES.md).
Every theorem used by a theorem-backed public claim has a raw
`Print Assumptions` artifact under
[`../docs/verification-assumptions/`](../docs/verification-assumptions/).

The current Rust bridge tests check these concrete correspondences:

- `peony-reloc` unit tests lock representative `patch_buf` byte formulas for
  `R_X86_64_64`, `PC32`, `PLT32`, `GOTPCREL`, and `SIZE32`, plus the checked
  overflow path.
- `peony-layout/tests/layout_gc_bridge.rs` exercises `compute_layout`
  page-congruence/non-overlap plus `gc_sections` reachability and
  `SHF_GNU_RETAIN` roots.
- `peony-layout/tests/icf_bridge.rs` exercises ICF's positive `.llvm_addrsig`
  fold plus negative coverage for `.llvm_addrsig`-listed symbols, named and
  section-relative address taint, exported/default-visible symbols, weak
  definitions, ABI-unique `_ZT*` names, missing addrsig metadata, and unresolved
  fold-key relocation targets.
- `peony-symbols/tests/symbol_bridge.rs` exercises the global symbol table's
  undefined-to-strong, weak-to-strong, and duplicate-strong rules.
- `peony-cache/tests/partial_relink.rs` and `peony-emit::input_work` tests cover
  planner metadata and accepted input-work ranges.
- `peony/tests/incremental.rs` checks the scoped partial-emit byte identity path
  used by the N1 claim.

The remaining items below are still model-only results or scoped
correspondences unless the claim table states a narrower theorem-backed status.
The executable task ledger for the verification work is
[`../docs/IMPLEMENTATION_VERIFICATION_TASKS.md`](../docs/IMPLEMENTATION_VERIFICATION_TASKS.md).

- `SymbolLattice.sjoin` ↔ the full `peony-symbols` resolution pipeline
  (`⊕` resolution plus archive/version/COMDAT interactions)
- `RelocDisjoint.apply1` / `footprint` ↔ `peony-reloc::apply_reloc` writing
  `width` bytes at `offset`; cross-batch disjointness is now bridged from the
  checked `peony-emit` section work ranges, while end-to-end byte-level
  `apply_reloc` refinement, including symbol/layout address resolution, is still
  a correspondence
- `SectionGC.bfs` ↔ the full `peony-layout::gc_sections` graph construction and
  traversal
- `Layout.layout_assign` ↔ the full `peony-layout::compute_layout` segment and
  section-header assignment
- `ICFSoundness.address_safe` ↔ the current Rust ICF pass's checked
  address-safety witness for folds emitted by `compute_fold_map`; object parser
  correctness and unsupported dynamic address-significance sources remain
  outside this bridge
- `IncrementalSoundness.capacity_stable` ↔ the accepted `peony-cache`
  partial-relink planner metadata; `render` ↔ the emit of section contents into
  the mmap
- `RelocMonoid.oplus` / `act` ↔ composing relocation writes into the output image
