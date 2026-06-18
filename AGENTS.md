- Keep `AGENTS.md` limited to project-specific rules explicitly set by the user.

## Design Records

- Update `agent-rewrite-plan.md` before implementing architecture or workflow changes.

## Submodules

- Inspect dirty submodules before deciding whether their changes belong to the checkpoint.
- Commit accepted submodule changes on a dedicated branch inside the submodule.
- Commit the root submodule pointer separately after the submodule commit.

## Commits

- Commit completed checkpoints automatically with appropriate Conventional Commit subjects.
- Commit every `AGENTS.md` update immediately.
