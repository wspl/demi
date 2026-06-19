- Keep `AGENTS.md` limited to project-specific rules explicitly set by the user.
- Work toward final-state designs; avoid framing architecture decisions as MVP stages.
- Do not preserve historical baggage or compatibility paths when they conflict with the intended final design.
- Prefer direct final-state changes over transitional layers; remove obsolete code instead of keeping shims.

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
