- Keep `AGENTS.md` limited to project-specific rules explicitly set by the user.
- Treat package boundaries in `docs/package-boundaries.md` as the highest architecture constraint.
- Work toward final-state designs; avoid framing architecture decisions as MVP stages.
- Do not preserve historical baggage or compatibility paths when they conflict with the intended final design.
- Do not add legacy-data detection, migration, cleanup, or normalization paths; fix the final-state read/write contract directly and leave historical artifacts outside runtime code.
- Prefer direct final-state changes over transitional layers; remove obsolete code instead of keeping shims.
- Prefer protocol, API, or file-state integrations over spawning external CLIs; keep external processes limited to intentional provider transports.
- Branch out per requirement: create a dedicated branch off `main` for each requirement or feature, and do not commit feature work directly to `main`.
- Do not use subagents: never invoke the `Agent`/Task tool to spawn subagents. Perform all work directly in the main session.
- Write all code comments in English only.

## Code Reuse

- Put all generic, common code in `@demicodes/utils` (test-only helpers in `@demicodes/testkit`); do not scatter utility functions across packages.
- Never re-implement, copy-paste, or create a same-purpose-but-differently-named helper; reuse the existing one and merge duplicates/similar functions instead of adding another.
- Only truly generic code goes in `@demicodes/utils`; domain helpers stay in their owning package (provider wire mapping in the provider kit, `TokenUsage` helpers in `@demicodes/core`, etc.).

## Design Records

- Keep project documentation under `docs/`.
- Verify runnable paths and external interfaces before writing concrete design plans.
- Document test modules and their intended coverage under `docs/`.

## Submodules

- Inspect dirty submodules before deciding whether their changes belong to the checkpoint.
- Commit accepted submodule changes on a dedicated branch inside the submodule.
- Commit the root submodule pointer separately after the submodule commit.

## Commits

- Commit completed checkpoints automatically with appropriate Conventional Commit subjects.
- Push immediately after every commit; never leave committed work unpushed.
- Commit every `AGENTS.md` update immediately.
