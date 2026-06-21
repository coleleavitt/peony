# Shared input pool — cross-link parsed-object reuse

**Status:** design / not started. Companion: [`NATIVE_LTO.md`](NATIVE_LTO.md),
[`INCREMENTAL_FRONTEND.md`](INCREMENTAL_FRONTEND.md).

## Goal

Many links consume the *same* input objects — the archive members of `libstd`,
`serde`, `tokio`, and friends. Within a single `cargo test` run, dozens of test
binaries link against an identical set of dependency rlibs; across projects, the
overlap is even larger. Today peony **re-parses every input on every link**, and
parse+resolve is ~40 % of a link, parse-dominated.

The idea: parse each shared input object **once**, keyed by content, and reuse
the parsed result across every link that consumes it. This is the natural step
past the per-output incremental daemon — "parse `libstd` once, reuse it for the
50th binary" — and it sidesteps the deserialize-cost trap we already measured
(blob-deserialize ≈ re-parse), because the parsed objects live **resident in
RAM**, not on disk.

## What is cached — and what is not

- **Cached:** the immutable, *link-agnostic* parse of one input object — section
  bytes/refs, raw symbol names + `st_*`, relocations, COMDAT groups,
  `.eh_frame` — keyed by content fingerprint.
- **Not cached:** global symbol resolution (depends on the *whole* input set, so
  it's inherently per-link), per-link name interning, GOT/PLT/layout. Resolution
  is re-run every link; it's cheaper than parse, and reusing it would be unsafe.

The dividing line is exactly "everything a single input determines on its own"
vs. "everything that depends on the other inputs."

## Key / invalidation

`fingerprint = blake3(content)` for archive members (peony-cache already computes
per-member digests), or `(canonical path, mtime_ns, size, dev/ino)` for whole
files when a content hash is too costly. **Content-keyed ⇒ a rebuilt rlib gets a
new key automatically**, so the pool can never serve a stale parse — the same
non-negotiable rule as the rest of peony's incremental machinery.

## The engineering crux: a link-agnostic parse

peony-object currently parses straight into an `InputArena` with per-link name
interning and per-link object ids. That representation is *not* reusable across
links. So:

1. Split parse into two stages:
   - **`RawParse`** — owned, link-agnostic: section data, *raw* `&str`/`String`
     symbol names, relocations, COMDAT keys, eh-frame. No object ids, no interned
     handles, no resolved addresses.
   - **`intern_into(&RawParse, &mut InputArena) -> InputObject`** — the per-link
     step that interns names and assigns ids.
2. The pool stores `Arc<RawParse>`. On a hit, the link calls `intern_into`;
   re-interning is far cheaper than re-parsing (no ELF header walk, no
   decompression, no reloc decode).

This split is the whole risk surface. The correctness gate (below) is that
`intern_into(parse_raw(f))` produces a link **byte-identical** to today's direct
parse — proving `RawParse` carries nothing link-specific.

## Where it lives

Extend the existing daemon (`peony/src/daemon.rs`) with a **second** cache,
independent of the per-output layout cache it already holds:

```
pool: RwLock<HashMap<Fingerprint, Arc<RawParse>>>   // or dashmap
      + single-flight per key (concurrent misses parse once)
      + size-bounded LRU (libstd members are tens of MB; cap e.g. 2–4 GB)
```

A driver invocation asks the daemon for each input by fingerprint: hit → reuse;
miss → parse and insert. Parallel `cargo` runs many peony processes at once, so
centralizing the pool in the daemon is what makes the reuse cross-process. A
non-daemon link can still dedup *within* its own invocation (P2), but the
cross-link win needs the resident daemon.

## Concurrency

- The pool is read-mostly and hammered by parallel links; `dashmap` or a sharded
  `RwLock` keeps contention low.
- **Single-flight:** two links missing the same key must parse it once, not
  twice — gate inserts on a per-key once-cell / in-flight set.
- **Mmap lifetime:** if `RawParse` borrows from an mmap of the source file, the
  daemon must keep that mapping alive for the entry's lifetime. Simpler and
  safer: **own** the bytes for archive members (copy on insert), mmap only whole
  files the daemon can hold open.

## Phasing

- **P1 — the split.** Land `RawParse` + `intern_into`. Gate: link a program both
  the old way and via `intern_into(parse_raw(...))`; `cmp` byte-identical across
  thread counts. No caching yet — this proves the representation is clean.
- **P2 — in-process pool.** Dedup repeated members within a *single* link (an
  archive pulled in by two `-l` flags, the same rlib referenced twice). Small,
  fully safe, exercises the cache plumbing.
- **P3 — daemon-resident global pool.** Fingerprint request/response protocol,
  LRU bound, single-flight, eviction. The cross-link/cross-project win.
- **P4 — measure.** A workspace with many binaries + shared deps (`cargo test`);
  report parse time elided and end-to-end wall-clock.

## Risks

- **Memory.** Bound + LRU from the start; libstd alone is large. Report what gets
  evicted (no silent unbounded growth).
- **Correctness.** Content-keyed makes staleness impossible; the P1 `cmp` gate
  makes link-specificity leaks impossible. Those two together are the safety
  argument — keep both.
- **Versioning.** The `RawParse` layout is an in-RAM ABI between driver and
  daemon of the *same build*; the daemon already keys on a version/manifest, so a
  peony rebuild must invalidate the pool (reuse the existing daemon version
  guard).

## Validation

- `cmp`: every relevant fixture linked with the pool on vs. a fresh parse →
  byte-identical (reuse the full-link `cmp` gate, run with the pool enabled).
- Bench: `cargo test` on a multi-binary workspace; measure parse time saved and
  the hit rate on shared dep members.
