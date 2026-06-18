- Keep `AGENTS.md` focused on durable operating principles.

## Context

- Read the relevant code before changing behavior.
- Check `agent-rewrite-plan.md` when the change touches architecture or workflow design.
- Update docs when a design decision or operating convention changes.

## Change Discipline

- Keep changes scoped to the current checkpoint and preserve unrelated user work.
- Prefer existing package boundaries, naming, and local patterns over new abstractions.

## Validation

- Validate with focused tests first; run broader checks when the change crosses package boundaries or affects user-facing behavior.
- Record any validation that could not run and why.

## Submodules

- Inspect dirty submodules before deciding whether their changes belong to the checkpoint.
- Commit submodule changes inside the submodule, then commit the root submodule pointer separately.

## Commits

- Keep the root worktree and touched submodules clean after each completed checkpoint.
- Commit completed checkpoints promptly using Conventional Commit subjects.
- Commit every `AGENTS.md` update promptly.
