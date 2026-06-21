# peony

A fast, **incremental** ELF linker for x86-64 Linux, written in Rust. Drop-in
compatible with the `ld`/`cc` command line, so it can be the final linker for
`rustc`- and `gcc`-based toolchains.

**My thesis.** You edit one file and rebuild a thousand times a day but every
linker relinks the whole program from scratch, every time (mold and lld are
fast, but they still redo all of it). peony doesn't. A one-file change relinks in
**~19 ms** one-shot, or **~3–4 ms** with a resident daemon, versus a ~32 ms full
link and the result is **byte-for-byte identical to a full link**, every time.

## How

A relink does work proportional to what *changed*, not to the size of the program:

| change | what peony does | one-`.o` relink (402-object program) |
|---|---|---|
| nothing | reuse the output untouched (stat cache) | instant |
| one file | re-parse only that `.o`, reuse the cached layout + symbol table, patch its bytes in place | **~19 ms** |
| + daemon | keep the layout + symbols resident in RAM | **~3–4 ms** |

The fast path is taken only when peony's cache gates establish the output should
be identical: same sizes, same symbols, same GOT/PLT/TLS demand, no
`--gc-sections`/`--icf`, no archive change. Anything else falls back to a full
link. **peony never serves stale bytes**; that is the one rule everything else
bends to.

Incremental is on by default. For the daemon, `export PEONY_DAEMON=1` and peony
spawns and reuses one automatically.

## Correctness Story

The real correctness gate is differential testing against the system linker:
peony links real objects and checks byte identity against a full link across the
incremental, daemon, and thread-count paths.

`rocq-tests/` holds machine-checked Rocq/Coq model proofs. They are useful
specifications for linker invariants, but they are not whole-program
verification of the Rust implementation. Public verification wording is scoped
by [`docs/VERIFICATION_CLAIMS.md`](docs/VERIFICATION_CLAIMS.md), with the
machine-readable source in
[`docs/VERIFICATION_CLAIMS.json`](docs/VERIFICATION_CLAIMS.json). That table
separates model-only results, bridge-tested correspondences, theorem bridges,
and narrow implementation-verified surfaces.

- Rocq theorem bridges and assumption audits:
  [`docs/THEOREM_TO_RUST_BRIDGES.md`](docs/THEOREM_TO_RUST_BRIDGES.md) maps
  theorem families to Rust surfaces, tests, trusted boundaries, and claim
  scopes. `docs/verification-assumptions/` stores the `Print Assumptions`
  outputs for theorem-backed public claims.
- Rust bridge and differential tests:
  `peony-cache/tests/partial_relink.rs`,
  `peony-emit::input_work` unit tests,
  `peony-reloc` byte-formula tests,
  `peony-layout/tests/layout_gc_bridge.rs`,
  `peony-layout/tests/icf_bridge.rs`, and
  `peony-symbols/tests/symbol_bridge.rs` exercise the matching concrete planner,
  emit-range, relocation-byte, layout/GC/ICF, and symbol-resolution surfaces.
  `peony/tests/incremental.rs` checks the scoped partial-emit byte identity path.

The current narrow claims do not cover parser correctness, dynamic-loader
behavior, unsupported ELF features, all relocation encodings, every mmap write
in every mode, or whole-linker correctness. The full task breakdown for the
verification work is tracked in
[`docs/IMPLEMENTATION_VERIFICATION_TASKS.md`](docs/IMPLEMENTATION_VERIFICATION_TASKS.md).

## Use it

```sh
cargo build --release

peony -o app crt1.o main.o … -lc     # invoked like ld
PEONY_DAEMON=1 cargo build           # sub-5 ms relinks, automatically
```

Point `rustc` at it with `-C linker=/path/to/peony -C linker-flavor=ld`.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
