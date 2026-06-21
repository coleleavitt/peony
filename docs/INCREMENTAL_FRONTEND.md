# Incremental Front-End — Sub-5ms One-File Relink (Executable Blueprint)

Status: Phase 0 + 2 + 3-4 + 5 LANDED (layout reuse + parse-only-changed +
object-granular/minimal emit + record fast-path; byte-identical, thread-stable).
A one-`.o` size-stable relink is now **~19ms vs a full link ~32ms** on the
402-obj harness — a ~38% win, down from the ~73ms NET LOSS we started with. Goal
remains a <5ms relink (10× vs mold's ~37ms-every-link). **Every phase MUST keep
the output byte-identical to a full link** (the non-negotiable gate).

> **THE load-bearing finding (2026-06-21, measured): for a one-shot CLI,
> persisting + deserializing front-end state costs ≈ recomputing it.** Layout
> blob deserialize ~1.36ms vs `compute_layout` ~6.9ms looks like a win, but the
> hazard fingerprint that GATES the reuse costs ~1.59ms (the id→name bijection
> fold — kept; it is the id-stability safety proof) + blob-hash 0.74ms, so the
> whole layout phase is ~4.8ms ≈ a from-scratch layout. The SAME wall kills
> Phases 3–4: deserializing 16k symbol resolutions ≈ re-resolving them
> (resolve-bare is only ~4ms). **So blob-persistence is structurally break-even;
> the 10× thesis needs a resident DAEMON (parse/resolve/layout/symbols stay in
> RAM, only the changed `.o` is reprocessed) or true zero-copy mmap'd state —
> NOT a faster bincode.** The mmap pivot is NOT worth it as a layout-only change:
> the deserialize it removes is just ~1.36ms.
>
> What Phases 2+5 DID deliver (all reusable, all byte-identical-gated): the
> hard-gated **hazard fingerprint** (`compute_layout_fingerprint`), the
> **persistence schema** (`FrontEndSnapshot` + separate `layout.bin`),
> **object-granular emit** (`emit_partial_objects` — only the changed object's
> contributions, not all of reddened `.text`), and the **record fast path**
> (separate blob file + killed two vestigial whole-output hashes: 6ms→2ms). The
> remaining big lever is **parsing only the changed object** (~8ms parse+resolve
> is fully redundant on reuse): resolve the changed object's relocs against the
> cached `name→VA` map + reuse layout, skipping the 406 unchanged parses — a
> large dedicated restructure, the real Phase 3–4.

Distilled from a 6-agent design pass (peony-object/parse, peony-symbols/resolve,
peony-layout, peony-reloc, peony-cache, and Wild/mold reference) + direct
measurement on a 402-object harness.

## Current state (measured)

A full ~402-object link is ~40ms: parse+resolve ~10ms (parse-bare 4.3ms parallel
+ resolve-bare 4ms SERIAL), reloc ~5ms, layout ~9ms, finalize ~1.3ms, emit ~7ms.

Incremental today is **emit-only and was a NET LOSS** for a real change:
- `incremental_emit_plan` runs AFTER full parse+resolve+layout (main.rs:~637).
- A one-file `partial_relink` was ~73ms — SLOWER than a full link — because the
  whole front-end re-ran AND `record_link_with_sections` content-hashed every
  input on every relink (~47ms).
- **Phase 0 (LANDED, commit d91ee55):** record reuses cached content fingerprints
  for unchanged inputs → `incremental:record` 47ms→5ms, byte-identical. Relink
  ~73→~47ms. Still > full, because the front-end (parse/resolve/layout) re-runs
  and `emit_partial` still iterates all 16k sections.

## The byte-identity theorem (why reuse is safe)

When the ONLY change is one `.o` whose contributing allocatable sections keep
identical **size + align + output-section + order**, and there is no hazard
(below), then `place()` threads the same cumulative VA at every step, so EVERY
`sh_addr`/`sh_offset` and every `(object,section)->address` entry is byte-
identical to a full relayout. `plan_partial_relink` already PROVES this post-hoc
(it bails on file_offset/vaddr/size mismatch). The waste today is *recomputing*
the layout we could *reuse*. SymbolId numeric values are decoupled from output
bytes EXCEPT common-symbol `.bss` order — so id reuse is byte-safe unless common
symbols are involved.

## Hazards → MUST fall back to a full link (the correctness core)

Detect these BEFORE reusing; on ANY uncertainty, full-link (always correct):

1. **Size change** beyond reserved capacity → addresses shift. (`plan_partial_relink`
   already bails: `SectionSizeChanged`.)
2. **GC liveness change** — a new/removed reference flips the live set → relayout.
   Wild *descopes `--gc-sections`* from incremental entirely. peony must detect a
   live-set delta (or descope gc for the fast path) and fall back.
3. **Symbol-set change** — a new/removed global name renumbers `SymbolId`
   (positional `SymbolId(names.len())`), perturbing `.symtab`/reloc `r_info` and
   common `.bss` order. Reuse ids ONLY for surviving names; fall back if the
   changed object added/removed a global, or was the unique strong definer of a
   still-referenced name.
4. **New archive member pulled** — a changed undefined ref can lazily extract a
   new `.a` member → object-set + id + address changes. Wild descopes archives.
5. **COMDAT change** — first-group-wins (`seen_comdat`); subtract/re-add can flip
   which copy is kept.
6. **ICF** — a content change can break/create a fold (`fold_map`).
7. **GOT/PLT/synthetic count change** — a changed object adding a GOT-needing ref
   shifts `.got`/`.plt`/`.rela.dyn` sizes and all later addresses.
8. **Strictly-ordered sections** (`.init_array`/`.ctors`/start-stop) cannot
   tolerate gaps — fall back if they change.

## What must be persisted (today's manifest is insufficient)

The manifest (peony-cache) has: FastFingerprint+content Fingerprint per input,
`SectionRecord` (output section name/addr/offset/size/**capacity**/fingerprint),
`CachedSymbolEntry` (name→vaddr,got_addr), reloc reverse index. **Missing:**
- The **SymbolId↔name↔defining-ObjectId map + id assignment order** (today only
  name→{vaddr,got} is stored; the id is not even recoverable).
- **Per-(object,input-section) contributions** (object_id, section_index, offset,
  size, align) so a changed object can be diffed contribution-by-contribution and
  the per-input-section address map (`output.addr + contribution.offset`) rebuilt.
- Synthetic-size drivers: got_syms/plt_syms order, common/copy set+sizes,
  tls.byte_size, dynamic counts, phnum.
- Per-object COMDAT-excluded set + lazily-extracted archive-member set.
- The GC live-set (to detect a liveness delta cheaply).
Do NOT cache parsed `InputObject`s — re-parsing the one changed object (~10-20µs)
is cheaper than deserializing + re-interning + re-binding arena file_ids.

## Phased plan (each shippable + `cmp`-gated)

| Phase | Reuse / change | Skips | Relink |
|---|---|---|---|
| 0 ✅ | record reuses unchanged-input fingerprints | re-hash all inputs | 73→47ms |
| 1 | Capacity SLACK on initial link (Wild STEP 1) so a same-or-smaller edit keeps file_offset/vaddr → `plan_partial_relink` stays green for size-*growing* edits too | — (robustness; **changes baseline output** → re-baseline) | — |
| 2 ✅ | Reuse cached **layout** when size-stable + no-hazard: hazard-fingerprint the whole front-end, and on a match deserialize the cached `Layout` and skip `compute_layout` (substituted *before* finalize, so finalize/reloc/emit run unchanged). Descopes `--gc-sections`/`--icf` + changed non-object inputs → full-link. **Net only ~1.5ms** (deserialize ~5.5ms ≈ compute ~6.9ms — see finding above). Also skips the reverse-index build on reuse (no symbol can move) and re-persists the cached blob verbatim (no re-serialize). | `compute_layout` minus deserialize | ~1.5ms saved |
| 5a ✅ | **Record fast path**: serialized layout moved to its own `layout.bin` (reuse relink rewrites only the small manifest, not the ~MB blob); killed two vestigial whole-output hashes (`SectionRecord.fingerprint` for dead `compute_red_green`, and the never-read `manifest.output`). | `incremental:record` 6ms→2ms | record −4ms |
| 5b ✅ | **Object-granular emit** (`emit_partial_objects`): a drivers match proves no address moved, so re-emit ONLY the changed objects' contributions (overwrite-in-place leaves all else byte-identical) instead of all of reddened `.text`. section-copy 16407 sections → 3 on the harness; scales with project size. | unchanged objects' input copy | emit 7→5ms |
| 3–4 ✅ | **Parse only the changed object** (`try_parse_only_changed`, runs BEFORE the full parse): parse just the changed `.o`(s), reuse the cached layout + symbol manifest, re-apply only its relocations against a MINIMAL `name→{va,got,plt,size}` view, and minimal-emit. Skips the 406 unchanged parses + the whole re-resolve + post-layout dynamic/symbol machinery. Gated by a reloc-complete digest match + a PC/GOT/PLT reloc whitelist (R64/TLS/IFUNC/COPY/absolute descope); falls through to the full pipeline (which still reuses the layout) on any ineligibility. | most of `parse+resolve` + finalize/append | **~32→~19ms** |

**Phase 2 implementation (landed):** serde on `Layout`+nested types (peony-layout/object/symbols);
`compute_layout_fingerprint` (per-object geometry+symbol digests, reused for unchanged objects, +
global drivers: full id→name bijection, GOT/PLT/TLS demand, commons, copy relocs, imports/exports,
config); manifest v6 `FrontEndSnapshot{drivers_hash, blob_hash, object_digests, object_paths}` +
the serialized layout in a separate `<output>.incr/layout.bin` (`blob_hash` guards a stale blob);
driver `try_reuse_layout` gate before `compute_layout`. Gate proven by `peony/tests/incremental.rs`
(byte-identical vs full link, alternating relinks) + the `/tmp/incbench` 402-object harness
(0 mismatches across 6 thread counts + cross-thread reuse). NOTE the original `compute_a.o`/`compute_b.o`
fixtures were compiled from differently-NAMED sources (`ca.c`/`cb.c`) → different `STT_FILE` symbol →
`.symtab` genuinely differs → reuse CORRECTLY declines; use same-filename `compute_42.o`/`compute_77.o`.

Phases 2–4 are coupled (they share the persisted id-order + contribution cache),
so the real first big slice is "persist front-end state + reuse it when no
hazard." Lead with the **HARD-GATED safe path**: reuse only when provably pure,
else full-link. Mirror Wild: descope `--gc-sections` + archives from the fast
path initially (full-link those), widen later.

## Verification harness (ready)

`/tmp/incbench`: 402 objects (40 fns each, cross-referenced), `ld.args` extracted
from `cc -### -B<peony-as-ld>`. The gate, run on every phase:
```
# alternate compute.o between two SIZE-STABLE variants each iteration (untimed swap),
# time `peony --incremental -o app @ld.args`, then `peony -o full @ld.args` and
# `cmp -s app full` — MUST be 0 mismatches across many alternating relinks.
```
Also sweep determinism with `RAYON_NUM_THREADS=1..32`. Keep ripgrep
(`target/bench-mold-current/reuse-direct-bench/ripgrep.ld.rsp`) byte-identical
throughout. NOTE: incremental needs DIRECT peony invocation — via `cc -B` the
driver touches the output and the cache state is rejected (full-emit fallback).

## Reference: Wild's persistent state (research/wild-incremental-design.html)

`output.incr/` dir, mmap-friendly host-endian (cross-machine NOT a use case):
index of inputs+args+linker-version (mismatch → full); persistent name→SymbolId
bimap (peony's `SymbolId(names.len())` must become a stable persistent bimap);
resolution table (addr/got/plt) as an mmapped Vec; per-output-section size AND
capacity (reserve slack). Wild explicitly descopes `--gc-sections`, archives, and
gap-sensitive ordered sections from incremental, full-linking those cases.
