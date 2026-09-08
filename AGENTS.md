# Working Principles

- Understand the existing implementation and verify assumptions before designing changes.
- Prefer simple, direct designs with clear responsibilities and explicit dependencies.
- Before adding a stateful flow, identify who owns its state, what transitions are valid, and who owns completion and cleanup. When another case makes those rules inconsistent or scattered, revise the design instead of adding another local exception.
- Keep each fact defined in one place. Reuse existing code and contracts; consolidate duplication.
- Represent related state so that valid combinations are explicit, and derive values where possible. Do not use independently mutable flags, optional fields, or type assertions to conceal an invariant that the implementation depends on.
- Give each asynchronous operation and resource an explicit owner responsible for completion, cancellation, failure, and release. Make intentional background work and ignored failures distinguishable from missing error handling.
- Implement the intended final design. Do not add compatibility layers or legacy-data migration, cleanup, or normalization paths.
- Validate external data at system boundaries using explicit schemas. Derive types from contracts; do not silently repair corrupt internal data.
- Prefer protocols, APIs, and file interfaces over external CLI processes.
- Preserve unrelated work and keep changes within the task's scope.
- Run checks appropriate to the change. Never run tests that call real models.
- Every UI behavior lives in `web-ui` as a reusable component or primitive; `web` and `web-gallery` supply only data, state and handlers. A behavior first built for one surface (a control's affordance, a page's interaction, a dialog flow) is generalized into `web-ui` before the checkpoint, never left local to the gallery or the product.
- Keep `web` and `web-gallery` synchronized in both directions: implement shared UI in `web-ui`, and update gallery examples and product usage together when shared behavior or design changes.
- Keep documentation consistent with the implementation. Describe the current design; keep history separate.
- Documentation must be readable and unambiguous. Name the responsible module, the data or action, and the observable result. Distinguish verified behavior, suspected problems, proposals, and open decisions. Explain necessary terminology in plain language; do not blur framework, backend, and UI responsibilities.
- Explain designs with concrete examples and diagrams when they improve clarity.
- Write code comments in English.
- Before committing, review the changed implementation separately from its test results. Trace state transitions and resource lifetimes, check package responsibilities, and remove redundant state and incidental complexity. Passing tests is necessary evidence of behavior, not sufficient evidence that the design is finished.
- Keep control flow readable: make meaningful state changes, branches, and cleanup steps visible. Extract code to clarify a responsibility or reuse a contract, not merely to shorten a function or hide complexity behind a name.
- Commit completed checkpoints with Conventional Commit subjects and push after each commit.

# Project References

- `docs/package-boundaries.md` is the authoritative contract for package responsibilities, dependencies, and module layout.
- Keep project documentation under `docs/`.
