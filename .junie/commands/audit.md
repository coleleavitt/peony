---
description: Audit a Peony change for architecture, ELF correctness, tests, and validation gaps.
---

Audit the current Peony work with an architecture-first lens.

Check:

1. Whether each touched file belongs to the correct crate boundary.
2. Whether linker invariants are encoded in types, explicit phases, validators,
   or focused tests instead of prose-only assumptions.
3. Whether parser, symbol, layout, relocation, emit, TLS, dynamic-linking, or
   incremental-cache behavior needs a real-object regression test.
4. Whether imported fixtures, benchmark baselines, or captured corpora were
   changed unnecessarily.
5. Which validation commands should be run before submission.

Return a concise report with:

- `Architecture`: crate/module boundary findings.
- `Correctness risks`: concrete ELF/linker risks.
- `Tests`: missing or sufficient coverage.
- `Validation`: exact commands and current status if already run.
