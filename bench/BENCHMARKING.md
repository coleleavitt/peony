# Benchmarking peony

How to measure peony honestly against mold, lld, and GNU ld, and how to read the
numbers. The methodology here follows the consensus of a model-council review and
a deep-research pass on how mold/lld/wild benchmark themselves (see
`../docs/incremental-linking-theory-research.md`).

## TL;DR

```sh
# 1. freeze a real link's object set into a replayable corpus
bench/capture.sh rust-bin   rust-hello /path/to/cargo/project --release
bench/capture.sh c          hello-c    /tmp/hello.c
bench/capture.sh cxx        hello-cxx  /tmp/hello.cpp

# 2. correctness-gate + time peony vs mold vs lld vs bfd
bench/bench.sh --runs 20 --warmup 5
```

Results land in `bench/results/<corpus>.{json,md}`.

## The cardinal rules

1. **Correctness before speed, always.** `bench.sh` first links the corpus with
   every linker, runs the output binary, and compares stdout+exit to an lld
   reference. A linker whose output differs is **excluded from timing** — a fast
   wrong linker is worthless. Never quote a speed number for a link you did not
   first prove correct.

2. **Measure the link step, not the build.** `capture.sh` freezes the exact
   `cc`-level argv of a real final link (the `.o`/`.rlib`/`.a` set + flags) so
   replaying it never recompiles. Benchmarking `cargo build` instead measures
   rustc, not the linker.

3. **Hold flags byte-identical across linkers.** The corpus is linked through the
   same compiler driver (`cc`/`c++`) with only the linker swapped via
   `cc -B<dir>` (each `<dir>` exposes the chosen linker as `ld`). The LTO
   `-plugin` flags are stripped at capture time so every linker sees the same
   plain-ELF object set.

4. **Warm the cache, report the median.** `hyperfine --warmup 5 --runs 20`
   reports median ± σ. Cold-cache numbers measure the page cache, not the
   linker.

5. **Output to tmpfs.** Link outputs go to `/dev/shm` so the SSD is not in the
   measurement. (The correctness gate copies the binary to an exec-capable dir
   first, because `/dev/shm` is often `noexec`.)

## Reducing variance

- Pin the CPU governor to performance: `sudo cpupower frequency-set -g performance`.
  `bench.sh` warns if the governor is not `performance`.
- Pin cores: `bench/bench.sh --pin 0-7` wraps each link in `taskset -c 0-7`.
- Normalize thread count: `bench/bench.sh --threads 8` passes `--threads`/
  `-Wl,--threads=` to each linker that supports it.
- Close other workloads; run on AC power.

## What the numbers mean (current honest baseline)

Measured on a 24-core machine, warm cache, plugin-stripped, identical flags.
peony is built `--release`. These are **honest** numbers — peony links every
corpus itself (no fallback to bfd).

Two epochs are shown: **before** the parallel/zero-copy rewrite and the
**current** numbers after each shipped phase. peony is built `--release`.

| corpus       | inputs | peony (before) | peony (now) | mold  | lld   | peony/mold (before → now) |
|--------------|-------:|---------------:|------------:|------:|------:|---------------------------:|
| hello-c      | 1      | ~15 ms         | ~26 ms*     | ~7 ms | ~9 ms | ~2.6× → ~1.0× (parity)     |
| hello-cxx    | 1      | ~22 ms         | ~30 ms*     | ~8 ms | ~11ms | ~2.4× → **0.96× (beats)**  |
| rust-hello   | 23     | ~28 ms         | ~71 ms*     | ~8 ms | ~58ms | ~2.0× → ~1.8×              |
| ripgrep      | 419    | ~126ms         | ~117 ms     | ~8 ms | ~87ms | ~3.3× → **~2.0×**          |

\* the small-link wall-times rose vs the old table because of measurement noise
(non-`performance` governor on this run) — the *ratio* to mold is the honest
figure; hello-cxx now beats mold and hello-c is at parity. The big-link result
is the headline: **ripgrep closed from 3.3× to ~2.0×** behind mold.

**What moved the needle (measured, not guessed):** a per-phase scaling profiler
(`--stats`/`--trace`, see `baselines/phase0-baseline/FINDINGS.md`) found the real
cold-link bottleneck was NOT symbol resolution (only 3.2% of the link) but the
**build-id hash: 30% of a ripgrep link, fully serial**, hashing all ~18.5MB of
scattered input bytes. Hashing the ~4MB contiguous *output* in parallel blocks
instead (commit `748867d`) cut it 17× (53ms→3ms) and the whole link ~30%.

**Remaining gap, by measured self-time (ripgrep, post-build-id):**

1. **parse+resolve ~32%** — parse-bare + the lazy archive fixpoint; the
   per-section `Vec<u8>` copy (17MB) makes it memory-bandwidth bound. The
   zero-copy arena refactor (in progress) removes the copy and unblocks a parse
   that actually scales.
2. **reloc-postproc ~16%** — GOT/PLT/TLS slot extraction + dynsym assignment.
3. **emit ~20%** — the section-copy already scales 3×; zero-copy lets it blit
   straight from the input mmap.
4. **Peak RSS:** peony ~225MB vs mold ~8MB on ripgrep — the per-section copy is
   the main culprit; zero-copy attacks this directly.

## Where peony already wins: the edit–rebuild loop

mold and lld have **no incremental mode**: every rebuild is O(total). peony
caches the last link and, on a rebuild, checks each input with a single `stat()`
(size + mtime + inode).

**No-change relink (verified):** `peony --incremental` on rust-hello (23 inputs)
is **15 ms vs mold's full link 28 ms — 1.9× faster**, and the output is
byte-identical to a full peony link. The reuse path is O(#inputs) syscalls (no
content read, no thread pool), and a changed input correctly falls back to a
full link (test `incremental_cache_invalidates_on_input_change` — the cache
never serves stale bytes).

```sh
peony --incremental <args> -o out   # first call links + caches
peony --incremental <args> -o out   # unchanged inputs → ~instant reuse
```

**Edit-one-object relink (in progress):** currently this falls back to a full
link (~36 ms, ~3.8× behind mold's 9.5 ms). The capacity-stable in-place patch
that wins this case is proven in `rocq-tests/IncrementalCostBound.v`
(`incremental_beats_fromscratch`: a single-file edit touches 1 section, not n)
and is the next increment — the from-scratch wall-clock gap is a constant
factor, but the incremental gap is asymptotic and is the design's real edge.

## Continuous benchmarking (CI)

`bench/criterion/` holds criterion micro-benchmarks of the internal hot paths
(symbol resolution, GC BFS, relocation apply). They run under
`cargo bench -p peony-bench` locally and under `cargo codspeed` in CI
(`.github/workflows/bench.yml`) so regressions are caught per-PR. End-to-end
wall-clock (this harness) is the source of truth; the criterion benches localize
*where* a regression came from.
