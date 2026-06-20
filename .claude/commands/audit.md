---
description: Audit a Peony change for crate boundaries, ELF correctness, tests, and validation gaps.
---

Audit the current Peony work for architecture and correctness.

Return sections for:

- `Architecture`: whether changes sit in the right crate/module boundary.
- `Correctness risks`: parser, symbol, layout, relocation, emit, TLS, dynamic,
  cache, or benchmark risks.
- `Tests`: missing or sufficient focused coverage.
- `Validation`: exact commands to run or already-run evidence.
