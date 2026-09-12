# Working Principles

- Understand the existing implementation and verify assumptions before designing changes.
- Prefer simple, direct designs with clear responsibilities and explicit dependencies.
- Keep each fact defined in one place. Reuse existing code and contracts; consolidate duplication.
- Do not keep a second value that can be calculated from existing data. For mutually exclusive phases, use one status such as `idle | running | finished`, rather than separate `running` and `finished` flags that can contradict each other.
- Whenever you create a timer, listener, stream, or worker, check where it is stopped or released on success, failure, and cancellation. Share cleanup code when those paths need the same cleanup. If you ignore an error, make clear why it is safe to ignore.
- Implement the intended final design. Do not add compatibility layers or legacy-data migration, cleanup, or normalization paths.
- Validate external data at system boundaries using explicit schemas. Derive types from contracts; do not silently repair corrupt internal data.
- Before changing a data boundary, identify its contract owner, parsing location, and error behavior using [Data Contracts](docs/data-contracts.md). Existing code and historical plans do not establish exceptions.
- Keep runtime validation, derived types, and external schema declarations on one contract. Keep argv conversion at the CLI boundary; typed RPC inputs do not inherit it. Verify invalid inputs and serialized round trips as well as successful inputs.
- Before adding a basic helper, compare existing shared functions and the installed library's actual semantics. Retained adapters must have a specific responsibility; renaming equivalent helpers does not remove duplication.
- Prefer protocols, APIs, and file interfaces over external CLI processes.
- Preserve unrelated work and keep changes within the task's scope.
- Run checks appropriate to the change. Never run tests that call real models.
- Every UI behavior lives in `web-ui` as a reusable component or primitive; `web` and `web-gallery` supply only data, state and handlers. A behavior first built for one surface (a control's affordance, a page's interaction, a dialog flow) is generalized into `web-ui` before the checkpoint, never left local to the gallery or the product.
- Keep `web` and `web-gallery` synchronized in both directions. Every UI change is made once in `web-ui`, whether the request came from the gallery or the product, and lands in both surfaces in the same checkpoint: update the gallery specimens and the product usage together, and verify the result in both. A change visible in only one of them is incomplete.
- Keep implementation documentation consistent with the implementation; keep history separate.
- When designing a new feature or architecture, directly update or create the corresponding design document under `docs/`. Describe the intended design in its final form; do not narrate what was missing, how it worked before, or the transition from the old design. Design-document changes may be committed before implementation; use their commit diff to determine the implementation work.
- Documentation must be readable and unambiguous. Name the responsible module, the data or action, and the observable result. Distinguish verified behavior, suspected problems, proposals, and open decisions. Explain necessary terminology in plain language; do not blur framework, backend, and UI responsibilities.
- Explain designs with concrete examples and diagrams when they improve clarity.
- Write code comments in English.
- Write separate steps on separate lines. Do not squeeze several assignments, branches, or cleanup actions into one line. A helper function should have a clear job; moving a complicated block into a vaguely named helper does not simplify it.
- Before committing, reread the complete functions you changed, not just the added lines. Check for repeated conditions, duplicate or unused values, ignored errors, and code in the wrong package. Fix those problems before calling the work complete, even when tests pass.
- When a batch of changes is ready for acceptance, restart every locally running process that serves it on the new code before reporting (the backend on port 3271 and the web front end); do not hand the restart to the user. Restart once per batch, after the whole batch is complete, not after every edit.
- Commit completed checkpoints with Conventional Commit subjects and push after each commit.

# Coding Standards

- TypeScript: Google TypeScript Style Guide.
- JavaScript: Google JavaScript Style Guide.
- Vue: Vue Style Guide.
- HTML/CSS: Google HTML/CSS Style Guide.
- C: LLVM Coding Standards.
- Bash: Google Shell Style Guide.
- Python: PEP 8.
- CMake: KDE CMake Coding Style.
- JSON: Google JSON Style Guide.
- YAML: Home Assistant YAML Style Guide.
- Markdown: Google Markdown Style Guide.

# Project References

- `docs/package-boundaries.md` is the authoritative contract for package responsibilities, dependencies, and module layout.
- Keep project documentation under `docs/`.
