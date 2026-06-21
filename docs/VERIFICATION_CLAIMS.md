# Verification Claims

`docs/VERIFICATION_CLAIMS.json` is the machine-readable source of truth for
public verification wording. This page is the human-readable view of the same
boundary.

Peony currently has model proofs, Rust bridge tests, theorem bridges, and a few
narrow implementation-verified surfaces. It does not have a whole-linker
correctness claim.

## Status Meanings

| Status | Meaning |
| --- | --- |
| `model-only` | Rocq theorem or model result only; no Rust implementation claim. |
| `bridge-tested` | Rust witness/test correspondence exists, but it is not a theorem-backed implementation refinement claim. |
| `theorem-bridged` | A Rocq theorem is connected to a named Rust surface and checked preconditions for that surface. |
| `implementation-verified` | A narrow implementation surface has completed dependencies, Rust witnesses, audited theorem assumptions, and explicit scope limits. |

## Claim Table

| Claim id | Status | Public scope | Not claimed |
| --- | --- | --- | --- |
| `v0-v1-bridge-harness-and-witness-schema` | `bridge-tested` | Harness and witness infrastructure. | V0/V1 alone certify no Rust behavior. |
| `s1-symbol-resolution-witnesses` | `bridge-tested` | Synthetic S1 symbol-resolution witness subset. | Archive extraction, COMDAT discard, full version semantics, and real-object symbol resolution. |
| `e1-parallel-input-work-range-disjointness` | `implementation-verified` | Accepted input-section worker ranges before serial or parallel dispatch. | Synthetic-section frame separation and whole-emitter byte correctness. |
| `r1b-relocation-byte-and-address-witnesses` | `bridge-tested` | Representative `apply_reloc` address inputs feeding the R1a byte model. | Exhaustive real-object relocation coverage, dynamic-loader behavior, final output-image composition, and whole relocation correctness. |
| `l1-layout-window-predicates` | `implementation-verified` | Supported layout witness predicates for windows, alignment, bounds, and contribution ownership/order. | GC, symbols, relocation address selection, dynamic sections, emit bytes, and whole layout semantics. |
| `g1-gc-reachability-witnesses` | `implementation-verified` | Supported GC roots and relocation edges exposed through the current bridge API. | Section-group edges, EH-frame FDE-to-code edges, unsupported root classes, and whole-GC semantics. |
| `i1-icf-address-safety-for-emitted-folds` | `implementation-verified` | Folds emitted by `compute_fold_map` under supported current inputs. | ICF maximality, parser correctness, dynamic-loader behavior, layout correctness, relocation correctness, emit correctness, and whole-linker correctness. |
| `n1-partial-emit-byte-preservation-scoped-fixture` | `implementation-verified` | N1 changed-object fixture connecting color decisions and write reports to the red/green model. | Every mmap write in every mode, unreported synthetic-section rewrites, and whole incremental linker correctness. |
| `incremental-planner-capacity-stability` | `theorem-bridged` | Metadata accepted by `peony-cache::plan_partial_relink`. | Rust emit byte correctness. |
| `relocation-write-composition-model` | `model-only` | Rocq memory model. | Rust output-image write composition. |
| `abstract-incremental-cost-model` | `model-only` | Rocq work-count model. | Wall-clock performance or scheduler implementation proof. |
| `abstract-parallel-scheduler-model` | `model-only` | Rocq work-span model. | Hardware runtime-speed or Rust scheduler implementation proof. |

## Trusted Base

The claim JSON records trusted-base items per row. The recurring boundary is:
Rocq/Coq kernel and standard library, the Rust compiler, object decoding outside
the named fixtures, cache metadata serialization where named, and unsupported
ELF or dynamic-loader behavior outside each scoped bridge.

## Assumption Audit

`docs/verification-assumptions/` contains the `Print Assumptions` output for
every theorem used by a `theorem-bridged` or `implementation-verified` claim.
The bridge gate accepts only closed theorems or the standard functional
extensionality axiom already named by the Rocq README.
