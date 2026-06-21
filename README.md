# peony

An experimental ELF linker written in Rust, targeting x86-64 Linux. Peony is
designed to be fast, incremental, and drop-in compatible with the standard
`ld`/`cc` command line so it can be used directly as the linker for `rustc` and
`gcc`-based toolchains.

Its headline is the **edit–rebuild loop**: a one-file change relinks in **~19 ms**
as a one-shot, or **~3–4 ms** with a resident daemon — vs a ~32 ms full link
(402-object program), always **byte-identical to a full link**. Incremental is on
by default; the daemon is one `PEONY_DAEMON=1` away.

## Features

- **ELF x86-64** — produces `ET_EXEC`, `ET_DYN` (PIE), and shared-object
  (`-shared`) outputs
- **Shared libraries** — full `ET_DYN` shared object support: `.dynsym`,
  `.plt`, `.got.plt`, `DT_SONAME`, `--version-script` for cdylib exports
- **TLS** — General-Dynamic, Local-Dynamic, and Initial-Exec TLS models for
  both executables and shared objects
- **GOT / PLT** — synthesised `.got`, `.got.plt`, `.plt`, and `.plt.got`
  sections; IFUNC (`R_X86_64_IRELATIVE`) support
- **Dynamic relocations** — `R_X86_64_RELATIVE`, `GLOB_DAT`, `JUMP_SLOT`,
  `DTPMOD64 / DTPOFF64 / TPOFF64`
- **`--gc-sections`** — mark-and-sweep dead-section elimination rooted at the
  entry point and (for shared objects) all exported symbols
- **COMDAT deduplication** — eliminates duplicate C++ inline/template sections
  across translation units
- **Relocatable output** (`-r`) — native partial linking: merge inputs into one
  `ET_REL` object with combined symbols and kept relocations (COMDAT inputs hand
  off to GNU `ld`)
- **Incremental linking** (on by default) — a multi-tier fast relink that is
  always **byte-identical to a full link** (the non-negotiable gate). A no-change
  relink is reused from a stat cache; a size-stable one-`.o` edit re-parses *only*
  the changed object, reuses the cached layout + symbol table, and patches just
  that object's bytes in place. Any unsafe change (size, layout, symbol set, or
  GOT/PLT/TLS demand) conservatively falls back to a full link rather than serving
  stale bytes. Opt out with `--no-incremental` or `PEONY_INCREMENTAL=0`.
- **Resident daemon** (`--daemon`, or `PEONY_DAEMON=1` to auto-spawn) — holds the
  deserialized layout and symbol table in RAM and serves relinks over a Unix
  socket, so each relink skips the per-invocation library search, layout
  deserialize, and symbol-table rebuild — **~3–4 ms** warm, ~10× faster than
  re-linking from scratch every time. Idle-times-out on its own.
- **Linker-synthesised symbols** — `_end`, `_edata`, `__bss_start`,
  `_GLOBAL_OFFSET_TABLE_`, `__executable_start`, `__dso_handle`, etc.
- **`--defsym`** — define absolute symbols on the command line
- **`--build-id`** — emit a `.note.gnu.build-id` section
- **Parallel** — work-stealing thread pool ([ws-deque]) for parallel section
  scanning and layout
- **`ld`-compatible CLI subset** — compiler-driver noise is tolerated, response
  files are expanded, and known unsupported output-changing flags fail
  explicitly instead of silently producing the wrong binary

[ws-deque]: https://github.com/coleleavitt/ws-deque

## Crate layout

| Crate | Role |
|---|---|
| `peony` | Driver binary — CLI parsing, top-level link pipeline |
| `peony-object` | ELF parsing: `InputObject`, `InputSection`, `InputSymbol`, shared-object metadata |
| `peony-symbols` | Global symbol table — resolution, COMDAT, weak/import handling |
| `peony-layout` | Section/segment layout, address assignment, GC, TLS block placement |
| `peony-reloc` | Relocation scanning and application (static + dynamic) |
| `peony-emit` | ELF binary serialisation to disk |
| `peony-cache` | Incremental cache — fingerprinting, manifest read/write |

## Building

```sh
cargo build --release
```

Requires a recent nightly or stable Rust (1.73+).

## Usage

Peony is invoked like `ld`:

```sh
peony -o output input.o [input2.o ...] [-L dir] [-l lib] [flags]
```

### Key flags

| Flag | Description |
|---|---|
| `-o FILE` | Output file (default `a.out`) |
| `-e SYM` | Entry symbol (default `_start`) |
| `-L DIR` | Add library search directory |
| `-l NAME` | Link library `libNAME.{a,so}` |
| `--gc-sections` | Dead-strip unreachable sections |
| `--incremental` | Incremental cache + fast relinks (**on by default**) |
| `--no-incremental` | Disable incremental (also `PEONY_INCREMENTAL=0`) |
| `--daemon` | Run a resident daemon serving sub-5 ms relinks (also `PEONY_DAEMON=1` to auto-spawn) |
| `--cache-report FILE` | Write JSON explaining cache reuse, partial relink, or full-emit fallback |
| `--build-id` | Emit `.note.gnu.build-id` |
| `-pie` / `-no-pie` | Position-independent executable |
| `-shared` | Produce a shared object (`ET_DYN`) |
| `-soname NAME` | Set `DT_SONAME` |
| `-dynamic-linker PATH` | Set `PT_INTERP` for dynamic executables |
| `-rpath PATH` / `--enable-new-dtags` | Emit `DT_RUNPATH` (or `DT_RPATH` with `--disable-new-dtags`) |
| `--as-needed` / `--no-as-needed` | Scope `DT_NEEDED` retention for shared libraries |
| `-Bstatic` / `-Bdynamic` | Scope `-l` lookup to archives or shared libraries |
| `--whole-archive` | Include every member of following archives |
| `--start-lib` / `--end-lib` | Treat following object files as lazy archive-style members |
| `--export-dynamic` / `--export-dynamic-symbol` | Export executable symbols into `.dynsym` |
| `--exclude-libs LIST` | Hide archive-provided symbols from dynamic exports |
| `--hash-style=sysv\|gnu\|both` | Select dynamic hash-table style where supported |
| `--no-undefined` / `-z defs` | Reject unresolved symbols in shared-object links |
| `-r` | Produce a relocatable (`ET_REL`) object — native partial linking (GNU `ld` handoff for COMDAT inputs) |
| `--version-script FILE` | Export/localise symbols per version script |
| `--defsym SYM=VAL` | Define an absolute symbol |
| `--threads N` | Worker thread count (0 = auto) |
| `-s` / `--strip-all` | Strip symbol table and debug sections |
| `-S` / `--strip-debug` | Strip debug sections but keep `.symtab` |

### Current limits

Native GCC/LLVM LTO plugin integration is not implemented; actual GCC LTO
objects and LLVM bitcode objects are handed to GNU `ld` so the real plugin can
materialize native code.

Relocatable `-r` (partial linking) is emitted **natively** as an `ET_REL` object
— input sections are merged by name, the symbol tables are combined, and
relocations are kept (not applied) so the result re-links correctly. Inputs that
carry COMDAT groups (typical C++ with inlines/templates) still hand off to GNU
`ld`, which regenerates the `.group` sections native `-r` does not yet rebuild.

## Incremental relinks & the daemon

Peony's edit–rebuild loop has three tiers, each **byte-identical to a full link**
(verified by a `cmp`-vs-full gate across thread counts and an adversarial
relocation sweep):

| Tier | What it does | One-`.o` relink (402-obj program) |
|---|---|---|
| **no-change reuse** | stat-cache hit → reuse the existing output untouched | instant |
| **one-shot fast relink** | re-parse *only* the changed `.o`, reuse the cached layout + symbol table, patch its bytes in place | **~19 ms** (vs ~32 ms full) |
| **resident daemon** | the above, but layout + symbols stay in RAM across invocations | **~3–4 ms** warm |

Anything the fast path can't prove safe (a size/layout change, a new symbol, a
new GOT/PLT/TLS demand, `--gc-sections`/`--icf`, a changed archive/shared lib)
falls back to a full link — peony never serves stale bytes.

Incremental is **on by default**. Turn it off for clean/CI builds that never
relink (and don't want the `<output>.incr/` cache) with `--no-incremental` or
`PEONY_INCREMENTAL=0`.

The daemon gives the sub-5 ms tier. Either run it explicitly:

```sh
peony --incremental -o app … objs…   # one seed link establishes the cache
peony --daemon       -o app … objs… &  # resident server on app.incr/daemon.sock
```

…or just export `PEONY_DAEMON=1` in your dev shell and let peony **auto-spawn**
one: once a cache exists, a relink spawns a detached background daemon (logging
to `<output>.incr/daemon.log`) and delegates to it. The daemon idle-times-out
after `PEONY_DAEMON_IDLE_SECS` (default 300 s), so it cleans up after itself.

### Invoking from rustc/Cargo

Peony is an `ld`-style final linker. When pointing `rustc` directly at Peony,
set `linker-flavor=ld` so `rustc` sends raw linker arguments instead of
compiler-driver (`cc`/`gcc`) arguments:

```sh
RUSTFLAGS="-C linker=/path/to/peony -C linker-flavor=ld -C link-self-contained=no" cargo build
```

Or set it in `.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "/path/to/peony"
rustflags = [
    "-C", "linker-flavor=ld",
    "-C", "link-self-contained=no",
]
```

For a checked-in project recipe, keep the direct-linker form in
`.cargo/config.toml` and avoid changing `RUSTFLAGS` between retries:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "/path/to/peony"
rustflags = [
    "-C", "linker-flavor=ld",
    "-C", "link-self-contained=no",
]
```

If the configured linker is a compiler driver such as `/usr/bin/cc`, `clang`, or
a wrapper script that expects compiler-driver flags, use `linker-flavor=gcc` for
that driver; do not copy that flavor to direct Peony invocations. For example:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "/usr/bin/cc"
rustflags = [
    "-C", "linker-flavor=gcc",
]
```

Peony's incremental cache is **on by default**, so no extra flag is needed to
get fast relinks. To turn it *off* for a CI/clean build, set
`PEONY_INCREMENTAL=0` in the environment (build systems invoke the linker via
`cc`/`ld`, so the env var is easier to inject than a flag). To get the sub-5 ms
daemon tier, export `PEONY_DAEMON=1` and peony auto-spawns one on the first
relink:

```sh
PEONY_DAEMON=1 cargo build   # auto-spawns a resident daemon for sub-5 ms relinks
```

For large projects where you want to tell Cargo-facing tooling what the final
linker did, add a machine-readable cache report. The report path is deliberately
excluded from Peony's cache key so turning diagnostics on does not dirty an
otherwise reusable link:

```sh
RUSTFLAGS="-C linker=/path/to/peony -C linker-flavor=ld -C link-self-contained=no -C link-arg=--cache-report=target/peony-cache-report.json" cargo build
```

Keep that `RUSTFLAGS` value stable between retries. Cargo fingerprints the full
`RUSTFLAGS` string before Peony starts, so changing only the report filename can
still make Cargo/rustc rebuild or relink even though Peony ignores that filename
inside its own incremental cache key.

Use `--stats` when invoking Peony directly if you also want a human-readable
stderr line such as `reused unchanged output`, `partial relink used`, or
`full emit fallback: section ... size changed`. The JSON report has stable
top-level fields for future build-system consumption:

```json
{
  "version": 1,
  "output": "target/debug/app",
  "cache": { "enabled": true },
  "action": "partial_relink",
  "message": "partial relink used: 1 red sections, 8 green sections",
  "sections": {
    "red": [".text"],
    "green": [".rodata"]
  }
}
```

Fallback reports use `action: "full_emit"` and include a stable
`reason.code`, for example `cache_state_unavailable`, `section_size_changed`,
`section_capacity_exceeded`, or `section_virtual_address_changed`.

Cargo decides whether to rerun build scripts or recompile crates from its own
fingerprints before the final linker starts. Peony can skip or patch the final
link once `rustc` invokes it, but it cannot make Cargo treat a dirty crate as
clean. If a retry rebuilds more than expected, inspect Cargo's dirty reasons
first, for example with:

```sh
CARGO_LOG=cargo::core::compiler::fingerprint=trace cargo test -p your-crate
```

## Testing

The test suite links real object files and compares output against reference
linkers (mold corpus, real-C objects, shared-library tests, TLS, relocations):

```sh
cargo test
```

## Profiling & tracing (`peony-prof`)

peony has a built-in profiler so you measure where a link spends its time —
and follow a bug through the pipeline — from *inside* the linker, instead of
guessing with external `strace`/`perf`.

```sh
peony --stats <args>    # phase breakdown table: parse/resolve/scan/layout/emit
peony --trace <args>    # call-flow tree: caller→callee by file:line, + events
```

`--stats` prints each phase's wall-clock, %, span count, byte/item throughput,
hot-path counters, and RSS snapshots with deltas. `--trace` additionally records the nested call flow with
source locations and point events (e.g. `archive-round: round 2: checked 3,
skipped 1, parsed 1, pulled 7, 3 undef left`), so you can see *what happened per
line* — this is how the O(N²) archive fixpoint was found and fixed. `--trace-stack`
adds Rust backtraces to each trace frame/event for deep bug hunts where function
stack and instruction-address context matter. All modes are near-zero cost when off
(a single atomic load short-circuits).

## Benchmarking

peony links real C, C++ (iostream/exceptions/STL), and Rust programs correctly,
and is benchmarked against mold, lld, and GNU ld with a correctness-gated
harness (a fast *wrong* linker is excluded from timing). See
[`bench/BENCHMARKING.md`](bench/BENCHMARKING.md) for methodology and the honest
baseline table.

```sh
bench/capture.sh rust-bin rust-hello /path/to/cargo/project --release
bench/bench.sh --runs 20 --warmup 5
```

Micro-benchmarks of the internal hot paths run under criterion / CodSpeed:

```sh
cargo bench -p peony-bench
```

## Formal verification

`rocq-tests/` holds nine machine-checked Rocq/Coq proofs (zero axioms beyond
functional extensionality). They cover GC reachability, layout congruence,
relocation disjoint-write determinism, symbol-resolution semilattice, and three
results that justify beating a from-scratch linker on the edit–rebuild loop:
incremental-relink soundness + the O(affected) cost bound, parallel-schedule
work–span optimality, and ICF (identical code folding) soundness. `make` in
`rocq-tests/` is the pass/fail oracle.

## License

Licensed under either of [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option.
