# Peony Claude Instructions

## Mission

Peony is an experimental Rust ELF linker for x86-64 Linux. It is intended to be
fast, incremental, and compatible enough with the `ld`/`cc` command line to be
used as the linker for Rust and GCC-based toolchains.

Linker behavior is correctness-sensitive. A fast link that emits a subtly wrong
ELF file is a bug, so preserve binary semantics before optimizing throughput.

## Architecture First

Peony is a Cargo workspace with focused crates rather than one flat linker
crate:

- `peony`: driver binary, CLI parsing, response-file handling, compatibility
  handoffs, and the top-level link pipeline.
- `peony-object`: ELF/archive parsing and input models such as objects,
  sections, symbols, shared-object metadata, and `.eh_frame` inputs.
- `peony-symbols`: global symbol resolution, weak/import handling, COMDAT, and
  archive-style extraction decisions.
- `peony-layout`: section and segment layout, address assignment,
  `--gc-sections`, TLS placement, and identical code folding.
- `peony-reloc`: relocation scanning and application for static and dynamic
  relocations.
- `peony-emit`: final ELF serialization to disk.
- `peony-cache`: incremental-cache fingerprints, manifests, and reused-output
  metadata.
- `peony-prof`: internal phase counters, tracing, RSS snapshots, and reporting.
- `peony-bench`: benchmark and micro-benchmark support.

Keep these ownership boundaries clear. Parsing belongs in `peony-object`, global
symbol policy in `peony-symbols`, placement decisions in `peony-layout`, byte
patching in `peony-reloc`, file serialization in `peony-emit`, and orchestration
in `peony`. Prefer domain-shaped modules under the owning crate over adding
root-level helper files.

The native link pipeline is roughly:

1. Parse command-line inputs, response files, libraries, and compatibility
   flags.
2. Load objects, archives, and shared libraries into typed input models.
3. Resolve symbols, archive members, weak/import behavior, COMDAT, and dynamic
   export/import needs.
4. Scan relocations and synthesize needed GOT, PLT, dynamic relocation, and
   linker-defined symbol state.
5. Lay out sections and segments, including GC, TLS, ICF, and build-id data.
6. Apply relocations against the chosen layout.
7. Emit the final ELF file and optional incremental-cache metadata.

## Current State

- The project already has a repository-wide `AGENTS.md` with the portable agent
  rules. Keep it compatible with this file.
- Project-local Junie skills live in `.junie/skills/` and include imported Rust,
  tracing, error-handling, handoff, and artifact-export skills.
- `MEMORY.md` is currently an index into local memory records; do not replace it
  with scratch notes unless the user explicitly asks.
- There are many benchmark corpora and imported test fixtures. Avoid generated
  churn in `peony/tests/lld/`, `bench/baselines/`, and captured corpora unless
  the task is specifically about those assets.

## Working Rules

- Keep Rust on edition 2021 and follow surrounding style.
- Prefer existing workspace patterns over new abstractions or dependencies.
- Use typed IDs, enums, newtypes, and explicit phase/state names where they
  clarify ELF/linker invariants.
- Avoid broad rewrites. Make targeted changes that preserve crate boundaries.
- Treat parser, symbol, layout, relocation, emit, TLS, dynamic-linking, and
  incremental-cache changes as high-risk until tested with real linked objects.
- Do not assume portability outside x86-64 Linux unless the task is explicitly
  about portability.
- Do not delete or normalize large imported fixtures or captured corpora unless
  the user specifically requests it.

## Validation

Run commands from the repository root unless noted otherwise.

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

Integration tests may invoke system tools such as `cc`, `c++`, `as`, `ar`,
`ld`, `readelf`, and `objdump`; missing host tools can explain failures.

## Project Claude and Junie Layout

- `AGENTS.md`: shared repository instructions for all coding agents.
- `CLAUDE.md`: this expanded architecture-first project guide.
- `.junie/AGENTS.md`: Junie-specific project guidance that keeps Junie aligned
  with `AGENTS.md` and this file.
- `.junie/skills/`: project-local skills imported for this checkout.
- `.junie/agents/`: focused Junie subagents for Peony research and verification.
- `.junie/commands/`: reusable Junie command prompts for audit and repro work.
- `.junie/rules/`: focused rule files for Rust style, security, and testing.
- `.claude/`: compatibility entrypoints and helper files for Claude-style tools.
