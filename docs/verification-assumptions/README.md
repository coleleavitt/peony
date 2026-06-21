# Verification Assumption Audit

These files are raw `Print Assumptions` outputs for every theorem used by a
`theorem-bridged` or `implementation-verified` public claim in
`docs/VERIFICATION_CLAIMS.json`.

Generated command shape:

```sh
printf 'From Peony Require Import <Module>.\nPrint Assumptions <theorem>.\n' \
  | coqtop -quiet -Q rocq-tests Peony
```

The public-claim gate accepts only:

- `Closed under the global context`
- `FunctionalExtensionality.functional_extensionality_dep`

Any other assumption in one of the listed artifacts fails
`scripts/check-verification-bridges.sh`.
