# Peony Full Implementation Verification Tasks

This is the task ledger for turning the current Rocq model proofs and Rust
bridge tests into a real implementation-verification story.

Current status: Peony has **no** whole-implementation verification claim. The
repository has checked model proofs, theorem bridges, narrow
implementation-verified surfaces, and executable bridge tests. Broader
verification requires more refinement work connecting concrete Rust data
structures and execution paths to the Rocq models.

Do not describe Peony as implementation-verified until every required task below
is complete and the final gate passes.

## Definition of Done

Full implementation verification means:

- every verified Rust path has an explicit trusted boundary;
- every Rocq model has a concrete Rust witness or extractor for the values it
  reasons about;
- every theorem precondition is established by Rust code, not assumed by docs;
- every bridge has both a positive and a negative executable test;
- `make -C rocq-tests -B`, `coqchk`, Rust tests, and the bridge-check harness
  pass from a clean checkout;
- README and release-facing docs state only the verified claim.

## Trusted Base

These items remain trusted unless a later task removes them:

- Rust compiler and standard library semantics.
- `object` crate parsing for raw ELF fields not independently decoded by Peony.
- Host assembler/compiler/linker used to build differential fixtures.
- Rocq/Coq kernel and standard-library axioms listed by `Print Assumptions`.
- Linux x86-64 ELF loader behavior outside emitted byte equivalence checks.

Every task that depends on one of these boundaries must name it in the new proof
or bridge-test documentation.

## Required Task Tracks

### V0 - Verification Harness

Owner surface:

- `rocq-tests/`
- `peony/tests/`
- `peony-cache/tests/`
- `peony-emit/src/input_work/`
- new bridge harness code, if added

Tasks:

- [ ] V0.1 Create a bridge-fixture format that serializes the Rust-side witness
      data used by proof bridges. It must be stable enough for tests, not a
      public file format.
- [ ] V0.2 Add a single bridge-check command that runs the Rust witness tests,
      `make -C rocq-tests -B`, and the explicit `coqchk` invocation.
- [ ] V0.3 Add a proof-placeholder gate that fails on `Admitted`, `admit`,
      `Axiom`, `Parameter`, and `Conjecture` in `rocq-tests/*.v`, except for an
      allowlist that names standard Rocq library assumptions.
- [ ] V0.4 Add a checked mapping table from each theorem to the Rust function,
      witness type, test, and trusted boundary it depends on.
- [ ] V0.5 Add CI or a repo-local script entry for the full bridge gate.

Acceptance:

- `scripts/check-verification-bridges.sh` exits
  non-zero if any witness test, proof build, kernel check, or placeholder scan
  fails.
- The mapping table has no theorem with an empty Rust surface or test column.

QA:

- Happy path: run the bridge-check command on the current tree and save output
  under `.omo/evidence/peony-implementation-verification/v0/full-gate.txt`.
- Failure path: temporarily inject a proof placeholder in a scratch copy and
  show `bash scripts/check-verification-bridges.sh --scan-placeholders <path>`
  rejects it.

### V1 - Rust Witness Extraction Layer

Owner surface:

- `peony-object`
- `peony-symbols`
- `peony-layout`
- `peony-reloc`
- `peony-emit`
- `peony-cache`
- `rocq-tests/`

Tasks:

- [ ] V1.1 Define small Rust witness structs for the model values used by Rocq:
      symbols, sections, ranges, relocation writes, GC graph edges, layout
      windows, ICF fold classes, and incremental section colors.
- [ ] V1.2 Implement witness extraction from real `InputObject`, `SymbolTable`,
      `Layout`, emit work items, and partial-relink plans.
- [ ] V1.3 Prove or test that witness extraction is deterministic under stable
      input ordering.
- [ ] V1.4 Add round-trip sanity tests for every witness type using real object
      files when practical.
- [ ] V1.5 Document every field intentionally omitted from a witness and why it
      is outside the theorem's claim.

Acceptance:

- Every Rocq theorem bridge uses a named Rust witness type or explicitly remains
  marked model-only.
- Witness extraction tests include at least one negative case where missing or
  inconsistent Rust state rejects the bridge.

QA:

- Happy path: extract all witnesses from a small multi-object executable link.
- Failure path: corrupt one witness field in test code and verify the bridge
  test fails before proof claims are consulted.

Initial V1 worker status:

- `peony-verification` defines internal Rust witness structs for symbols,
  sections, checked ranges, relocation writes, GC roots/edges, layout windows,
  ICF folds, and incremental colors.
- Initial extractors exist for `InputObject` sections, `SymbolTable` symbols,
  `Layout` output-section windows, and `PartialRelinkPlan` section colors.
- Emit work range extraction now has an E1 bridge: `validate_work_item_ranges`
  returns an `AcceptedWorkRanges` token consumed by both serial and parallel
  input-section dispatch, while `peony-verification::EmitWorkRangeWitness`
  checks section-copy and planned relocation-footprint containment.
- This status is witness scaffolding only. It does not complete S1, G1, L1,
  R1, I1, or N1 refinement claims.

### S1 - Symbol Resolution Refinement

Current theorem surface:

- `rocq-tests/SymbolLattice.v`

Current Rust surface:

- `peony-symbols/src/lib.rs`
- `peony-symbols/tests/symbol_bridge.rs`
- archive and COMDAT consumers in `peony-object`, `peony-layout`, and `peony`

Tasks:

- [ ] S1.1 Extend the Rocq symbol model from strength-only resolution to the
      concrete Peony result shape: defined, undefined, weak, common, absolute,
      imported, duplicate-strong error, and provenance tie-break.
- [ ] S1.2 Model archive extraction and COMDAT discard inputs separately from
      final global symbol selection.
- [ ] S1.3 Extract a Rust symbol-resolution witness after the real pipeline
      finishes processing objects and archives.
- [ ] S1.4 Prove that the Rust witness corresponds to the Rocq resolution fold
      for non-error cases.
- [ ] S1.5 Prove or test that duplicate strong definitions map to the model's
      error result, not to an arbitrary winner.
- [ ] S1.6 Add real-object differential fixtures for weak override, common
      symbol, absolute symbol, archive extraction, and COMDAT cases.

Acceptance:

- The docs may say symbol resolution is bridge-verified only after all concrete
  symbol result variants used by Peony have a model case or an explicit trusted
  exclusion.

QA:

- Happy path: bridge witness for a multi-object link with weak-to-strong
  resolution matches the Rocq model.
- Failure path: duplicate strong definitions reject at both Rust and model
  bridge layers.

S1 worker status:

- `peony-symbols/tests/symbol_bridge.rs` now covers the concrete symbol-table
  outcomes for defined, undefined, weak override, weak-vs-weak first-winner
  provenance, common merge, absolute definition, imported shared export
  metadata, and duplicate-strong rejection.
- `peony-verification::SymbolWitness` now preserves imported symbol
  `version`/`soname` metadata, and `SymbolErrorWitness` maps duplicate-strong
  and undefined-symbol error paths into bridge-test data.
- This strengthens S1 witness/model coverage only. Archive extraction, COMDAT
  discard inputs, real-object differential fixtures, Rocq model extension, and
  full version semantics remain future S1 work.

### G1 - GC Reachability Refinement

Current theorem surface:

- `rocq-tests/SectionGC.v`

Current Rust surface:

- `peony-layout/src/gc.rs`
- `peony-layout/src/gc/live.rs`
- `peony-layout/tests/layout_gc_bridge.rs`

Tasks:

- [ ] G1.1 Define the exact graph extracted from Rust: root set, relocation
      edges, section group edges, EH metadata roots, init/fini roots, export
      roots, and `SHF_GNU_RETAIN` roots.
- [ ] G1.2 Extend `SectionGC.v` so the model roots and edges match the Rust
      witness shape instead of only an abstract graph.
- [ ] G1.3 Add a Rust witness extractor for the GC graph before traversal.
- [ ] G1.4 Prove that Rust `gc_sections` output equals the Rocq reachable set
      for the extracted graph.
- [ ] G1.5 Add negative tests for missing entry root, dropped unreachable
      section, retained alloc section, and relocation-chain reachability.

Acceptance:

- `gc_sections` may be called bridge-verified only for root and edge classes
  enumerated in the witness type.

QA:

- Happy path: an object graph with a two-hop relocation chain keeps all reachable
  sections.
- Failure path: deleting one extracted edge in a test witness makes the model and
  Rust live sets disagree.

G1 worker status:

- `peony-layout::gc_graph_rooted` now extracts the Rust GC graph before
  traversal from the same entry/export/implicit-root and relocation-target map
  logic used by `gc_sections`.
- The extracted root classes are entry, export, init/fini/preinit arrays,
  `.eh_frame`, `.gcc_except_table`, and alloc `SHF_GNU_RETAIN`. The extracted
  edge class is relocation target reachability.
- `peony_verification::GcReachabilityWitness` records roots, edges, and the Rust
  live set. `check_gc_witness` computes the SectionGC-style model reachable set
  and rejects mismatches before theorem use.
- Tests cover a two-hop relocation chain, unreachable-section rejection,
  retained alloc roots, all exposed root classes, missing-edge rejection, and
  missing-root rejection.
- Section-group edges and EH-frame FDE-to-code dependency edges remain not
  implementation-verified because the current GC implementation/API does not
  expose them as traversed edge classes. This G1 slice must not be described as
  whole-linker verification or as verification of GC behavior beyond the root
  and edge classes listed above.

### L1 - Layout Refinement

Current theorem surface:

- `rocq-tests/Layout.v`

Current Rust surface:

- `peony-layout/src/lib.rs`
- `peony-layout/tests/layout_gc_bridge.rs`

Tasks:

- [ ] L1.1 Split the Rust layout witness into input contributions, output
      section windows, segment windows, file offsets, virtual addresses, and
      alignment requirements.
- [ ] L1.2 Extend the Rocq layout model to cover every section/segment class
      Peony emits or explicitly exclude unsupported classes.
- [ ] L1.3 Prove Rust layout windows are aligned, page-congruent, in-bounds,
      and pairwise non-overlapping through the extracted witness.
- [ ] L1.4 Prove contribution placement preserves input-section byte order
      inside output sections.
- [ ] L1.5 Add real-object fixtures for alloc sections, non-alloc debug
      sections, BSS/NOBITS, TLS, RELRO, and dynamic sections.

Rust bridge status:

- L1 now extracts a `LayoutWitness` from `peony_layout::Layout`, including
  output-section windows, program-header windows, section header type/flags,
  file offsets, virtual ranges, alignments, and section-relative contribution
  ranges.
- `check_layout_witness` rejects malformed witnesses before theorem use:
  invalid alignment, file bounds escapes, non-alloc virtual ranges, alloc
  page-congruence failures, output-section overlap, PT_LOAD overlap, missing
  load containment, duplicate contribution ownership, contribution range/order
  overlap, and contribution bounds escapes.
- Current fixtures cover text, rodata, data, BSS/NOBITS, non-alloc debug, and
  TLS. RELRO/dynamic-section content semantics remain outside this L1 slice.
- This L1 bridge verifies layout witness properties only. It does not verify
  GC, symbol resolution, relocation address resolution, emit byte correctness,
  ICF, incremental relinking, or whole-linker correctness.

Acceptance:

- The layout bridge is not complete until section headers, program headers, and
  contribution ranges are all covered by witness checks.

QA:

- Happy path: witness for an executable with text, rodata, data, bss, debug, and
  TLS sections satisfies the Rocq layout predicates.
- Failure path: overlapping output-section windows are rejected before theorem
  use.

### R1 - Relocation Byte Refinement

Current theorem surface:

- `rocq-tests/RelocDisjoint.v`
- `rocq-tests/RelocMonoid.v`

Current Rust surface:

- `peony-reloc/src/apply.rs`
- `peony-reloc/src/lib.rs`
- `peony-emit/src/input_sections.rs`

Tasks:

- [ ] R1.1 Define a Rocq relocation expression model for every x86-64 relocation
      type Peony applies, including TLS relaxations and overflow behavior.
- [ ] R1.2 Extract a Rust relocation-write witness containing offset, width,
      original bytes, produced bytes, symbol value, addend, place, GOT/PLT/TLS
      addresses, and output range ownership.
- [ ] R1.3 Prove `patch_buf` writes exactly the modeled little-endian bytes for
      each supported relocation kind.
- [ ] R1.4 Prove rejected overflow paths correspond to model rejection, not
      silent truncation.
- [ ] R1.5 Connect `apply_reloc` symbol/layout address resolution to the witness
      used by `patch_buf`.
- [ ] R1.6 Prove that all writes stay within the owning emit work range, or make
      Rust reject the work item before relocation application.
- [ ] R1.7 Add real-object fixtures for every relocation kind Peony claims to
      support.

R1a Rust byte-model status:

- `peony-verification` now defines `RelocationByteInputs`,
  `X86_64RelocationExpression`, and `model_x86_64_relocation_bytes` for the
  x86-64 relocation expressions handled directly by `patch_buf`.
- `cargo test -p peony-reloc --lib` compares every supported `patch_buf` match
  arm against the model, including scalar absolute/PC-relative writes, GOT/GOTPC
  forms, TLS offset writes, TLSGD/TLSLD/TLSDESC relaxations, TLSDESC call
  rewriting, and the unsupported no-op fallback.
- The R1a failure surface covers signed and unsigned 32-bit rejection paths,
  including TLSGD relaxation immediates, so those paths reject instead of
  silently truncating.
- This is not whole R1 completion: Rocq-side relocation-expression proofs,
  full `apply_reloc` address-resolution witnesses, relocation write containment
  in emit work ranges, and real-object fixtures remain open R1/R1b work.

R1b Rust address-witness status:

- `peony-reloc::apply_reloc` now routes through a crate-local address-resolution
  action that records the exact `RelocAddrs` passed to `patch_buf` without
  changing production behavior.
- `cargo test -p peony-reloc --lib` covers local section symbols, global symbols,
  weak-undefined GOT references, PLT/GOTPCREL inputs, static TLS offsets, missing
  local target layout, and unresolved strong symbols. The positive cases feed
  the recorded address inputs into the R1a byte model and compare the bytes
  produced by the real `apply_reloc` path.
- `peony-verification` now defines `ApplyRelocAddressWitness` and
  `check_apply_reloc_address_witness`, which accept only when S/A/P/G/L/Z and
  TLS inputs match the symbol/layout/GOT/PLT/TLS resolution facts and reject
  missing or mismatched address facts before a byte-model claim is made.
- This strengthens R1.5 only. It still does not verify whole linker correctness,
  exhaustive real-object relocation coverage, dynamic-loader semantics, or final
  output-image write composition.

Acceptance:

- `RelocDisjoint.apply1` may be called an implementation bridge only after
  `apply_reloc` witness extraction proves both address calculation and byte
  writes, not just representative `patch_buf` formulas.

QA:

- Happy path: each supported relocation kind has a passing witness comparison
  against the model expression.
- Failure path: a relocation whose computed write extends past its section range
  is rejected before emit.

### E1 - Emit and Parallel Range Refinement

Current theorem surface:

- `rocq-tests/RelocDisjoint.v`

Current Rust surface:

- `peony-emit/src/sections.rs`
- `peony-emit/src/input_sections.rs`
- `peony-emit/src/input_work.rs`
- `peony-emit/src/input_work/tests.rs`

Tasks:

- [x] E1.1 Promote `validate_work_item_ranges` output into an explicit witness
      consumed by the parallel emit path.
- [x] E1.2 Prove every worker's section copy and planned relocation footprints
      for currently modeled emit writes are contained in its accepted half-open
      range.
- [ ] E1.3 Prove synthetic-section serial writes do not overlap parallel input
      work ranges, or separate their phases with an explicit frame theorem.
- [x] E1.4 Add bridge tests for serial and parallel paths using the same witness
      data.
- [x] E1.5 Add a negative test where overlapping work items reject before any
      worker receives a mutable slice.

Acceptance:

- The existing batch theorem stays valid for accepted input work ranges. E1 now
  establishes pre-dispatch rejection for overlapping work items, section-copy
  containment, and planned relocation-footprint containment for relocation types
  whose current emit writes expose a finite footprint. Full relocation
  expression/address correctness remains R1a/R1b work, and synthetic-section
  frame separation remains open under E1.3.

QA:

- Happy path: a multi-object parallel emit has accepted disjoint work ranges and
  byte-identical output across thread counts.
- Failure path: constructed overlapping work items fail validation before emit.

### I1 - ICF Address-Safety Refinement

Current theorem surface:

- `rocq-tests/ICFSoundness.v`

Current Rust surface:

- `peony-layout/src/icf.rs`
- `peony-layout/tests/icf_bridge.rs`

Tasks:

- [ ] I1.1 Enumerate every address-significance source Peony can observe:
      `.llvm_addrsig`, section-relative relocations, symbol relocations, dynamic
      references, exported symbols, and unsupported parser cases.
- [ ] I1.2 Extend the Rocq ICF model so `address_safe` is derived from a Rust
      address-taint witness.
- [ ] I1.3 Extract an ICF fold witness containing fold keys, canonical section,
      duplicate section, byte equality, relocation equality, and taint result.
- [ ] I1.4 Prove Rust `compute_fold_map` only folds sections satisfying the Rocq
      `address_safe` side condition.
- [ ] I1.5 Add negative tests for every address-significance source that must
      prevent folding.

Acceptance:

- ICF can be called bridge-verified only after every fold eligibility predicate
  in `compute_fold_map` is represented in the witness.

QA:

- Happy path: identical `.text` sections with empty `.llvm_addrsig` fold and
  satisfy the model.
- Failure path: taking a function address prevents folding through both Rust and
  model witness checks.

I1 Rust bridge status:

- `peony_verification::extract_icf_fold_witnesses` now consumes the Rust
  `compute_fold_map` output and records canonical/duplicate sections, fold keys,
  byte equality, relocation-summary equality, address-taint knowledge, and the
  derived address-safe predicate.
- `peony_verification::check_icf_fold_witnesses` recomputes current
  fold-eligibility predicates before the Rocq `address_safe` premise is used:
  text/non-empty section eligibility, resolved fold-key relocation targets,
  `.llvm_addrsig` presence/listing, named and section-relative address taint,
  ABI-unique `_ZTV`/`_ZTI`/`_ZTS` names, weak definitions, and default-visible
  non-local exports.
- This is not whole-linker verification. Object decoding, dynamic
  address-significance sources not represented in `InputObject`, ICF maximality,
  and layout/relocation/emit correctness remain outside the I1 claim.

### N1 - Incremental Emit Preservation Refinement

Current theorem surface:

- `rocq-tests/IncrementalSoundness.v`

Current Rust surface:

- `peony-cache/src/lib.rs`
- `peony-cache/tests/partial_relink.rs`
- `peony/tests/incremental.rs`
- `peony-emit`

Tasks:

- [ ] N1.1 Extend the incremental model from accepted planner metadata to the
      actual emit operation over the mmap output image.
- [ ] N1.2 Extract a partial-relink witness containing previous bytes, current
      red ranges, current green ranges, relocation-dependent red sections, and
      emitted byte ranges.
- [ ] N1.3 Prove every green byte in the witness is not written by the partial
      emit path.
- [ ] N1.4 Prove every red byte written by partial emit equals the corresponding
      full-link byte.
- [ ] N1.5 Add negative tests for stale green bytes, moved symbols, size drift,
      file-offset drift, virtual-address drift, and capacity overflow.

Acceptance:

- Incremental correctness may be stated as implementation-verified only after
  the write set of partial emit is connected to the `red` set in the model.

QA:

- Happy path: layout-reuse relink produces byte-identical output and the witness
  shows green ranges were untouched.
- Failure path: a symbol movement affecting a relocation marks the dependent
  section red and rejects a green-byte preservation claim.

### P1 - End-to-End Verified Claim Boundary

Owner surface:

- `README.md`
- `rocq-tests/README.md`
- `docs/`
- release notes or benchmark claims, if any

Tasks:

- [x] P1.1 Add a machine-readable claim table listing `model-only`,
      `bridge-tested`, `theorem-bridged`, and `implementation-verified` claims.
- [x] P1.2 Gate README wording on the claim table so docs cannot accidentally
      make a broad whole-linker verification claim while any required bridge
      remains incomplete.
- [x] P1.3 Add a final proof-audit document with `Print Assumptions` output for
      every theorem used in an implementation claim.
- [x] P1.4 Add a final differential corpus gate that links real programs
      byte-identically against the full-link path for the verified surfaces.
- [x] P1.5 Document every remaining trusted-base item and every unsupported ELF
      feature outside the claim.

Acceptance:

- A skeptical reader can tell exactly which claims are model proofs, which are
  Rust bridge tests, and which are full implementation-verification claims.

QA:

- Happy path: claim table lists all implemented bridges and docs match it.
- Failure path: changing README to overclaim verification fails a repo-local
  wording gate.

## Execution Order

Recommended sequence:

1. V0, because every later task needs a single gate and theorem-to-code map.
2. V1, because every proof bridge needs Rust witness extraction.
3. E1 and R1, because emit-range safety and relocation writes are the most
   direct byte-level correctness claims.
4. L1 and G1, because relocation and emit witnesses depend on layout and live
   section facts.
5. S1, because symbol resolution feeds relocation and layout addresses.
6. I1, because ICF is currently opt-in and side-condition heavy.
7. N1, because incremental emit preservation depends on layout, relocation,
   symbol, and emit witnesses.
8. P1, after the technical gates are true.

## Final Verification Wave

Run these after every required task is complete:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
make -C rocq-tests -B
coqchk -Q rocq-tests Peony \
  Peony.SymbolLattice \
  Peony.RelocDisjoint \
  Peony.RelocMonoid \
  Peony.SectionGC \
  Peony.Layout \
  Peony.IncrementalSoundness \
  Peony.IncrementalCostBound \
  Peony.ParallelSchedule \
  Peony.ICFSoundness
make -C rocq-tests check
```

The final verification wave is not enough by itself. It only counts after the
witness extraction and refinement tasks above exist and are included in the
bridge-check command.
