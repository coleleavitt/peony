---
description: Build a focused reproduction workflow for a Peony linker bug.
---

Create a focused reproduction plan for the described Peony issue.

Prefer a small real-object test over a synthetic unit-only assertion when the
bug involves ELF parsing, symbol resolution, layout, relocations, TLS, dynamic
linking, emitting, or incremental cache behavior.

Plan format:

1. `Inputs`: minimal source/object/archive/shared-library setup.
2. `Command`: exact `cc`, `as`, `ar`, `peony`, `readelf`, or `objdump` command.
3. `Expected`: what a correct linker should produce.
4. `Actual`: how the bug is observed.
5. `Regression`: where the test should live and what it should assert.
6. `Validation`: commands to run after the fix.

Call out missing host tools separately from linker failures.
