# Peony Junie Instructions

These instructions apply to the Peony checkout rooted at this repository. They
are Junie-specific but must remain compatible with the root `AGENTS.md` and
`CLAUDE.md`.

## Mission

Peony is an experimental Rust ELF linker targeting x86-64 Linux. Preserve ELF
correctness before performance claims: wrong output binaries, broken dynamic
linking metadata, bad TLS placement, or invalid relocations are correctness
bugs even if the link is fast.

## Architecture Boundaries

- Use `peony` for CLI parsing and top-level orchestration.
- Use `peony-object` for object/archive/shared-library parsing and input models.
- Use `peony-symbols` for global symbol resolution, weak/import handling, and
  COMDAT decisions.
- Use `peony-layout` for section/segment placement, GC, TLS, and ICF.
- Use `peony-reloc` for relocation scanning and patching.
- Use `peony-emit` for final ELF serialization.
- Use `peony-cache` for incremental-cache manifests and fingerprints.
- Use `peony-prof` for stats, tracing, and internal profiling.
- Use `peony-bench` and `bench/` for benchmark work.

When adding Rust code, prefer domain-shaped modules under the owning crate over
flat helper files. Do not introduce a new crate unless dependency direction,
reuse, compile-time isolation, or public versioning justifies it.

## Working Rules

- Follow Rust edition 2021 and the surrounding module/file style.
- Prefer existing patterns and small targeted changes.
- Add or update focused regression tests for parser, symbol, layout,
  relocation, emit, TLS, dynamic-linking, or incremental-cache changes.
- Link real object files in tests when practical.
- Avoid generated churn in `peony/tests/lld/`, `bench/baselines/`, and captured
  corpora unless explicitly requested.
- Keep secrets and machine-local settings out of repository files.
- If host toolchain programs are missing, report that as an environment issue
  rather than weakening tests.

## Useful Commands

```sh
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo test -p peony --test <test_name>
cargo bench -p peony-bench
```

Benchmark commands:

```sh
bench/bench.sh --runs 20 --warmup 5
bench/bench.sh --strict-env --pin 0-7 --threads 8 --runs 20 --warmup 5
```

Formal proof suite:

```sh
cd rocq-tests
make
```

## Local Scaffolding

- Use `.junie/skills/` for project-local skills.
- Use `.junie/agents/peony-researcher.md` for read-only architecture/source
  investigation.
- Use `.junie/agents/peony-verifier.md` for build, test, and validation passes.
- Use `.junie/commands/audit.md` for architecture/correctness audits.
- Use `.junie/commands/repro.md` for bug reproduction workflows.
- Use `.junie/rules/` for focused supporting guidance.
