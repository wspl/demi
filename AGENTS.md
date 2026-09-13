# Working Principles

- Read the authoritative design before discussing changes. Inspect the implementation when needed to verify feasibility or investigate behavior; resolve discrepancies explicitly rather than treating code as an implicit design decision.
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
- Keep `web` and `web-gallery` synchronized in both directions. Every UI change is made once in `web-ui`, whether the request came from the gallery or the product, and lands in both surfaces in the same checkpoint: update the gallery specimens and the product usage together, and verify the result in both. A change visible in only one of them is incomplete.
- Write code comments in English.
- Write separate steps on separate lines. Do not squeeze several assignments, branches, or cleanup actions into one line. A helper function should have a clear job; moving a complicated block into a vaguely named helper does not simplify it.
- Before committing, reread the complete functions you changed, not just the added lines. Check for repeated conditions, duplicate or unused values, ignored errors, and code in the wrong package. Fix those problems before calling the work complete, even when tests pass.
- When a batch of changes is ready for acceptance, restart every locally running process that serves it on the new code before reporting (the backend on port 3271 and the web front end); do not hand the restart to the user. Restart once per batch, after the whole batch is complete, not after every edit.
- The Cloud and a paired device are the same thing: a Host behind a runner. Code never distinguishes them except where the design says they differ (pairing, revocation, lifecycle). Anything that reaches a conversation's Host goes through the conversation's host access (`withHost`, see `docs/demi-next/sessions-and-targets.md` § Host operations), which resolves the target, wakes a stopped Cloud, and holds the file gate. There is no second way to a Host; if `withHost` does not fit, change the design first.
- A change to the runner reaches every runtime that carries it: every build target, the paired device, and the Cloud guest image. Acceptance of anything that touches a Host is done on both a paired device and the Cloud.
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
