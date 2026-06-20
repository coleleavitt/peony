# Peony Security and Safety Rules

- Do not commit secrets, local credentials, machine-specific paths, or private
  environment details.
- Treat linker outputs as executable artifacts: malformed output can become a
  security issue when consumed by loaders or tooling.
- Avoid destructive cleanup. Delete only files created by the current task and
  only when their purpose is known.
- Do not weaken tests, mark failures ignored, or skip validations to force a
  green result.
- Keep user-level Junie files under `~/.junie/` private unless the user asks to
  copy or summarize them.
- Avoid changing imported corpora or benchmark baselines unless the task is
  explicitly about those assets.
