# Peony Rust Code Style

- Keep Rust code on edition 2021 and follow the surrounding file style.
- Prefer domain modules under the owning crate over root-level helper files.
- Use explicit enums, newtypes, typed IDs, and phase-specific types for linker
  invariants when raw primitives or booleans would be ambiguous.
- Keep public APIs narrow and re-export intentionally from crate or module hubs.
- Prefer borrowed data and arena/input models already used in the workspace over
  clone-heavy reshaping.
- Add dependencies only when existing workspace patterns cannot cover the need.
- Use `thiserror`/`anyhow` consistently with the surrounding crate.
- Format Rust changes with `cargo fmt --all` when Rust files are edited.
