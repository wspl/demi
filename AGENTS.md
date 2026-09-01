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

Guidance on picking the right tool when handling data of uncertain shape:

- Reading a field off a caught `unknown` (an error's `code`, its message)? `@demicodes/utils` already has `errorCode` / `errorMessage` / `asError` / `isAbortError` — use those; if one is missing, add it there rather than writing a local copy.
- Parsing structured data that arrives from outside the process (an HTTP body, an inbound protocol frame, a third-party response)? Define a zod schema next to the boundary's types and derive the TS type with `z.infer`. If you find yourself writing an `isRecord` + field-by-field check chain, that's the sign a schema wants to exist.
- When both producer and consumer are our code, prefer putting the type on the contract itself (a typed error class, a tagged union) so downstream just uses it — a second round of checking adds noise, not safety.
- Data this process built or wrote itself (outbound frames, our own persisted rows) doesn't need validating on the way back in; if it reads back corrupt, a loud failure beats silent normalization.

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
