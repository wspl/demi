# Working Principles

- Be pragmatic, never formalistic. Every step either moves the work forward or protects its correctness; drop ceremony that does neither, such as a check that cannot fail on the change at hand, a repeated full suite, or a report nobody needs.
- Tests follow [Testing](docs/delivery/testing.md). In short: each test protects a behavior a user or another component relies on, or a fixed bug, at the boundary where it is observable, once, with scenarios first; it fails before the fix or with the planted defect; it waits for events, never for time; it stays within its time budget and states its cost; it never restates the implementation or asserts a defect as correct. A test you touch that breaks these rules is fixed or deleted then.
- Follow the boy scout rule and decide on the spot. When you notice on the way something that slows the work or is wrong (a slow or duplicated build, a flaky test, a stale script, a wasteful habit), fix it then and say what you changed and why in the commit message, where the next session reads it. Do not stop to ask or save it for a review. Stop and ask only about what changes the agreed design or scope, cannot be undone, or reaches beyond the repository and its build products.
- Read the authoritative design before discussing changes. Inspect the implementation when needed to verify feasibility or investigate behavior; resolve discrepancies explicitly rather than treating code as an implicit design decision.
- Prefer simple, direct designs with clear responsibilities and explicit dependencies.
- Keep each fact defined in one place. Reuse existing code and contracts; consolidate duplication.
- Do not keep a second value that can be calculated from existing data. For mutually exclusive phases, use one status such as `idle | running | finished`, rather than separate `running` and `finished` flags that can contradict each other.
- Whenever you create a timer, listener, stream, or worker, check where it is stopped or released on success, failure, and cancellation. Share cleanup code when those paths need the same cleanup. If you ignore an error, make clear why it is safe to ignore.
- Implement the intended final design. Do not add compatibility layers or legacy-data migration, cleanup, or normalization paths.
- Before writing a helper, state its purpose in one generic sentence. If that sentence does not mention this project's domain, the helper almost certainly exists: search the standard library, the module's declared dependencies (`go.mod`), and the workspace, in that order (in the frontend: the package's declared dependencies, `@demicodes/utils`, and the workspace). Write it only when the search fails, and place it where the next caller will find it. Small size is not a reason to write a local copy.
- Use a library from its installed types and documentation, not from memory. Any cast (`as unknown as`, `any` or a type-only import in TypeScript; `unsafe`, `reflect` or an unchecked type assertion in Go) used to get around a library's types means you do not know its API: stop and read it. If the library genuinely lacks the capability, say so in a comment at the workaround.
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
- Every reusable UI behavior lives in `web-ui` as a component or primitive, and a plugin's feature UI lives in its plugin package, written only against `@demicodes/plugin-sdk` ([Plugin pages](docs/architecture/plugin-pages.md)); `web-ui` knows no plugin, and `web` and `web-gallery` supply only data, state, services and handlers. A behavior first built for one surface (a control's affordance, a page's interaction, a dialog flow) is generalized into `web-ui` before the checkpoint, never left local to the gallery, the product or one plugin.
- A control placed at a container's edge (an icon button at the end of a menu row, a settings row, a tab) keeps the same distance to every edge it touches: (container height - control height) / 2. The container's `web-ui` primitive computes that inset and offers a dedicated slot for such controls; a caller never positions a control through a generic slot with its own margin or padding.
- Keep `web` and `web-gallery` synchronized in both directions. Every UI change is made once, in `web-ui` or in the plugin package that owns it, whether the request came from the gallery or the product, and lands in both surfaces in the same checkpoint: update the gallery specimens and the product usage together, and verify the result in both. A change visible in only one of them is incomplete.
- Every control in the gallery responds when used. A specimen's button, menu item or link either acts on the specimen's own state the way the product would (Cancel drops the row, Clear empties the list, a dialog's buttons close it and a control opens it again), or simulates what the product would do with a host or another page and says so, for example in a neutral toast naming the action. A click that does nothing is a defect, in a specimen that shows a pinned state or a look as much as in a live one. Bind every event a specimen's component emits, and click through each new specimen before the checkpoint.
- Write code comments in English.
- Write separate steps on separate lines. Do not squeeze several assignments, branches, or cleanup actions into one line. A helper function should have a clear job; moving a complicated block into a vaguely named helper does not simplify it.
- Before committing, reread the complete functions you changed, not just the added lines. Check for repeated conditions, duplicate or unused values, ignored errors, and code in the wrong package. Fix those problems before calling the work complete, even when tests pass.
- When a batch of changes is ready for acceptance, check it yourself on processes started from the new code (the backend and the web front end, or the gallery) before reporting, and stop the processes you started once the check is done. Start them once per batch, after the whole batch is complete, not after every edit.
- The Cloud and a paired device are the same thing: a Host behind a runner. Code never distinguishes them except where the design says they differ (pairing, revocation, lifecycle). Anything that reaches a conversation's Host goes through the conversation's host access (see [Host operations](docs/execution/sessions-and-targets.md#host-operations)), which resolves the target, wakes a stopped Cloud, and holds the file gate. There is no other way to a conversation's Host, and every other way to a Host is named in that section; if the host access does not fit, change the design first.
- Cross-build with Go's own `GOOS` and `GOARCH`, and in development build and package only the targets of the Hosts in use (`docs/delivery/builds-and-releases.md`). All six targets are for a published release.
- Commit completed checkpoints with Conventional Commit subjects and push after each commit.

# Building and Testing

- One work package is one checkpoint, committed and pushed once. While writing, run only `go build ./...` and the test that covers the code; run `scripts/check.sh` on the work package's packages once at its end, and on `./...` when the change crosses packages.
- Build with `CGO_ENABLED=0` ([Toolchain](docs/delivery/builds-and-releases.md#toolchain)). The only exceptions are `go test -race` on Linux, which builds its test binary with `CGO_ENABLED=1 -tags netgo,osusergo`, and the purego FSEvents call in the tree watch's darwin file.
- Format the files you changed with `scripts/fmt.sh <files>`.
- A Go test that starts one of the repository's programs gets it from `internal/programtest`, which builds it once per test binary. TypeScript tests never build a program: `bun run test` builds them into `.cache/test-programs` and runs the suite in parallel; to run some tests, build first and set `DEMI_TEST_PROGRAMS=.cache/test-programs` ([Programs used by tests](docs/delivery/builds-and-releases.md#programs-used-by-tests)).
- Work in large steps: read what a step needs in one call, write the whole step, then compile once. Start long builds and tests in the background and keep working meanwhile.
- zsh does not split an unquoted variable into words; pass argument lists as arrays, or run scripts with bash.

# Go Code

- `context.Context` is the first parameter of every function that waits. Errors are returned, wrapped with `%w` and compared with `errors.Is` and `errors.As`, never by text. No panic crosses a package boundary.
- Every goroutine has an owner that cancels it and waits for it. Every lease, permit, timer, listener, file and process is released on success, failure and cancellation, with `defer` where it is acquired. No lock is held across a blocking call.
- Small interfaces are declared where they are used; constructors return concrete types. Another package's exported API changes in that package, for every caller, never through a local workaround.
- Contract types are Go types with `+demi:` markers; their decoders, encoders, validation and Zod are generated by `tools/contractgen` (`go generate`) and committed. Never write a second declaration of a contract shape by hand, and never decode outside input except through its generated decoder.
- A contract type's and field's doc comment is product text: the generated JSON Schema's `description`, which `--help` and the model's tool schemas show. Change it as product text; lint does not require it to start with the name there.
- Write wire JSON only through the generated encoders or `contract.EncodeJSON`, never `encoding/json.Marshal`: it re-escapes `<`, `>`, `&`, U+2028 and U+2029 and so changes bytes the contracts fix.
- Never build outgoing JSON from `map[string]any`: Go sorts its keys. An object Demi writes is a struct in its contract's field order; a JSON object Demi passes on as received (a `+demi:object` field) keeps its read order through `contract.ObjectFields` and `contract.EncodeJSON`.
- Tests use only the standard library, `github.com/google/go-cmp`, `go.uber.org/goleak` and `testing/synctest`; a test waits for events or synctest time, never for wall time.
- The repository holds no Rust: `scripts/check.sh` refuses `.rs` and Cargo files.

# Go Readability and Naming

Every rule here comes from a published Go source, cited in brackets; none is
this project's own taste. Where a source leaves a number open, the number is
the default of the community linter that checks it. Apply the rules exactly;
do not substitute habits of your own. The sources:
[EG] Effective Go, [CRC] Go Code Review Comments, [GSG] the Google Go Style
Guide (Decisions and Best Practices), [STD] the standard library's practice.
`scripts/check.sh` runs the checks a tool can make (`.golangci.yml`).

## Layout

- Format with `gofumpt`, a stricter superset of `gofmt` [EG: gofmt], and wrap
  long lines with `golines` (`scripts/fmt.sh <files>`).
- Go has no fixed line length; avoid uncomfortably long lines, and wrap by
  meaning, not at a column [CRC: Line Length; GSG: Line length]. The checked
  limit is `lll`'s default, 120 columns. A string a formatter cannot wrap is
  split where its meaning breaks: SQL by clause, a message by sentence.
- Keep functions focused; when one does several things, split it into
  functions named for those things [GSG: Function names; Best Practices].
  The checked limits are the linters' defaults: `funlen` (60 lines, 40
  statements), `gocognit` (30), `nestif` (5), revive `argument-limit` (8) and
  `max-control-nesting` (5). Test files are not held to `funlen`, `gocognit`
  and `nestif`: a scenario is a long sequence of steps. That exclusion is this
  project's configuration, the one choice here that no source makes. A
  `+demi:` contract marker is exempt from the line limit: like a
  `//go:generate` directive, it is one line the generator reads.
- [Owner] A function body that holds statements is never written on one
  line, declared or literal: `func() {` ends its line and `}` starts its own,
  as gofmt already does for `if`, `for` and `switch`. gofmt keeps a one-line
  function body as it finds it, so `tools/bodycheck` (in `scripts/check.sh`)
  refuses it and `go run ./tools/bodycheck -fix` rewrites it. An empty body
  may stay `{}`.
- Handle errors first and return early; keep the normal path at the left
  margin; no `else` after a `return` [CRC: Indent Error Flow].
- Many parameters, or several of one type, become an option struct [GSG: Function
  argument lists].

## Naming

- A package name is short, lower case, one word, with no underscores or
  mixedCaps, and names what the package provides; never `util`, `common`,
  `misc` or `helper` [EG: Package names; CRC: Package Names; GSG: Package names].
- An exported name does not repeat its package: `hostaccess.Error`, not
  `hostaccess.HostAccessError` [EG: Package names; GSG: Repetition].
- MixedCaps everywhere, constants included [EG: MixedCaps; GSG: Constant
  names]. Initialisms keep one case: `ID`, `URL`, `HTTP`, `userID`, `parseURL`
  [CRC: Initialisms]. Other abbreviations are used only when widely known
  [GSG: Naming].
- No `Get` prefix on getters (`Owner()`), `Set` on setters [EG: Getters].
- A one-method interface is the method plus `-er` [EG: Interface names], and
  an interface is declared where it is used [CRC: Interfaces].
- A receiver is a short abbreviation of its type, the same in every method,
  never `this` or `self` [CRC: Receiver Names; GSG: Receiver names].
- A name is as long as its scope needs: short in a few lines, descriptive far
  from its declaration [CRC: Variable Names; GSG: Variable names]. No type in
  the name (`userMap`, `idStr`) [GSG: Repetition].
- A function's name says what it does or returns [GSG: Function names]. Two
  conventions cover the common pairs [STD]: an exported function that only
  wraps an unexported one doing the same job shares its name in lower case
  (`Close` and `close`, as `os.File` does), and a function whose caller must
  hold a lock ends in `Locked` (`closeLocked`).
- A sentinel error is `ErrSomething`, an error type `SomethingError`
  [revive error-naming, following the standard library's `io.EOF`,
  `*fs.PathError`]. An error string starts lower case and
  ends without punctuation [CRC: Error Strings], except a message a user or
  the model reads as product text, which keeps its wording.
- A test names the behavior it checks, and reports `got` and `want`
  [CRC: Useful Test Failures].

## Comments

- Every exported identifier has a doc comment, a full sentence that starts
  with its name [EG: Commentary; CRC: Doc Comments] (contract product text
  excepted, as above).
- Comments explain why and what is not obvious, not what the code plainly
  says [GSG: Commentary]; an unexported function is documented when its
  purpose or contract is not obvious from its name and signature.

# Go Idioms

Some shapes read naturally to a Rust programmer but are not Go: a
result enum with a failure variant, a channel end with `Lagged` and `Closed`
variants, a future to wait on, a guard that cleans up when dropped. Go writes
each of them differently. The sources are those above plus [FAQ] the Go FAQ.

## Results and errors

- A function that can fail returns `(T, error)` with the error last, and `nil`
  means success [GSG: Returning errors; EG: Errors]. It never returns a value
  whose variants are success and failure, such as a `Written` that is either a
  revision or `Refused{Err error}`: a failure carried inside the result is an
  in-band error, and the caller can forget to check it [GSG: In-band errors;
  CRC: In-Band Errors]. Go left variant types out on purpose; for the error
  case the FAQ points to an interface value holding the error [FAQ: Why does
  Go not have variant types?].
- A result that may be absent returns `(T, bool)`, the comma-ok form, when the
  caller needs no explanation [GSG: In-band errors; EG: comma ok], as
  `os.LookupEnv` does [STD]. A pointer that exists only to say "absent" (a
  `*bool`, a `*string`, a pointer to a copy of a value) is Rust's `Option` and
  an in-band signal. A lookup of an object that callers share by pointer may
  return nil for "not found", as `flag.Lookup` and `template.Lookup` do [STD];
  so may a nullable field's value that the caller stores as it is.
- A failure a caller must tell apart is a sentinel, `var ErrInUse =
  errors.New("...")`, returned wrapped with `fmt.Errorf("...: %w", ErrInUse)`
  and tested with `errors.Is` [GSG Best Practices: Error structure, Sentinel
  error placement; STD: `os.ErrNotExist`, `sql.ErrNoRows`, `io.EOF`]. An error
  type exists only when a caller reads its data with `errors.As`, as with
  `*fs.PathError` [GSG Best Practices: Error structure]. That section gives
  an error structure "if callers need to interrogate the error", so a `Kind`
  or `Reason` field that no caller branches on is removed: the message
  describes the failure, and a caller that only prints the error needs
  nothing else.
- Two successful results that differ (stored, or already there) are told
  apart by an extra result value, as `sync.Map.LoadOrStore` returns `loaded
  bool` [STD; GSG: In-band errors]. A condition the caller treats as a failure
  (still in use) is an `error` [GSG: Returning errors], not a success
  value with a flag.
- A closed set of types used as data stays an interface with a type switch:
  wire messages, stream events, JSON unions, as `go/ast.Expr` does [FAQ: Why
  does Go not have variant types?; STD]. The rules above are about what a
  function returns to say how it went, not about data that is sent or stored.

## Concurrency

- A stream is received from a `<-chan T` that the sender closes at its end
  [GSG Best Practices: Channel direction; EG: Channels], or read with an
  iterator, `Next() bool` then `Err() error`, as `bufio.Scanner` and `sql.Rows`
  do [STD]. A receive that returns `Frame`, `Lagged` or `Closed` variants is
  Rust's channel API: lag and end are errors from `Err()`, or values the
  channel carries. A non-blocking receive is a `select` with a `default`, not
  a `TryReceive` method.
- A function returns its result when the work is done [CRC: Synchronous
  Functions]. An object whose only use is a `Wait()` right after it is
  returned is a future; the function blocks instead. A `Start` and `Wait`
  pair stays only where callers do real work between the two, as with
  `exec.Cmd` [STD].
- Every goroutine's end is plain from the code that starts it [CRC: Goroutine
  Lifetimes].

## Names and cleanup

- A type is named for what it is, as `os.File`, `net.Conn`, `exec.Cmd` and
  `http.Server` are [STD], not for a Rust ownership role: no `Handle`, `Guard`,
  `Sender` or `Receiver` suffix unless the type really is one end of a
  channel pair.
- Cleanup is a `Close` or `Release` method, or a returned function as
  `context.CancelFunc` is, that the caller runs with `defer` where it
  acquired the resource [EG: Defer; STD]. Go has no destructor, so there is no
  guard object that cleans up when it goes out of scope.
- A constructor returns a ready value; where a zero value can be useful, it is
  [EG: Allocation with new]. A `DefaultX()` that only fills fields is kept
  only when the zero value cannot be made to work.

## Comments

- A comment describes Go behavior. It never mentions Rust or a Rust library
  (`tokio`, `serde`, `axum`, ...), which the repository no longer has [Owner].
  When the reason for a choice is a wire format or a behavior fixed
  elsewhere, the comment states that format or behavior itself.

## What stays

These look like Rust to a Rust reader but are Go practice: `Unwrap() error`
methods [STD: `errors`], `MustX` functions [STD: `regexp.MustCompile`],
getters without `Get` [EG: Getters], and a panic for an invariant that only a
bug can break, which never crosses a package boundary [GSG Best Practices:
When to panic].

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

- Go: Effective Go, Go Code Review Comments, the Google Go Style Guide and the standard library's practice, as [Go Readability and Naming](#go-readability-and-naming) cites them; `gofumpt` and `golines` formatting; linter defaults for every checked number.
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

- `docs/architecture/packages.md` is the authoritative contract for package responsibilities, dependencies, and module layout.
