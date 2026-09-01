- Keep `AGENTS.md` limited to project-specific rules explicitly set by the user.
- Treat package boundaries in `docs/package-boundaries.md` as the highest architecture constraint.
- Follow the Module Layout Conventions in `docs/package-boundaries.md`: one composition root per product package, directories mirror design modules, files split by responsibility (never by line count), no catch-all directories.
- Work toward final-state designs; avoid framing architecture decisions as MVP stages.
- Do not preserve historical baggage or compatibility paths when they conflict with the intended final design.
- Do not add legacy-data detection, migration, cleanup, or normalization paths; fix the final-state read/write contract directly and leave historical artifacts outside runtime code.
- Prefer direct final-state changes over transitional layers; remove obsolete code instead of keeping shims.
- Prefer protocol, API, or file-state integrations over spawning external CLIs; keep external processes limited to intentional provider transports.
- Branch out per requirement: create a dedicated branch off `main` for each requirement or feature, and do not commit feature work directly to `main`.
- Write all code comments in English only.
- Never run tests that call real models: keep the `real-*.e2e.test.ts` env gates (`DEMI_*_E2E`, `DEMI_CLAUDE_CODE_EVAL`) unset, and verify changes with scoped `bun test packages/<pkg>` runs instead of the full suite.

## Data Validation

- Three tiers, chosen by where the data comes from:
  1. Single-field probing of foreign thrown values (error `code` etc.) uses the shared guards in `@demicodes/utils` — never define per-file private variants.
  2. Structured data crossing a trust boundary (HTTP bodies, inbound protocol frames, third-party responses) is schema-defined with zod: one schema module per boundary, types derived via `z.infer`, no hand-rolled `isRecord` + field-probe chains.
  3. Inside our own code (both sides ours), type the contract (typed error classes, tagged unions) — re-probing or re-validating already-typed data is forbidden.
- Validate inbound only; never validate data this process constructed itself (including reading back our own single-writer persisted state — corruption should fail loudly, not be normalized).

## Code Reuse

- Put all generic, common code in `@demicodes/utils`; do not scatter utility functions across packages. Test-only helpers live in the owning package's `/testing` entrypoint (e.g. `@demicodes/provider/testing`, `@demicodes/shell/testing`); never create a standalone test-utilities package.
- Never re-implement, copy-paste, or create a same-purpose-but-differently-named helper; reuse the existing one and merge duplicates/similar functions instead of adding another.
- Only truly generic code goes in `@demicodes/utils`; domain helpers stay in their owning package (provider wire mapping in the provider kit, `TokenUsage` helpers in `@demicodes/core`, etc.).

## Design Records

- Keep project documentation under `docs/`.
- Verify runnable paths and external interfaces before writing concrete design plans.
- Document test modules and their intended coverage under `docs/`.
- Keep a live implementation log per roadmap milestone in `docs/demi-next-progress.md`: status, pitfalls encountered, and conclusions, updated as the work happens so it can be resumed and reviewed at any time.
- Write design records as standalone final-state documents: never leave residue of superseded designs ("originally X", "replaced/retired by review", "no longer") in them — readers have no historical context. Review history and rejected alternatives belong only in the progress log.

## Submodules

- Inspect dirty submodules before deciding whether their changes belong to the checkpoint.
- Commit accepted submodule changes on a dedicated branch inside the submodule.
- Commit the root submodule pointer separately after the submodule commit.

## Commits

- Commit completed checkpoints automatically with appropriate Conventional Commit subjects.
- Push immediately after every commit; never leave committed work unpushed.
- Commit every `AGENTS.md` update immediately.
