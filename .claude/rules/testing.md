# Peony Testing Rules

- Add focused regression tests for parser, symbol, layout, relocation, emit,
  TLS, dynamic-linking, or incremental-cache changes.
- Prefer real object files for linker behavior tests.
- Compile before trusting test results.
- Run relevant downstream tests when shared crates or pipeline behavior changes.
- Validate Rocq/Coq changes with `make` in `rocq-tests/`.
