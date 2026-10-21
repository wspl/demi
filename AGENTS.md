<!-- Instructions for the coding agents that work in this repository. Human
contributors start with CONTRIBUTING.md. -->

# Working Principles

- The system serves the interface, and the interface serves the user; never the reverse. Design the interaction first, by the principle of least astonishment and the industry's consensus, with a precedent in mainstream software (browsers, Finder, macOS's own apps); then design the system to deliver it. An interaction that follows from what the mechanism makes easy, or that no mainstream software has, is wrong however correct its mechanism is. The technical design of a capability Demi does not have yet starts from how the leading open-source projects of its kind built it: read their code before the design, and its decision record says what they do and where it departs.
- Create first, then edit. An Add action creates the thing at once, with a default for every field (a name such as *Compatible API*, *Compatible API (2)* for the second), and opens the same editor an existing one opens; nothing asks for a value in a dialog before the thing exists, as Finder's New Folder and Mail's Add Account do not. Adding and editing are one view, with the same fields in the same order, and every field stays editable afterwards unless the thing's nature fixes it. A field the thing cannot work without is shown as missing, never demanded up front; a field it can work without is optional.
- What the product does, shows and says changes only by the user's request or a design the user agreed. Stop and ask only about what changes the agreed design or scope, cannot be undone, or reaches beyond the repository and its build products.
- When the user is unhappy with how you or another agent behaved, first look for the existing rule that led to it, in AGENTS.md, the workflow, the instructions or a decision record, and propose changing or deleting that rule; propose a new rule only when no existing one explains the behavior.
- Single source of truth: one owner per fact and per purpose; two implementations of the same one-sentence purpose are a defect regardless of length or package. No stored value that can be derived; mutually exclusive phases are one status, never several flags.
- Every prompt, AGENTS.md, a workflow, a profile, a brief to an agent, a system prompt or a tool's description, says what is needed and what the result must meet, never how to do it.
- Correct a model's behavior (a system prompt, an instruction, a tool's description) with the general rule it missed, never by writing the case at hand or its answer into the text: a prompt that quotes the incident it was fixed for teaches that incident and nothing else, and every such patch makes the prompt longer and worse. The incident belongs in the commit message.
- Prefer protocols, APIs, and file interfaces over external CLI processes.
- Everything committed, code, comments, documents, tests, fixtures, gallery data and commit messages alike, holds only invented examples: made-up tasks and titles, names such as *alex*, domains such as `example.com`, paths such as `/Users/alex`. Nothing committed comes from a real development session or environment: no conversation's title or words, no account of who asked for a change or how it was discussed, no real person's, account's, machine's, host's or server's name, no real path, address or secret.
- Write code comments in English.
- A work package is one change a user or a reviewer understands on its own, such as "a device is drawn as its system's logo" or "a Claude Code answer that replaces a failed stream is handed on": its decision record when it makes a new decision, its code and its tests together. It is one commit with a Conventional Commit subject that says what changed and why, pushed once; a later fix to it before it lands, a review's rework or a reworded sentence of its decision record included, is folded into that commit, never added as a commit of its own. A commit never holds a part of a work package, such as its runner half or its decision record alone.
- Release only when the user asks for a release; releasing and upgrading the user's server then need no further confirmation. Deploy to the server `DEMI_DEPLOY_HOST` names in the user's `.env` by the steps of `docs/delivery/upgrades.md` § A development build on a server when the user or the workflow asks for it, and then the whole merged batch.

# Building and Testing

- Tests follow [Testing](docs/delivery/testing.md).
- The full typecheck and test suites run once, when a work package is delivered, never between edits. A small change the lead makes itself is checked once by what it changes: a test it changes runs alone, and the CI's checks cover the rest.
- Every build shares one copy of the dependencies; no build compiles them again for another selection ([Testing](docs/delivery/testing.md) § Running).
- Whoever builds runs `bun xtask sweep` in that checkout when the work is done, so old build products do not fill the disk.
- Keep test binaries few, as `ARCHITECTURE.md` § Test binaries says.
- A development build builds only the targets of the Hosts in use; all six targets are for a published release (`.github/workflows/release.yml`).
- Do not run `cargo fmt` or `cargo clippy`, and do not add them to any check: neither is part of this project's validation.
- The compiler is the only hard constraint on how code and text are written. A convention (naming, capitalization, layout, style) is written down in `ARCHITECTURE.md`, an entry comment or the gallery and followed; never enforce one with a lint, a script or a test. Tests protect behavior.

# Product Checks

- Any change whose effect can be seen on screen, a layout, an interaction, a state, a flow, a message or a fixed bug, is accepted by whoever made it on the running product, with one screenshot of each screen it affects showing the change in effect. The acceptance starts its processes from the new code once, after the whole change is complete, sends real messages where the change needs them, and stops the processes it started when it is done. A subagent saves its screenshots outside the repository and lists their paths in its report; the lead sends them to the user, one image per screen.
- Product and gallery checks drive the browser with `bun browse` ([Product checks](docs/delivery/browse.md)). The tool is everyone's to change: a check that needs what it lacks adds it, and a command that misleads or breaks is fixed then, never worked around.
- A product check that needs something the development setup lacks, such as a paired device, a runner, a browser on a Host or a model's reply, sets it up and runs. Remove what was set up only for the check once it is done.

# The Gallery

- Keep `web` and `web-gallery` synchronized in both directions. Every UI change is made once, in `web-ui` or in the plugin package that owns it, whether the request came from the gallery or the product, and lands in both surfaces in the same commit: update the gallery specimens and the product usage together, and verify the result in both. A change visible in only one of them is incomplete.
- Every control in the gallery responds when used. A specimen's button, menu item or link either acts on the specimen's own state the way the product would (Cancel drops the row, Clear empties the list, a dialog's buttons close it and a control opens it again), or simulates what the product would do with a host or another page and says so, for example in a neutral toast naming the action. A click that does nothing is a defect, in a specimen that shows a pinned state or a look as much as in a live one. Every event a specimen's component emits is bound.

# Writing and Communication

- Explain so the reader understands the first time. Start with what happens in a concrete example, then the rule, in familiar words; explain a technical term on first use. For a problem or an ambiguity, show the exact situation, what each option would make happen, and why the difference matters. Say who does what, to which thing, and with what result.
- Distinguish verified facts, suspected problems, proposals, and open decisions.
- UI text follows macOS capitalization: the gallery's Writing page (`packages/web-gallery/src/sections/WritingSection.vue`) is the rule.
- UI text, in English and in every language Demi speaks, is chosen by comparing many products that say the same thing well, never by one source or a fixed rule: AI apps of the same kind (Codex, ChatGPT, Claude), the system's own apps (macOS, iOS), and developer tools such as VS Code only for developer concepts ([Localization](docs/product/localization.md)). Take what a user of that language reads without stopping; when the products differ, judge which is best and say why in the glossary. English is decided first; every other language says what the English says.
- Every document, decision record and comment follows the [Google developer documentation style guide](https://developers.google.com/style), and its Markdown the [Google Markdown style guide](https://google.github.io/styleguide/docguide/style.html).

# Design Documentation

- The code is the only authority on what Demi does and how. Whatever code can say, a type, a name, a test, the code says; a comment says only what the code cannot: a module's purpose and boundaries, an invariant, the reason for a non-obvious choice and the alternative it refused. A comment that restates what the code does is deleted.
- A module's design lives in its entry comment (`lib.rs`, `main.rs`, `index.ts`), next to the code it describes, with an ASCII diagram of at most 80 columns where a structure or a flow is clearer drawn. `ARCHITECTURE.md` holds only the map and the rules that hold across modules, and the dependency graphs.
- What a screen does and shows lives in the gallery and in the tests that protect it, never in prose.
- A new decision, one that changes what the user sees, a protocol or a module's boundaries, is written as a decision record (ADR) before it is built: `docs/decisions/NNNN-name.md`, numbered in order, following the minimal template of [MADR](https://adr.github.io/madr/). A decision that changes what the user sees draws the expected screen as an ASCII sketch. The lead shows it to the user, and it lands in its work package's commit. A merged decision record is never edited, except for its status; a later change is a new record that names the one it supersedes.
- A bug fix makes no new decision and needs no decision record: its commit message says the symptom, the cause and the fix, and it adds a test that fails before the fix. A fix that changes a module's structure, its state's owners or its boundaries, is a decision and has its record.
- `docs/` holds only what is not about one module: the overview, the comparison, guides, operator steps (setup, installation, upgrades), testing, product checks, the localization glossary, the decision records and the brand's assets.

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
- Doc comments also follow their language's conventions: in Rust, [RFC 505](https://rust-lang.github.io/rfcs/0505-api-comment-conventions.html), [RFC 1574](https://rust-lang.github.io/rfcs/1574-more-api-documentation-conventions.html) and the rustdoc book's [How to write documentation](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html); in TypeScript and JavaScript, the comment sections of the Google TypeScript and JavaScript style guides; in shell, the Google Shell Style Guide's comment section; in Python, [PEP 257](https://peps.python.org/pep-0257/).

# Project References

- `ARCHITECTURE.md` holds the map of the code, the rules across modules and the dependency graphs the boundary checks read; each crate's and package's role and limits are in its entry comment.
