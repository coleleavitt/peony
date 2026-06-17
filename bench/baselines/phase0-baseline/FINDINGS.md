# Phase-0 findings — the measurement that re-ranks the rewrite

Measured on a 24-core machine, cold link, replayed through `cc -B<peony>` (crt +
libc, identical to bench.sh). Corpus: ripgrep, 419 inputs. peony `--release`.

## Per-phase self-time, 1 thread vs 24 threads

| phase            | t=1      | t=24     | scaling | % of t=24 |
|------------------|---------:|---------:|--------:|----------:|
| emit             | 76.43ms  | 66.62ms  | 1.15×   | **38.6%** |
| parse+resolve    | 56.41ms  | 42.13ms  | 1.34×   | 24.4%     |
| other (untimed)  | 34.33ms  | 33.07ms  | 1.04×   | **19.2%** |
| layout           | 14.83ms  | 14.21ms  | 1.04×   | 8.2%      |
| resolve-inputs   | 15.90ms  | 12.98ms  | 1.22×   | 7.5%      |
| reloc-scan       |  9.79ms  |  3.48ms  | 2.81×   | 2.0%      |
| **TOTAL**        | 207.69ms | 172.50ms | 1.20×   |           |

## parse+resolve sub-breakdown (`--trace`, t=24, 43ms)

| sub-phase               | wall    | note                                  |
|-------------------------|--------:|---------------------------------------|
| classify-inputs         |  1.09ms |                                       |
| parse-bare              | 13.88ms | already parallel (419 ≥ 256 thresh)   |
| **resolve-bare**        | **5.50ms** | the serial symbol-table fold        |
| include_archive_members | 12.76ms | lazy archive fixpoint                 |

## The decisive conclusions

1. **The sharded parallel symbol resolver is LOW ROI.** It was planned as the
   "max-effort centerpiece" to close the ≈1.1× scaling gap. But `resolve-bare`
   is **5.50ms = 3.2%** of the link. Even a perfect ∞-core resolver saves ≤5.5ms
   off 172ms. The council + advisor both flagged this risk; the measurement
   confirms it. **Demoted to conditional/last; do not spend max effort here.**

2. **emit is the #1 lever: 38.6% and scales only 1.15×.** A real parallel emit
   (blit mmap→output by disjoint offset, no intermediate copy) is the single
   biggest opportunity. The section-copy is *already* threaded above a 2048-item
   threshold, so the 1.15× says the bottleneck is elsewhere in emit (reloc
   apply, symtab, the per-byte `build_id_hash` over all 17MB of section data, or
   the output write/msync) — instrument it.

3. **"other" is 19.2% and entirely untimed.** 33ms of unattributed cost between
   the named phases (init_thread_pool spin-up, the ~190 lines of GOT/PLT/TLS/
   copy-reloc post-processing between reloc-scan and layout, fixed per-link
   overhead). It scales 1.04× (serial). **Must be instrumented** — it may hide
   the cheapest wins.

4. **Zero-copy (P1) is still the right foundational first step**, but for
   broader reasons than "parse speed": it removes the 17MB section-byte copy in
   parse-bare (part of the 13.88ms), lets emit blit straight from the mmap
   (attacks the 38.6%), and eliminates the teardown free of thousands of section
   `Vec`s. It is the enabling refactor for the two phases that actually dominate.

5. **layout (8.2%, 1.04×) is a serial Amdahl tail** — a legitimate but smaller
   target than emit/other.

## Re-ranked plan (by measured serial cost × poor scaling)

1. **P1 zero-copy section data** (foundational; unblocks emit + parse).
2. **Instrument "other"** — split the 33ms untimed mass with phase markers.
3. **Parallel/zero-copy emit** — the 38.6% phase, the biggest single lever.
4. **Parallel parse / archive** — parse-bare + archive ≈27ms combined.
5. **Layout serial tail** — if it dominates after the above.
6. **Sharded resolver — CONDITIONAL.** Only if, after 1–4, resolve-bare is still
   a top-2 serial cost (the measurement says it will not be).

## Baseline output sha256 (the byte-compare gate for every later phase)

See `sha256.txt`. All four corpora linked **deterministically** (link-twice
sha256 matched itself) at this baseline.

---

# Phase-1 update — instrumentation attributed "other" + emit internals

After adding `phase()` markers to the untimed gaps and `trace()` frames inside
emit (byte-identical output, all tests green), the ripgrep breakdown is now:

| phase           | t=24     | note                                       |
|-----------------|---------:|--------------------------------------------|
| **emit**        | 78.97ms  | **44.5%** — see emit internals below       |
| parse+resolve   | 40.32ms  | 22.7%                                      |
| **reloc-postproc** | 17.77ms | **10.0%** — was hidden in "other"        |
| layout          | 12.03ms  | 6.8%                                       |
| resolve-inputs  | 14.98ms  | 8.4%                                       |
| finalize-syms   |  5.94ms  | 3.3% — was hidden in "other"               |
| reloc-scan      |  3.12ms  | 1.8%                                       |
| other           |  4.40ms  | 2.5% (was 19.2% — now attributed)          |

## emit internals (`--trace`), t=24 vs t=1 — THE SMOKING GUN

| sub-phase                  | t=24    | t=1     | scaling | verdict             |
|----------------------------|--------:|--------:|--------:|---------------------|
| **emit:build-id-hash**     | **52.97ms** | **53.64ms** | **1.00×** | SERIAL, 30% of link |
| emit:flush                 | 15.71ms |  4.94ms | 0.31×   | INVERTED (msync)    |
| emit:section-copy-dispatch |  5.00ms | 15.09ms | 3.02×   | scales fine         |
| emit:sym-index-build       |  2.77ms |  2.65ms | —       | small               |
| emit:mmap-open             |  1.32ms |  1.38ms | —       | small               |

### The decisive finding

**`build_id_hash` is the single biggest cost in the whole link — 53ms = 30%,
fully serial, and it is doing far too much work:**

- `cc` injects `--build-id` by default (`--enable-linker-build-id`), so it runs
  on every real link — cannot be skipped.
- It is a **byte-at-a-time double-FNV** (`build_id_hash`, peony-emit:587) over
  **all ~18.5MB of *input* section bytes** — including `.debug_*`, COMDAT-
  discarded, and non-allocated sections — scattered across thousands of separate
  `Vec`s. Throughput ≈349 MB/s.
- The emitted output is only ~4MB. mold/lld compute the build-id over the
  **contiguous output image** (and use a parallel tree hash), not the scattered
  oversized input set.

**This dwarfs everything the original plan targeted.** The sharded resolver
(5.5ms) and even all of parse (14ms) are noise next to this 53ms serial hash.

### Re-ranked again (by this measurement)

1. **build-id hash** — biggest single win (30%), low-risk, INDEPENDENT of the
   zero-copy refactor. Hash the contiguous output buffer (4MB not 18.5MB) with a
   parallel/blocked hash. NOTE: changes the 16 build-id bytes → re-baseline the
   sha256 gate (peony's build-id was never byte-identical to ld's anyway — the
   gate is self-consistency, validated by executing the output).
2. **emit:flush 15.7ms, inverted scaling** — msync/dirty-page writeback; smaller.
3. zero-copy + parallel emit copy (the dispatch already scales 3×).
4. reloc-postproc 17.8ms — newly surfaced #3 phase.
5. everything else (parse, layout) — smaller.
6. sharded resolver — still last/conditional.

---

# Phase-2 result — build-id hash fixed (the 30% win, SHIPPED)

Changed `build_id_hash` to hash the **contiguous output image (~4MB)** in
parallel 256KB blocks (folded in index order → thread-count-independent),
instead of a serial byte-at-a-time double-FNV over **all ~18.5MB of scattered
input** sections. Split `write_build_id` into a header-only write (synthetic
pass) + `finalize_build_id` run after the whole image is written.

## Measured on ripgrep (threads=0)

| metric              | before   | after    | delta        |
|---------------------|---------:|---------:|--------------|
| emit:build-id-hash  | 52.97ms  |  3.08ms  | **17× faster** |
| emit (total)        | 78.97ms  | ~15-25ms | ~3-5× faster |
| **TOTAL link**      | 177.54ms | **~120-128ms** | **~30% faster** |

## Gates (all pass)

- **Determinism:** 4 links (threads=0 ×2, threads=1 ×2) → **1 distinct sha256**.
  The blocked hash folds in index order, so the build-id is identical across
  thread counts.
- **Correctness:** the linked ripgrep runs (`ripgrep 15.1.0`, exit 0) with a
  valid `.note.gnu.build-id`; bench.sh correctness gate passes on all 4 corpora
  (peony output matches the lld reference).
- **Tests:** full `cargo test --workspace` green, incl. `features::build_id_note`
  (determinism + note presence).

## sha256 re-baselined

build-id bytes legitimately changed (peony's custom FNV was never byte-identical
to ld's anyway). New deterministic baseline: `bench/baselines/p2-buildid/`.
ripgrep = `ff8e621e…`. This is the byte-compare gate for subsequent phases.

## Post-Phase-2 ripgrep profile (the new target ranking)

| phase           | t=0      | %     |
|-----------------|---------:|------:|
| parse+resolve   | 41.31ms  | 32.1% |
| reloc-postproc  | 21.05ms  | 16.4% |
| emit            | 25.09ms  | 19.5% |
| resolve-inputs  | 14.48ms  | 11.3% |
| layout          | 13.29ms  | 10.3% |
| finalize-syms   |  5.45ms  |  4.2% |
| reloc-scan      |  2.94ms  |  2.3% |

Next target: parse+resolve (32%) and the newly-surfaced reloc-postproc (16.4%).

---

# Phase-3 result — zero-copy section data (SHIPPED)

Replaced `InputSection.data: Vec<u8>` (a per-section copy out of the mmap) with
`SectionData { src, off, len }` — a `Copy+Send+'static` index into a link-wide
`InputArena { mmaps, owned }`. Bare objects (the bulk of a link) now borrow their
section bytes straight from the input `mmap`; only compressed `.debug_*` is owned.
`.eh_frame` terminator-strip is a `len -= 4` slice (no copy). Archives copy the
member blob into the arena's owned store once (not zero-copy from the `.a`, a
small non-hot cost). ICF `FoldKey` holds a 128-bit content digest (no byte clone)
+ a verify-on-collision byte compare (sound). Design per two council rounds:
central arena, no lifetime parameter, atomic type-flip, parallelization deferred
to separate commits.

## Measured on ripgrep (threads=0)

| metric          | before (p2) | after (p3) | delta            |
|-----------------|------------:|-----------:|------------------|
| parse+resolve   | ~41ms       | 27.4ms     | the 17MB copy gone |
| emit            | ~25ms       | 12.5ms     | blit straight from mmap |
| **TOTAL link**  | ~120ms      | **83ms**   | **~30% faster**  |
| **peak RSS**    | ~225MB      | **140MB**  | **−38%**         |

Parse is still SERIAL (council rule: never combine data-model change with
concurrency); the win here is removing the copy + RSS, not parse threading.
Parallel parse is a separate follow-up now unblocked by `Send`-able indices.

## Gates (all pass)

- **Byte-identical:** all 4 corpora sha256 unchanged vs the p2-buildid baseline.
- **Determinism:** ripgrep links identically at threads=0 and threads=1.
- **Correctness:** linked ripgrep runs (`ripgrep 15.1.0`, exit 0); `--icf` link
  runs correctly with the new digest FoldKey.
- **Tests:** full `cargo test --workspace` green (same counts as baseline).

## Cumulative vs mold (ripgrep)

3.3× (start) → 2.0× (build-id) → with the link now at 83ms vs mold ~8ms the
wall-clock ratio is re-measured in the next bench pass. The two shipped phases
(build-id + zero-copy) together cut peony's own cold-link time ~177ms → ~83ms
(−53%) and RSS 225MB → 140MB.
