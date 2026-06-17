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

| corpus       | inputs | peony  | mold  | lld   | bfd   | peony/mold |
|--------------|-------:|-------:|------:|------:|------:|-----------:|
| hello-c      | 1      | ~15 ms | ~6 ms | ~9 ms | ~8 ms | ~2.6×      |
| hello-cxx    | 1      | ~22 ms | ~9 ms | ~11ms | ~31ms | ~2.4×      |
| rust-hello   | 23     | ~28 ms | ~13ms | ~15ms | ~21ms | ~2.0×      |
| ripgrep      | 419    | ~126ms | ~38ms | ~42ms | ~300ms| ~3.3×      |

**Reading it honestly:** peony links real C, C++ (iostream/exceptions/STL), and
Rust correctly, and on big links already beats GNU ld (bfd). It trails mold by
2–3.3×. The gap is two things, both understood:

1. **Fixed per-link overhead** (small links): allocator + I/O. Largely addressed
   — mimalloc dropped hello-c page-faults 2916→339; header-only ELF
   classification cut redundant reads.
2. **Parallel scaling** (big links): peony currently gets little speedup from
   threads (≈1.1× from 1→24 cores on ripgrep) while mold scales near-linearly.
   This is the open architectural gap — parallel symbol resolution and a
   parallel emit that actually saturates cores (tracked in the task list).

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
