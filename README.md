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
machine-checked Rocq/Coq proofs (`make` is the oracle; zero axioms beyond
functional extensionality). A few of the load-bearing ones:

**Byte-identity.** A "green" (unchanged) section renders byte-for-byte the same,
which is what licenses skipping it:

```coq
Theorem green_is_byte_stable :
  forall s s' m,
    s_offset s = s_offset s' ->          (* same place *)
    s_content s = s_content s' ->        (* same bytes  *)
    forall a, render_section s' m a = render_section s m a.
```

**Cost is O(affected), not O(program).** A relink's cost equals the number of
changed sections — and a from-scratch link's cost grows without bound while a
one-edit relink stays at 1:

```coq
Theorem incremental_cost_eq_num_changed :
  forall old new, incremental_cost old new = num_changed old new.

Theorem incremental_beats_fromscratch :
  forall n, exists old new,
    length new = S n  /\  incremental_cost old new = 1  /\  fromscratch_cost new = S n.
```

**Parallel emit is deterministic.** Relocations that touch disjoint bytes may be
applied in any order, on any number of threads, and produce identical memory:

```coq
Corollary parallel_reloc_deterministic :
  forall sched1 sched2 m,
    Permutation sched1 sched2 ->
    pairwise_disjoint sched1 ->
    apply_all sched1 m = apply_all sched2 m.
```

**ICF is sound.** Folding identical functions to one copy preserves every call's
result, and only moves addresses where they can't be observed:

```coq
Theorem icf_observationally_equivalent :
  forall P (F : fold_map P),
    address_safe P F ->
      (forall f, call_result P (rep F f) = call_result P f) /\
      (forall f g, observably_compared P f g ->
         addr_eq_after P F f g = addr_eq_before f g).
```

The rest cover parallel-schedule work–span optimality (within 2× of the Brent
bound), GC reachability, layout page-congruence, and the symbol-resolution
semilattice. And the byte-identity guarantee is enforced in CI too: a
`cmp`-against-a-full-link test on every relink path, across thread counts, plus
an adversarial relocation sweep.

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
