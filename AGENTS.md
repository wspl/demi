- Keep `AGENTS.md` limited to project-specific rules explicitly set by the user.
- Treat package boundaries in `docs/package-boundaries.md` as the highest architecture constraint.
- Work toward final-state designs; avoid framing architecture decisions as MVP stages.
- Do not preserve historical baggage or compatibility paths when they conflict with the intended final design.
- Prefer direct final-state changes over transitional layers; remove obsolete code instead of keeping shims.
- Prefer protocol, API, or file-state integrations over spawning external CLIs; keep external processes limited to intentional provider transports.
- Do not create or switch to new branches during routine work; stay on the current branch unless the user explicitly requests branch management.
- Do not use subagents: never invoke the `Agent`/Task tool to spawn subagents. Perform all work directly in the main session.

## Code Reuse

- Put all generic, common code in `@demi/utils` (test-only helpers in `@demi/testkit`); do not scatter utility functions across packages.
- Never re-implement, copy-paste, or create a same-purpose-but-differently-named helper; reuse the existing one and merge duplicates/similar functions instead of adding another.
- Only truly generic code goes in `@demi/utils`; domain helpers stay in their owning package (provider wire mapping in the provider kit, `TokenUsage` helpers in `@demi/core`, etc.).

## Design Records

- Keep project documentation under `docs/`.
- Verify runnable paths and external interfaces before writing concrete design plans.
- Update `docs/agent-rewrite-plan.md` before implementing architecture or workflow changes.
- Document test modules and their intended coverage under `docs/`.

## Submodules

- Inspect dirty submodules before deciding whether their changes belong to the checkpoint.
- Commit accepted submodule changes on a dedicated branch inside the submodule.
- Commit the root submodule pointer separately after the submodule commit.

## Commits

- Commit completed checkpoints automatically with appropriate Conventional Commit subjects.
- Commit every `AGENTS.md` update immediately.
