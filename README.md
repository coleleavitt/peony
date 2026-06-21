# peony

A fast, **incremental** ELF linker for x86-64 Linux, written in Rust. Drop-in
compatible with the `ld`/`cc` command line, so it can be the final linker for
`rustc`- and `gcc`-based toolchains.

**The thesis.** You edit one file and rebuild a thousand times a day — but every
linker relinks the whole program from scratch, every time (mold and lld are
fast, but they still redo all of it). peony doesn't. A one-file change relinks in
**~19 ms** one-shot, or **~3–4 ms** with a resident daemon — versus a ~32 ms full
link — and the result is **byte-for-byte identical to a full link**, every time.

## How

A relink does work proportional to what *changed*, not to the size of the program:

| change | what peony does | one-`.o` relink (402-object program) |
|---|---|---|
| nothing | reuse the output untouched (stat cache) | instant |
| one file | re-parse only that `.o`, reuse the cached layout + symbol table, patch its bytes in place | **~19 ms** |
| + daemon | keep the layout + symbols resident in RAM | **~3–4 ms** |

The fast path is taken only when peony can *prove* the output is identical — same
sizes, same symbols, same GOT/PLT/TLS demand, no `--gc-sections`/`--icf`, no
archive change. Anything else falls back to a full link. **peony never serves
stale bytes**; that is the one rule everything else bends to.

Incremental is on by default. For the daemon, `export PEONY_DAEMON=1` and peony
spawns and reuses one automatically.

## Proofs

The edit–rebuild claim isn't only measured — it's *proved*. `rocq-tests/` holds
machine-checked Rocq/Coq proofs (zero axioms beyond functional extensionality)
for the load-bearing results: incremental-relink **soundness** and its
**O(affected)** cost bound, parallel-schedule work–span optimality, relocation
disjoint-write determinism, GC reachability, layout congruence, symbol-resolution
semilattice, and ICF soundness.

And the byte-identity guarantee is enforced, not asserted: a `cmp`-against-a-full-link
test runs on every relink path, across thread counts, plus an adversarial
relocation sweep.

## Use it

```sh
cargo build --release

peony -o app crt1.o main.o … -lc     # invoked like ld
PEONY_DAEMON=1 cargo build           # sub-5 ms relinks, automatically
```

Point `rustc` at it with `-C linker=/path/to/peony -C linker-flavor=ld`.

peony is a real linker, not a toy: PIE and shared objects, TLS (GD/LD/IE),
GOT/PLT/IFUNC, dynamic relocations, `--gc-sections`, COMDAT dedup, ICF, version
scripts, native `-r` partial linking, and build-ids. `peony --help` lists the
flags; `peony --stats` / `--trace` profile a link from the inside;
[`bench/BENCHMARKING.md`](bench/BENCHMARKING.md) has the correctness-gated
numbers. LTO/bitcode objects (and `-r` with COMDAT) are handed to GNU `ld`.

## Crates

`peony` (driver) · `peony-object` (parse) · `peony-symbols` (resolve) ·
`peony-layout` (layout / GC / TLS) · `peony-reloc` (relocations) ·
`peony-emit` (serialize) · `peony-cache` (incremental).

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
