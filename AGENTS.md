# Working Principles

- Understand the existing implementation and verify assumptions before designing changes.
- Prefer simple, direct designs with clear responsibilities and explicit dependencies.
- Keep each fact defined in one place. Reuse existing code and contracts; consolidate duplication.
- Do not keep a second value that can be calculated from existing data. For mutually exclusive phases, use one status such as `idle | running | finished`, rather than separate `running` and `finished` flags that can contradict each other.
- Whenever you create a timer, listener, stream, or worker, check where it is stopped or released on success, failure, and cancellation. Share cleanup code when those paths need the same cleanup. If you ignore an error, make clear why it is safe to ignore.
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
- Write separate steps on separate lines. Do not squeeze several assignments, branches, or cleanup actions into one line. A helper function should have a clear job; moving a complicated block into a vaguely named helper does not simplify it.
- Before committing, reread the complete functions you changed, not just the added lines. Check for repeated conditions, duplicate or unused values, ignored errors, and code in the wrong package. Fix those problems before calling the work complete, even when tests pass.
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
