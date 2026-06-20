# Peony Security and Safety Rules

- Do not commit secrets or machine-local credentials.
- Treat malformed ELF output as a correctness and potential security issue.
- Avoid destructive cleanup and fixture churn.
- Do not weaken, skip, ignore, or delete failing tests to force a pass.
