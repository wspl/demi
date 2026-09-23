# Working Principles

- Be pragmatic, never formalistic. Every step either moves the work forward or protects its correctness; drop ceremony that does neither, such as a check that cannot fail on the change at hand, a repeated full suite, or a report nobody needs.
- Tests protect behavior, not implementation. Keep one test for each behavior another component or the user relies on and one for each bug that was fixed, at the boundary where it is observable (a wire format, a state machine, an API scenario), once, not again in every layer. Do not test internal steps. A slow test needs a reason: soaks and long waits stay out of the regular suite. A test you touch that is redundant, tests internals or waits without cause is fixed or deleted then.
- Follow the boy scout rule and decide on the spot. When you notice on the way something that slows the work or is wrong (a slow or duplicated build, a flaky test, a stale script, a wasteful habit), fix it then and note what you changed and why where the next session reads it; for the Rust migration that is `plan.md` § Working method. Do not stop to ask or save it for a review. Stop and ask only about what changes the agreed design or scope, cannot be undone, or reaches beyond the repository and its build products.
- Read the authoritative design before discussing changes. Inspect the implementation when needed to verify feasibility or investigate behavior; resolve discrepancies explicitly rather than treating code as an implicit design decision.
- Prefer simple, direct designs with clear responsibilities and explicit dependencies.
- Keep each fact defined in one place. Reuse existing code and contracts; consolidate duplication.
- Do not keep a second value that can be calculated from existing data. For mutually exclusive phases, use one status such as `idle | running | finished`, rather than separate `running` and `finished` flags that can contradict each other.
- Whenever you create a timer, listener, stream, or worker, check where it is stopped or released on success, failure, and cancellation. Share cleanup code when those paths need the same cleanup. If you ignore an error, make clear why it is safe to ignore.
- Implement the intended final design. Do not add compatibility layers or legacy-data migration, cleanup, or normalization paths.
- Before writing a helper, state its purpose in one generic sentence. If that sentence does not mention this project's domain, the helper almost certainly exists: search the standard library, the crate's declared dependencies, and the workspace, in that order (in the frontend: the package's declared dependencies, `@demicodes/utils`, and the workspace). Write it only when the search fails, and place it where the next caller will find it. Small size is not a reason to write a local copy.
- Use a library from its installed types and documentation, not from memory. Any cast (`as unknown as`, `any` or a type-only import in TypeScript; `unsafe`, `transmute` or a downcast in Rust) used to get around a library's types means you do not know its API: stop and read it. If the library genuinely lacks the capability, say so in a comment at the workaround.
- When a library is adopted for a job, use the whole of it for that job. Using it for one step and hand-writing the adjacent step it also covers (its coercion, its introspection, its error reporting) is a defect.
- A type assertion is not a check. A value from outside the process (network, file, socket, environment, storage, model output, child process) is validated against a schema at the point of entry; the type is derived from the schema, never asserted onto the value. Do not silently repair corrupt data.
- Two implementations of the same one-sentence purpose are a defect regardless of length or package. Consolidate to one owner and import.
- Prefer protocols, APIs, and file interfaces over external CLI processes.
- Preserve unrelated work and keep changes within the task's scope.
- Run checks appropriate to the change. An automated test never calls a real
  model: it works against fixtures or stubs, so a suite costs nothing and
  answers the same way every time. Accepting a change by using the running
  product is a different thing, and there sending a real message is part of
  the check; stop a turn once it has shown what you were looking for.
- Every UI behavior lives in `web-ui` as a reusable component or primitive; `web` and `web-gallery` supply only data, state and handlers. A behavior first built for one surface (a control's affordance, a page's interaction, a dialog flow) is generalized into `web-ui` before the checkpoint, never left local to the gallery or the product.
- A control placed at a container's edge (an icon button at the end of a menu row, a settings row, a tab) keeps the same distance to every edge it touches: (container height - control height) / 2. The container's `web-ui` primitive computes that inset and offers a dedicated slot for such controls; a caller never positions a control through a generic slot with its own margin or padding.
- Keep `web` and `web-gallery` synchronized in both directions. Every UI change is made once in `web-ui`, whether the request came from the gallery or the product, and lands in both surfaces in the same checkpoint: update the gallery specimens and the product usage together, and verify the result in both. A change visible in only one of them is incomplete.
- Every control in the gallery responds when used. A specimen's button, menu item or link either acts on the specimen's own state the way the product would (Cancel drops the row, Clear empties the list, a dialog's buttons close it and a control opens it again), or simulates what the product would do with a host or another page and says so, for example in a neutral toast naming the action. A click that does nothing is a defect, in a specimen that shows a pinned state or a look as much as in a live one. Bind every event a specimen's component emits, and click through each new specimen before the checkpoint.
- Write code comments in English.
- Write separate steps on separate lines. Do not squeeze several assignments, branches, or cleanup actions into one line. A helper function should have a clear job; moving a complicated block into a vaguely named helper does not simplify it.
- Before committing, reread the complete functions you changed, not just the added lines. Check for repeated conditions, duplicate or unused values, ignored errors, and code in the wrong package. Fix those problems before calling the work complete, even when tests pass.
- When a batch of changes is ready for acceptance, restart every locally running process that serves it on the new code before reporting (the backend on port 3271 and the web front end); do not hand the restart to the user. Restart once per batch, after the whole batch is complete, not after every edit.
- The Cloud and a paired device are the same thing: a Host behind a runner. Code never distinguishes them except where the design says they differ (pairing, revocation, lifecycle). Anything that reaches a conversation's Host goes through the conversation's host access (`Shard::with_host`, see `docs/execution/sessions-and-targets.md` § Host operations), which resolves the target, wakes a stopped Cloud, and holds the file gate. There is no other way to a conversation's Host, and every other way to a Host is named in that section; if the host access does not fit, change the design first.
- Build native code with the machine's own cross tools, not the build container, and in development build and package only the targets of the Hosts in use (`docs/delivery/builds-and-releases.md`). All six targets are for a published release.
- A change to the runner reaches every runtime that carries it: every build target, the paired device, and the Cloud guest image. Acceptance of anything that touches a Host is done on both a paired device and the Cloud.
- Keep the local Cloud guest image current yourself, without being asked: whenever the runner or a native command package changes (`demi-commands`, `demi-claude`, any crate the guest carries), cross-compile it for the guest, embed it in the rootfs, restart the machine manager, and reset the local Cloud onto the new base before acceptance. The local Cloud is a development environment: rebuild its image, restart its manager and reset it freely. A feature that runs on the Cloud is not accepted until it has run on the updated image.
- Commit completed checkpoints with Conventional Commit subjects and push after each commit.

# Rust Migration

While the Rust migration in `docs/internal/rust-migration/plan.md` is under way, reread its § Working method and § Checkpoint checks at the start of every session and after every context compaction, before any other migration work, and follow them. In short:

- Work the critical path: port the TypeScript. In Rust that already works, fix only the high findings; leave its medium and low findings for when that module next changes, or for the cleanup sweep at the end.
- Port tests by behavior, not case by case: from a module's TypeScript tests, list what it must guarantee and cover that with a few Rust scenario and unit tests. `ledger.md` entries are checked off by the test or scenario that covers their behavior, or as testing TypeScript internals.
- The last work packages keep a cleanup sweep: the deferred findings, the tests (redundant, internal or slow), and whatever the port missed.
- One work package is one checkpoint, committed and pushed once. While writing, run only `cargo check` and the test that covers the code; run the work package's checks once at its end. The Host acceptance (native builds for the Hosts in use, the refreshed Cloud image and a reset, a paired device and the Cloud, restarted processes, a real message) runs once per phase and before a batch is reported for acceptance.
- Build and test with one Cargo selection everywhere: `cargo check --workspace --all-targets --features demi-runner/test-fixtures` and `cargo test --workspace --features demi-runner/test-fixtures`, adding `--test <name>` for one target. Never `-p <crate>` for a test build: each selection keeps its own copy of the dependencies, and switching has cost 140 s a time.
- Keep test binaries few. A new integration test joins its crate's shared test binary (`crates/runner/tests/runner/`, `crates/demi-commands/tests/browser/`); a test gets a binary of its own only when it changes or saturates process-wide state. Each binary costs a link and, when newly built, a first-launch check of about three seconds here. TypeScript tests never build a program: `bun run test` builds them and runs the suite in parallel; to run some tests, build first and set `DEMI_TEST_PROGRAMS=target/debug`.
- Work in large steps: read what a step needs in one call, write the whole step, then compile once. Start long builds and tests in the background and keep working meanwhile.
- zsh does not split an unquoted variable into words; pass argument lists as arrays, or run scripts with bash.

# Writing and Communication

- Make the first explanation understandable without requiring the reader to ask for a simpler version. Start with what happens in a concrete example, then explain the rule. Use familiar words; introduce a technical term only when needed and explain it on first use.
- When identifying a problem or ambiguity, show the exact situation, what each rule would make happen, and why the difference matters. Do not substitute abstract labels such as "content missing" or "rules overlap" for that explanation.
- Before sending an explanation, check whether the reader can tell who does what, to which thing, and with what result. Rewrite sentences that require the reader to translate terminology or infer omitted steps. Keep necessary technical detail; remove detail that does not help answer the question.
- Prefer diagrams for structures, relationships, and flows when they clarify the explanation; use ASCII diagrams when practical. Labels must be understandable from the accompanying explanation.
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

- Rust: Rust API Guidelines, `rustfmt` defaults and the workspace clippy lints.
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

- `docs/architecture/crates-and-packages.md` is the authoritative contract for crate and package responsibilities, dependencies, and module layout.
