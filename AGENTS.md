# Repository Instructions

## Scope

These instructions apply to the entire repository rooted here.

## Project Context

Peony is an experimental Rust ELF linker targeting x86-64 Linux. The workspace is
split into focused crates:

- `peony`: driver binary, CLI parsing, and top-level link pipeline.
- `peony-object`: ELF/archive parsing and input models.
- `peony-symbols`: global symbol resolution, weak/import handling, and COMDAT.
- `peony-layout`: section/segment layout, GC, TLS placement, and ICF.
- `peony-reloc`: relocation scanning and application.
- `peony-emit`: ELF output serialization.
- `peony-cache`: incremental cache support.
- `peony-prof`: internal stats and tracing support.
- `peony-bench`: micro-benchmarks for hot paths.

The repository also includes end-to-end benchmark scripts under `bench/`, formal
Rocq/Coq proofs under `rocq-tests/`, and a large imported lld ELF test corpus
under `peony/tests/lld/`.

## Development Guidance

- Treat linker behavior as correctness-sensitive. For parser, symbol, layout,
  relocation, emit, TLS, dynamic-linking, or incremental-cache changes, add or
  update focused regression tests that link real object files when practical.
- Prefer existing workspace patterns over new abstractions or dependencies.
- Keep Rust code on edition 2024 and format with standard `rustfmt`.
- Avoid broad rewrites, generated churn, or fixture normalization in
  `peony/tests/lld/`, `bench/baselines/`, or captured corpora unless the task is
  specifically about those assets.
- The integration tests invoke system toolchain programs such as `cc`, `c++`,
  `as`, `ar`, `ld`, `readelf`, and `objdump`; failures may reflect missing host
  tools rather than Rust compilation failures.
- Peony currently targets x86-64 Linux. Do not assume tests or produced binaries
  are portable to other architectures or operating systems unless the change is
  explicitly about portability.

## Useful Commands

Run from the repository root unless noted otherwise.

```sh
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo test -p peony --test <test_name>
cargo bench -p peony-bench
```

For benchmark methodology and publishable end-to-end numbers:

```sh
bench/bench.sh --runs 20 --warmup 5
bench/bench.sh --strict-env --pin 0-7 --threads 8 --runs 20 --warmup 5
```

For the formal proof suite:

```sh
cd rocq-tests
make
```
