# Working Principles

- Understand the existing implementation and verify assumptions before designing changes.
- Prefer simple, direct designs with clear responsibilities and explicit dependencies.
- Keep each fact defined in one place. Reuse existing code and contracts; consolidate duplication.
- Implement the intended final design. Do not add compatibility layers or legacy-data migration, cleanup, or normalization paths.
- Validate external data at system boundaries using explicit schemas. Derive types from contracts; do not silently repair corrupt internal data.
- Prefer protocols, APIs, and file interfaces over external CLI processes.
- Preserve unrelated work and keep changes within the task's scope.
- Run checks appropriate to the change. Never run tests that call real models.
- Keep documentation consistent with the implementation. Describe the current design; keep history separate.
- Explain designs with concrete examples and diagrams when they improve clarity.
- Write code comments in English.
- Commit completed checkpoints with Conventional Commit subjects and push after each commit.

# Project References

- `docs/package-boundaries.md` is the authoritative contract for package responsibilities, dependencies, and module layout.
- Keep project documentation under `docs/`.
