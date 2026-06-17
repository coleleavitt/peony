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
