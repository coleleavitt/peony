# Peony Rust Code Style

- Keep Rust on edition 2024 and follow surrounding style.
- Respect crate boundaries: parse in `peony-object`, symbols in
  `peony-symbols`, layout in `peony-layout`, relocation logic in `peony-reloc`,
  serialization in `peony-emit`, orchestration in `peony`.
- Prefer domain modules, explicit enums, newtypes, typed IDs, and phase-specific
  types for linker invariants.
- Avoid broad rewrites and new dependencies unless clearly justified.
