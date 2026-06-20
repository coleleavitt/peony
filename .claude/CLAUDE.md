# Peony Claude Workspace Entry

Use the root `CLAUDE.md` as the primary architecture guide and keep it
compatible with `AGENTS.md`.

Quick orientation:

- Peony is a Rust ELF linker for x86-64 Linux.
- Correctness-sensitive areas include object parsing, symbols, layout,
  relocations, emitting, TLS, dynamic linking, and incremental cache behavior.
- Keep changes inside the owning crate boundary.
- Prefer focused real-object regression tests for linker behavior.
- Use `.claude/agents/`, `.claude/commands/`, and `.claude/rules/` as helper
  scaffolding; the root files remain authoritative.
