# Working Principles

- The system serves the interface, and the interface serves the user; never the reverse. Design the interaction first, by the principle of least astonishment and the industry's consensus, with a precedent in mainstream software (browsers, Finder, macOS's own apps); then design the system to deliver it. An interaction that follows from what the mechanism makes easy, or that no mainstream software has, is wrong however correct its mechanism is.
- Be pragmatic, never formalistic. Every step either moves the work forward or protects its correctness; drop ceremony that does neither, such as a check that cannot fail on the change at hand, a repeated full suite, or a report nobody needs.
- Tests follow [Testing](docs/delivery/testing.md). In short: each test protects a behavior a user or another component relies on, or a fixed bug, at the boundary where it is observable, once, with scenarios first; it fails before the fix or with the planted defect; it waits for events, never for time; it stays within its time budget and states its cost; it never restates the implementation or asserts a defect as correct. A test you touch that breaks these rules is fixed or deleted then.
- Follow the boy scout rule and decide on the spot: fix what you notice then, and say what you changed and why in the commit message, where the next session reads it. Stop and ask only about what changes the agreed design or scope, cannot be undone, or reaches beyond the repository and its build products.
- Read the authoritative design before discussing changes. Inspect the implementation when needed to verify feasibility or investigate behavior; resolve discrepancies explicitly rather than treating code as an implicit design decision.
- Single source of truth: one owner per fact and per purpose; two implementations of the same one-sentence purpose are a defect regardless of length or package. No stored value that can be derived; mutually exclusive phases are one status, never several flags.
- Every timer, listener, stream or worker is released on success, failure and cancellation. An ignored error says why it is safe to ignore.
- Implement the intended final design. Do not add compatibility layers or legacy-data migration, cleanup, or normalization paths. The one exception is the schema migrations between published releases, which [Storage](docs/backend/storage.md#schemas-and-migrations) requires: a change to a database schema adds the previous release's schema to the history with its migration, in the same work package.
- Before writing a helper whose purpose does not mention this project's domain, search the standard library, the crate's declared dependencies and the workspace, in that order (in the frontend: the package's declared dependencies, `@demicodes/utils`, and the workspace). Write it only when the search fails, where the next caller will find it.
- Use a library from its installed types and documentation, not from memory. Any cast (`as unknown as`, `any` or a type-only import in TypeScript; `unsafe`, `transmute` or a downcast in Rust) used to get around a library's types means you do not know its API: stop and read it. If the library genuinely lacks the capability, say so in a comment at the workaround.
- When a library is adopted for a job, use the whole of it for that job. Using it for one step and hand-writing the adjacent step it also covers (its coercion, its introspection, its error reporting) is a defect.
- Parse, don't validate: a value from outside the process is decoded against a schema at the point of entry, its type derived from the schema, never asserted. Corrupt data is never silently repaired.
- Prefer protocols, APIs, and file interfaces over external CLI processes.
- Run checks appropriate to the change. An automated test never calls a real
  model: it works against fixtures or stubs, so a suite costs nothing and
  answers the same way every time. Accepting a change by using the running
  product is a different thing, and there sending a real message is part of
  the check; stop a turn once it has shown what you were looking for.
- Every reusable UI behavior lives in `web-ui` as a component or primitive, and a plugin's feature UI lives in its plugin package, written only against `@demicodes/plugin-sdk` ([Plugin pages](docs/architecture/plugin-pages.md)); `web-ui` knows no plugin, and `web` and `web-gallery` supply only data, state, services and handlers. A behavior first built for one surface (a control's affordance, a page's interaction, a dialog flow) is generalized into `web-ui` before the checkpoint, never left local to the gallery, the product or one plugin.
- A control placed at a container's edge (an icon button at the end of a menu row, a settings row, a tab) keeps the same distance to every edge it touches: (container height - control height) / 2. The container's `web-ui` primitive computes that inset and offers a dedicated slot for such controls; a caller never positions a control through a generic slot with its own margin or padding.
- Keep `web` and `web-gallery` synchronized in both directions. Every UI change is made once, in `web-ui` or in the plugin package that owns it, whether the request came from the gallery or the product, and lands in both surfaces in the same checkpoint: update the gallery specimens and the product usage together, and verify the result in both. A change visible in only one of them is incomplete.
- Every control in the gallery responds when used. A specimen's button, menu item or link either acts on the specimen's own state the way the product would (Cancel drops the row, Clear empties the list, a dialog's buttons close it and a control opens it again), or simulates what the product would do with a host or another page and says so, for example in a neutral toast naming the action. A click that does nothing is a defect, in a specimen that shows a pinned state or a look as much as in a live one. Bind every event a specimen's component emits, and click through each new specimen before the checkpoint.
- Write code comments in English.
- Write separate steps on separate lines.
- Before committing, reread the complete functions you changed, not just the added lines. Check for repeated conditions, duplicate or unused values, ignored errors, and code in the wrong package. Fix those problems before calling the work complete, even when tests pass.
- When a batch of changes is ready for acceptance, check it yourself on processes started from the new code (the backend and the web front end, or the gallery) before reporting, and stop the processes you started once the check is done. Start them once per batch, after the whole batch is complete, not after every edit.
- Stop only the processes you started, by the process id or server id you recorded when you started them, never by a name or a pattern (`pkill -f`, `killall`): the user's own development backend runs the same programs.
- Product and gallery checks drive the browser with `bun browse` ([Product checks](docs/delivery/browse.md)), a whole check in one `bun browse run`, not a shell call per step. The tool is everyone's to change: a check that needs what it lacks adds it, and a command that misleads or breaks is fixed then, never worked around with a one-off script.
- A product check that needs something the development setup lacks, such as a paired device, a runner, a browser on a Host or a model's reply, sets it up and runs: pair a runner to the development backend through Add Device, install what the Host needs, send the real message. A check skipped because the setup lacked it is not a pass, and a gallery check never stands in for the product's. Remove what was set up only for the check once it is done.
- A screenshot is reviewed, not only taken. Whoever takes or sends one looks at everything on it, not only the change it was taken for, and fixes on the spot what it shows wrong (a raw timestamp, text that crowds a row, a misaligned control, wording against the Writing page), as the boy scout rule says; the lead does so for every screenshot before it reaches the user, and hands what it finds to the work in progress.
- Any change whose effect can be seen on screen is accepted with screenshots, not only a change of style: a layout, an interaction, a state, a flow, a message, a fixed bug. The self-check takes one screenshot of each screen the change affects, showing the change in effect, and the report puts them in front of the user, one image per screen. A subagent saves its screenshots outside the repository and lists their paths in its report; the lead sends them to the user.
- The Cloud and a paired device are the same thing: a Host behind a runner. Code never distinguishes them except where the design says they differ (pairing, revocation, lifecycle). Anything that reaches a conversation's Host goes through the conversation's host access (`with_host` in `backend-host-access`, see `docs/execution/sessions-and-targets.md` § Host operations), which resolves the target, wakes a stopped Cloud, and holds the file gate. There is no other way to a conversation's Host, and every other way to a Host is named in that section; if the host access does not fit, change the design first.
- In development, build native code with the machine's own toolchain and cross tools, not the build container, and build and package only the targets of the Hosts in use (`docs/delivery/builds-and-releases.md`). All six targets are for a published release, which the release workflow builds on GitHub's hosted runners, each platform on its own (§ Release workflow).
- Commit completed checkpoints with Conventional Commit subjects and push after each commit.

# Building and Testing

- One work package is one checkpoint, committed and pushed once. While writing, run only `cargo check` and the test that covers the code; run the work package's checks once at its end.
- Build and test with one Cargo selection everywhere: `cargo check --workspace --all-targets --features demi-runner/test-fixtures` and `cargo test --workspace --features demi-runner/test-fixtures`, adding `--test <name>` for one target. Never `-p <crate>` for a test build: each selection keeps its own copy of the dependencies, and switching has cost 140 s a time.
- Keep test binaries few: each crate has one (`crates-and-packages.md` § Module layout), such as `crates/runner/tests/runner/` and `crates/command-package-browser/tests/browser/`, and the crate boundary check enforces it; a test gets a binary of its own only when it changes or saturates process-wide state. Each binary costs a link and, when newly built, a first-launch check of about three seconds here. TypeScript tests never build a program: `bun run test` builds them and runs the suite in parallel; to run some tests, build first and set `DEMI_TEST_PROGRAMS=target/debug`.
- Do not run `cargo fmt` or `cargo clippy`, and do not add them to any check: neither is part of this project's validation.
- The compiler is the only hard constraint on how code and text are written. A convention (naming, capitalization, layout, style) is written down in the docs or the gallery and followed; never enforce one with a lint, a script or a test. Tests protect behavior.
- Work in large steps: read what a step needs in one call, write the whole step, then compile once. Start long builds and tests in the background and keep working meanwhile.
- zsh does not split an unquoted variable into words; pass argument lists as arrays, or run scripts with bash.

# Delegation

- The main agent is the one mind that holds the whole context: it does all design, planning and decision work itself, and writes every design document and every document change itself. A subagent never makes a significant decision and never writes documentation.
- A subagent only executes: it writes code to a design the main agent settled, runs tests, or checks a result, and reports what it found. Research a subagent does is input to the main agent's decision, not the decision.

# Parallel Development Mode

Only when the user says to enter parallel development mode:

- Hand each independent implementation, test or acceptance item to a background subagent and stay free to talk with the user. Items that change the same files run one after another.
- Subagents work in four slots, `../demi-slots/1` to `../demi-slots/4`. A slot is a git worktree that stays between tasks, with its own warm `target/`, `node_modules/` and `.cache/`, so a task starts without copying anything. A slot is made once by cloning the checkout's warm `target/` into it with `cp -c` (APFS copy-on-write; skip `target/debug/incremental`). At most four items run at once; another waits for a free slot.
- The lead names a free slot in each subagent's brief. The subagent starts its branch there from the lead's branch with `git switch --discard-changes -C <branch> feat/demi-next` and `git clean -fd`, which keeps the ignored build products. It commits on its branch and never pushes.
- Slot *n* starts its servers on ports 33*n*0–33*n*9 and 189*n*0–189*n*9, so slots never collide with each other or with the user's own development servers.
- The lead reviews each result, merges it into the branch of the user's own checkout, tests and pushes from there, and deletes the branch, which frees the slot. After each merge it rebuilds and restarts the development backend and web front end from the checkout (`bun run dev`, `bun run web:dev`), so the user sees the new code at once.
- When the disk runs low, the lead removes from the checkout and every slot the build products no build has used for a week.

# Writing and Communication

- Make the first explanation understandable without requiring the reader to ask for a simpler version. Start with what happens in a concrete example, then explain the rule. Use familiar words; introduce a technical term only when needed and explain it on first use.
- When identifying a problem or ambiguity, show the exact situation, what each rule would make happen, and why the difference matters. Do not substitute abstract labels such as "content missing" or "rules overlap" for that explanation.
- Before sending an explanation, check whether the reader can tell who does what, to which thing, and with what result. Rewrite sentences that require the reader to translate terminology or infer omitted steps. Keep necessary technical detail; remove detail that does not help answer the question.
- Prefer diagrams for structures, relationships, and flows when they clarify the explanation; use ASCII diagrams when practical. Labels must be understandable from the accompanying explanation.
- Explain responsibilities, boundaries, observable behavior, and design rationale. Leave implementation details that code can express clearly to code.
- Distinguish verified facts, suspected problems, proposals, and open decisions.
- UI text follows macOS capitalization: the gallery's Writing page (`packages/web-gallery/src/sections/WritingSection.vue`) is the rule.
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

- Rust: Rust API Guidelines.
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
