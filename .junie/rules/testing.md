# Peony Testing Rules

- Parser, symbol, layout, relocation, emit, TLS, dynamic-linking, and
  incremental-cache changes need focused regression coverage.
- Prefer integration tests that link real object files when practical.
- Compile before trusting test results: `cargo check --workspace` or the
  relevant build command should pass.
- Run all tests relevant to the modified crate and downstream behavior.
- Use `cargo test -p peony --test <test_name>` for focused integration tests,
  then broaden when shared crates or pipeline behavior changed.
- Benchmark changes must separate correctness gating from timing collection.
- Rocq/Coq proof changes are validated with `make` from `rocq-tests/`.
- Missing `cc`, `c++`, `as`, `ar`, `ld`, `readelf`, or `objdump` is an
  environment failure to report, not a reason to weaken the test.
