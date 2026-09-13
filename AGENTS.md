# Working Principles

- Read the authoritative design before discussing changes. Inspect the implementation when needed to verify feasibility or investigate behavior; resolve discrepancies explicitly rather than treating code as an implicit design decision.
- Prefer simple, direct designs with clear responsibilities and explicit dependencies.
- Keep each fact defined in one place. Reuse existing code and contracts; consolidate duplication.
- Do not keep a second value that can be calculated from existing data. For mutually exclusive phases, use one status such as `idle | running | finished`, rather than separate `running` and `finished` flags that can contradict each other.
- Whenever you create a timer, listener, stream, or worker, check where it is stopped or released on success, failure, and cancellation. Share cleanup code when those paths need the same cleanup. If you ignore an error, make clear why it is safe to ignore.
- Implement the intended final design. Do not add compatibility layers or legacy-data migration, cleanup, or normalization paths.
- Before writing a helper, state its purpose in one generic sentence. If that sentence does not mention this project's domain, the helper almost certainly exists: search the standard library, the package's declared dependencies, `@demicodes/utils`, and the workspace, in that order. Write it only when the search fails, and place it where the next caller will find it. Small size is not a reason to write a local copy.
- Use a library from its installed types and documentation, not from memory. Any cast, `as unknown as`, `any`, or type-only import used to get around a library's types means you do not know its API: stop and read it. If the library genuinely lacks the capability, say so in a comment at the workaround.
- When a library is adopted for a job, use the whole of it for that job. Using it for one step and hand-writing the adjacent step it also covers (its coercion, its introspection, its error reporting) is a defect.
- A type assertion is not a check. A value from outside the process (network, file, socket, environment, storage, model output, child process) is validated against a schema at the point of entry; the type is derived from the schema, never asserted onto the value. Do not silently repair corrupt data.
- Two implementations of the same one-sentence purpose are a defect regardless of length or package. Consolidate to one owner and import.
- Prefer protocols, APIs, and file interfaces over external CLI processes.
- Preserve unrelated work and keep changes within the task's scope.
- Run checks appropriate to the change. Never run tests that call real models.
- Every UI behavior lives in `web-ui` as a reusable component or primitive; `web` and `web-gallery` supply only data, state and handlers. A behavior first built for one surface (a control's affordance, a page's interaction, a dialog flow) is generalized into `web-ui` before the checkpoint, never left local to the gallery or the product.
- Keep `web` and `web-gallery` synchronized in both directions. Every UI change is made once in `web-ui`, whether the request came from the gallery or the product, and lands in both surfaces in the same checkpoint: update the gallery specimens and the product usage together, and verify the result in both. A change visible in only one of them is incomplete.
- Write code comments in English.
- Write separate steps on separate lines. Do not squeeze several assignments, branches, or cleanup actions into one line. A helper function should have a clear job; moving a complicated block into a vaguely named helper does not simplify it.
- Before committing, reread the complete functions you changed, not just the added lines. Check for repeated conditions, duplicate or unused values, ignored errors, and code in the wrong package. Fix those problems before calling the work complete, even when tests pass.
- When a batch of changes is ready for acceptance, restart every locally running process that serves it on the new code before reporting (the backend on port 3271 and the web front end); do not hand the restart to the user. Restart once per batch, after the whole batch is complete, not after every edit.
- Commit completed checkpoints with Conventional Commit subjects and push after each commit.

# Writing and Communication

- Write for the reader's understanding. Be clear, concrete, and concise.
- Prefer diagrams for structures, relationships, and flows; use ASCII diagrams when practical. Prefer concrete examples over abstract descriptions.
- Explain responsibilities, boundaries, observable behavior, and design rationale. Leave implementation details that code can express clearly to code.
- Distinguish verified facts, suspected problems, proposals, and open decisions.
- Follow the [Google Developer Documentation Style Guide](https://developers.google.com/style). Use Diátaxis, arc42, and C4 as optional aids to clarity and completeness, not as mandatory directories, sections, or deliverables.

# Design Documentation

- Web documentation covers only the technology stack, architecture, and system boundaries. Use the gallery for component, style, layout, typography, and interaction examples; do not duplicate them in design documents.

- Keep project documentation under `docs/`, with a concise reading index. Organize by stable design topic: content understood and changed together belongs together. Each design rule has one authoritative home; other documents briefly explain the connection and link to it instead of restating the rule. Split a topic only when the parts can be understood and changed independently, not merely because they involve different packages or the document is long.
- A design document must support discussion without reading code and implementation without guessing key behavior. As needed, explain the purpose and scope; how it works and who is responsible, using a concrete example or diagram; required behavior, data meanings, boundaries, and relevant failure handling; and the rationale for non-obvious decisions. These are questions to answer, not mandatory headings. Specify interface details when they are necessary for components to work together.
- Write only what is needed to understand, decide, and implement the design. Simple designs may take a few paragraphs. Omit empty sections, obvious details, code walkthroughs, repeated rules, and speculative extensions.
- Describe the selected design in its intended final form. Clearly identify unresolved decisions that affect implementation; resolve them before implementing dependent behavior. Keep implementation progress and decision history separate from the design body. An agreed design does not imply completed implementation.
- Follow this workflow for new designs and changes: read the authoritative document, discuss the change, update that document, implement it, and check the result against the design. Design updates may be committed before implementation; use the design diff together with the full affected contract to determine the work. If implementation reveals a design gap, resolve it and update the document rather than silently deciding in code.
- Before completing a checkpoint, reread the affected design, check relevant links and referring documents, and verify implemented behavior against its rules and examples. Resolve contradictions at the authoritative source and remove duplicate rules. If implementation is deferred, state that explicitly in the handoff. Acceptance requires that the design can be discussed without code, that key behavior needs no guesswork, and that completed implementation matches it.

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
