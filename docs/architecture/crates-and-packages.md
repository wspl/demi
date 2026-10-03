<a id="crates-and-packages"></a>

# Go and TypeScript packages

This document is the boundary contract for every Go package and every
TypeScript package in the repository, and the highest architectural constraint
on changes to them. It names each package, its public items, what it owns and
what it must not do. Other documents describe behavior and link here for
ownership.

The two dependency graphs in [Dependency graphs](#dependency-graphs) are the
required set of Go and TypeScript packages and the edges between them. Go
imports and TypeScript manifests follow the graphs: when an import or
`package.json` disagrees with its line, the implementation changes, or this
document changes first through a design change. The [boundary
checks](#boundary-checks) fail until both agree.

## Extension principle

Every capability of the agent is a [plugin](plugins.md), and the plugin
contract is all the agent runtime, the backend and the web app know about it: a
plugin contributes commands, the model's context, Host directories and a part
of the page through messages, and nothing in those components is about one
plugin. The agent runtime supplies its own tools and the `demi agent` and `demi
shell` groups; the product supplies the conversation's Host and the `demi host`
group; everything else the agent can do is a plugin's.

A capability that keeps state on the Host is, in addition, a command package
behind the runner's narrow port, and that port is all the runner and the
backend know about it. The runner and `internal/cmdsdk` supply mechanism: the
trusted conversation and caller identity on every invocation, and the generic
conversation release with its status query ([Conversation-scoped
state](../execution/native-runtime.md#conversation-scoped-state)). The backend
owns one policy: when a conversation is idle ([Conversation idle and Host
resource release](../execution/resource-lifecycle.md)). Each tool owns its own
state and cleanup.

For example, the conversation browser keeps its tabs between shell jobs. Its
operations are types in `internal/cmdpkg/browser/browserop`, the `demi.browser`
package (`internal/cmdpkg/browser` with the Chrome subpackages) implements
them, and `internal/plugins/browser` declares them as `demi browser` commands.
The tabs are keyed by the conversation the runner names on every invocation,
and they end when the runner forwards the generic conversation release. No
browser state or browser-specific message exists in the runner's packages,
`internal/runnerwire`, `internal/backend/remotehost`, the agent's packages or
the backend's conversation lifecycle. The backend does not name the browser at
all: where the user's page reaches the browser directly,
`internal/plugins/browser` declares the live view's `browser` user stream and
serves the tab methods, which call the browser's own operations and hold no
browser logic or state.

A new capability is therefore a plugin of its own: an `internal/plugins/<name>`
package, and, when it keeps state on a Host, a command package with its
operation types in a contract package and its commands in its program, which
the plugin's `native` leaves bind. The runner, `internal/runnerwire`,
`internal/backend/remotehost`, the agent, `internal/backend/plugins` and the
backend's conversation lifecycle do not change. A proposal that adds a
tool-specific message, adapter or lifecycle hook to any of those packages
violates this contract and is redesigned at the plugin contract or the runner's
port instead. A generic mechanism is different: a message the runner answers
without knowing who asked or why, such as a filesystem operation, a
working-tree listing or a network stream, belongs to the runner wire like the
others, and an operation any plugin could use, such as a Host directory,
belongs to the plugin contract.

## Dependency direction

Dependency direction is an architecture invariant. A lower-level package does
not know the products, adapters, user interfaces, concrete providers or machine
implementations above it.

- The entries below are the single source of each package's responsibilities
  and boundaries. Do not scatter package-specific rules across other sections.
  The graphs are the single source of dependencies; the entries do not repeat
  them.
- When a package is added, removed, renamed or split, its entry and its graph
  line change in the same change.
- Production code never depends upward, and neither do tests: a test reaches a
  fake through the test-support package of the package that owns what it fakes
  ([Module layout](#module-layout)), and no test imports an implementation that
  depends on its subject.
- A package's public items are the ones its entry names; everything else is
  unexported. A TypeScript package exposes only what its entry names as its
  public boundary.
- Do not keep compatibility shims when a split moves an implementation to its
  final package.

<a id="crates"></a>

## Go package boundaries

One module, `github.com/wspl/demi`, contains the first-party packages below, in
migration package-map order. Paths are relative to its root. Each entry
replaces its mapped Rust crate's boundary; a split distributes the original
boundary without another implementation. `shared-cli` becomes `internal/cli`.

Plugins are backend-linked libraries. They use the plugin contract and contract
packages, never a backend implementation or another plugin.

The browser and machine-manager split keeps the original external dependency
boundaries; their subpackage entries assign each former module one owner.

Contract packages hold Go types with `+demi:` markers and their generated
encoders, decoders and validation. They perform no IO and own no workers.
[Contracts](contracts.md) owns the generation and validation rules.

<a id="shared-types"></a>

<a id="contract-crates"></a>

#### `internal/core`

- **Owns:** the data the web app, the agent and the backend share:
  - transcript blocks (`Block`): the explicit submission `User`, the only
    editable type; `Context`, `Wakeup`, `Steer` and `AgentMessage` inputs;
    `Resume` and `Abort`; provider output; `CompactionBoundary` and
    `CompactionMarker`;
  - user and tool content (`UserContentBlock`, `MediaSource`, `DocumentSource`,
    `ToolResultContentBlock`, `ToolMediaSource`), whose media are blob
    references (`BlobRef`), or a URL, and never bytes; a tool result's medium
    that is gone, with why (`GoneCause`); the tag that names an attachment to a
    model (`AttachmentTag`), the one test of a blank text with the trim that
    goes with it (`IsBlank`, `Trim`), and where a cut after a number of
    characters falls in a text (`CharOffset`), which the transcript, the title
    and attachments cut texts at;
  - models and their selection (`Model`, `ModelSelection`, `ThinkingConfig`)
    and token usage (`TokenUsage`);
  - tool views (`ToolView`, `ShellToolView`, `OutputChunk`, `EditedFile`) and a
    command's output views, which the shell status frame and the shell contract
    are built from (`StreamView`, `OutputView`, `BinaryStdout`);
  - agent messages (`AgentMessage`, `CompletionID`);
  - the session phase, queued messages and pending steers;
  - provider failure facts (`ProviderFailureFacts`,
    `ProviderErrorDiagnostics`);
  - what the product shows of a provider entry: its model catalog
    (`ProviderModelList`, `ProviderModel`, `ServiceTier`, `ModelCost`), which
    carries portable facts only and never a source label such as
    `codex-backend`, `models.dev` or `cache`, with the one conversion of a
    catalog model into a selection (`ProviderModel.Selection`, with the
    attachment types it derives, `AttachmentFileExtensions`); the wire format
    an `openai` entry speaks (`WireAPI`), which its configuration names and its
    provider sends; its authentication and runtime states (`AuthState`,
    `RuntimeState`); its subscription accounts as the web app sees them
    (`AccountInfo`, `LoginPending`); and an account's quota snapshot
    (`QuotaSnapshot`, `QuotaWindow`, `QuotaPlan` and their sets);
  - the identities blocks and frames name (`BlockID`, `TurnID`, `NodeID`,
    `WakeupID`, `ShellID`, `CommandID`, `OperationID`), with generated `Parse`
    constructors and boundary validation: string types serialize as strings,
    and validated inputs meet their checks;
  - the file-type table the product previews by (`PreviewMediaType`,
    `ShowsInPlace`), and the media types a model accepts with their sniffing
    (`SniffModelMediaType`);
  - base64 bytes (`B64Bytes`), times (`Timestamp`), the wall clock times are
    read from (`Clock`, `SystemClock`), and their generated codecs. Nullable
    fields and decode rules are `+demi:` markers; the generic strict reader and
    field-path errors belong to `internal/contract`, used by every generated
    boundary decoder.
- **Public boundary:** the types and functions above.
- **Must not:** contain concrete provider names, catalog source names, shell
  runtime details, Host details, user-interface concepts, transport URLs or
  backend identifiers.

#### `internal/contract`

- **Owns:** the generic runtime generated contract code calls: strict object
  reading, required-field presence and duplicate-key checks, UTF-8 checks and
  field-path errors. Domain shapes remain in their contract packages.
- **Public boundary:** strict readers, presence tracking and decode errors, as
  specified in [Contracts](contracts.md).
- **Must not:** declare domain contracts, perform IO, generate code or know
  providers, products or transports.

<a id="conversation-socket-protocol"></a>

#### `internal/framewire`

- **Owns:** the conversation WebSocket's frames: `ClientFrame` and its content
  (`ClientContent`, whose `upload`, `remote_file`, `media` and `attachment`
  variants refer to files rather than carry them) with the edit request
  (`EditRequest`), `ServerFrame`, `TranscriptPatch`, `TranscriptVersion`, the
  nested edit and steer outcomes (`EditOutcome`, `SteerOutcome`), `SubagentJob`
  and `ShellStatus`; and the decode function of client frames
  (`DecodeClientFrame`), which tells a message that is not JSON from an invalid
  frame.
- **Public boundary:** the types above. The agent resolves the files a frame's
  content refers to through the backend (`internal/agent/server`'s
  `ContentResolver`) before the session sees the content. The protocol's
  behavior is in [Frame protocol](../agent/runtime.md#frame-protocol).
- **Must not:** hold session logic or a transport, or carry file bytes inside a
  frame.

<a id="command-protocol"></a>

#### `internal/commandwire`

- **Owns:** the wire between an execution host and a command program:
  - invocation metadata (`Invocation`, `LocalInvocation`), the command context
    (`CommandContext`, `CommandCaller`, `CommandLocale`), completion
    (`Completion`), service information (`ServiceInfo`), the conversation
    release and status requests, and the conversation numbers stream's messages
    (`NumbersRequest`, `NumbersAnswer`);
  - record framing (`Record`, `RecordDecoder`);
  - package descriptors and their identities (`PackageDescriptor`), artifact
    locations and target triples (`TargetTriple`), and the one canonical digest
    of a JSON value (`CanonicalDigest`: the SHA-256 of its RFC 8785 form),
    which also identifies the agent's edit requests;
  - the edit journal and its context (`EditContext`, `EditJournal`), and the
    one test of whether bytes are text (`IsText`), which edit tracking and line
    counts read at both ends.
- **Public boundary:** the types and functions above; test support is in
  `internal/commandwire/commandwiretest`.
- **Must not:** speak the wire: the client, the server and the recorder are
  `internal/cmdsdk`'s.

<a id="command-declarations"></a>

#### `internal/declare`

- **Owns:** command declarations, which are also the manifest's nodes (`Node`,
  `Group`, `Leaf`, `LeafKind` with its `RPC` and `Native` bindings: a
  declaration names a native command's package and operation
  (`NativeOperation`), and `Node.Pin` pins each to the descriptor a manifest
  carries, as a `Binding`); a declaration's schemas, compiled once, and the one
  check of a value against a schema with its one wording (`Schema.Check`); the
  command input subset (`CheckInputSubset`, which registration runs); argv
  parsing (`Node.Select`, `Selected.Parse`, `Parsed.Validate`), which reads
  each field's schema to convert its tokens; the argument check both ends run
  (`Leaf.CheckArguments`); help rendering (`Node.Help`, `HelpDefaults`); and
  the settings every declaration's JSON Schema is generated with
  (`CommandSchemaSettings`).
- **Public boundary:** the items above. This package is the single
  implementation of argv parsing, help and the input subset: the runner parses
  argv and renders `--help` with it, and the backend renders the model's
  command help and checks registrations with it. Argument validation is the
  JSON Schema validator over the declaration's schema at both ends, through the
  one check and its one wording; the runner checks `--json` output against the
  leaf's output schema with the same check. Behavior:
  [Commands](../execution/commands.md).
- **Must not:** hold handlers or their bindings (`internal/host` pairs
  declarations with handlers), read stdin, or perform any other IO.

<a id="runner-protocol"></a>

#### `internal/runnerwire`

- **Owns:** the wire between the backend and a runner:
  - `Inbound` and `Outbound` messages and their codec
    (`runnerwire.DecodeInbound`, `runnerwire.DecodeOutbound`,
    `runnerwire.Encode`);
  - the manifest envelope (`Manifest`): its verification and canonical hash,
    and `BuildManifest`, so both ends compute the hash one way;
  - the managed boot record (`ManagedBoot`) and the runner release record
    (`RunnerRelease`);
  - `Signal`, and the platform a runner reports in its hello
    (`RunnerPlatform`);
  - the protocol constants: `Version`, `MaxMessageBytes`, `JobViewBytes`,
    `JobLiveBytes`, `JobLiveInterval`, `JobGrowthInterval`, `StdinChunkBytes`,
    `LogReadLines` and `ServiceStderrChars`;
  - where a Cloud image embeds the command package executables that the runner
    starts in place of downloading them (`runnerwire.ArtifactsPath`).
- **Conversation scope:** jobs and service streams carry the [command
  context](../execution/native-runtime.md#command-context);
  `ConversationRelease` is the one generic release message, and
  `NumbersReserve` the one generic request for a conversation's numbers; the
  wire carries no conversation browser policy.
- **Public boundary:** the items above. The runner and the backend link it; the
  machine manager uses `ManagedBoot`, and `internal/machinewire`'s image
  manifest check and `tools/release`'s image build use `ArtifactsPath`.
  Behavior: [Runner](../execution/runner.md).
- **Must not:** contain network IO, a Host implementation, a shell environment,
  the job table, credentials, claim policy, the device registry or conversation
  state.

<a id="machine-manager-protocol"></a>

#### `internal/machinewire`

- **Owns:** the machine-manager socket: requests (`MachineRequest`,
  `MachineCall` with one parameter type per operation), responses
  (`MachineResponse`: `ok`, `error` and `death`), each operation's result
  (`Operation.Output`) and the line codec (`DecodeRequest`, `DecodeResponse`,
  `EncodeLine`, `MaxLineBytes`); machine image state (`MachineImageState`,
  `RuntimeState`, `Volume`) and the names of stored images (`DeviceID`,
  `GenerationID`, `BaseVersion`: one path component each); and the Cloud image
  manifest (`CloudImageManifest`), which embeds `internal/runnerwire`'s runner
  release and `internal/commandwire`'s package descriptors.
- **Public boundary:** the items above. The backend's machine-manager client
  and the manager link it; `tools/release` writes the image manifest with it.
  Behavior: [Managed Cloud hosts](../cloud/managed-hosts.md).
- **Must not:** hold the manager's policy or IO.

<a id="web-api-protocol"></a>

#### `internal/webapi`

- **Owns:** every REST request and response body, such as `ProductState`,
  `ConversationPatch` and `ProviderDTO`; the messages of the page's
  synchronization channel (`SyncEvent`); the error body (`ErrorBody`) and
  `ErrorCode`, the one list of every error code the web app can see; and the
  identifier and text types those bodies use. It reuses the runner's
  working-tree change types (`GitChanges`) and platform (`RunnerPlatform`, a
  device's platform) and `internal/commandwire`'s command locale
  (`CommandLocale`, which the web app reports as a preference) instead of
  declaring them again. A Host log line is its own type: the runner wire
  carries its time as integer milliseconds, and the web app receives it as an
  RFC 3339 time.
- **Public boundary:** the types above; their TypeScript form is generated into
  `web`. Behavior: [Web API](../product/web-api.md).
- **Must not:** hold route handling or domain logic.

<a id="command-package-file-protocol"></a>

#### `internal/cmdpkg/file/fileop`

- **Owns:** the `demi.file` package's id (`Package`, which its release
  descriptor names) and operations (`Operation`): the arguments and results of
  `file.read`, `file.create`, `file.edit` and `file.patch`, with their limits.
- **Public boundary:** the types above. `internal/plugins/file` declares the
  `demi file` commands from them and `demi-file` decodes invocations with them.
  Behavior: [Commands](../execution/commands.md).
- **Must not:** implement operations or perform IO.

<a id="command-package-browser-protocol"></a>

#### `internal/cmdpkg/browser/browserop`

- **Owns:** the `demi.browser` package's id (`Package`) and operations
  (`Operation`): the arguments and results of `browser.*`, browser targets,
  queries, tab state, observations and resource event payloads, the live view's
  messages and frame header, the capture extension's events and commands, and
  the pinned Chrome release record. Limits are the package's constants, shared
  by the inputs they bound; each input type carries its default timeout.
- **Public boundary:** the types above. `internal/plugins/browser` declares the
  commands from these types, the browser's packages decode invocations with
  them, the plugin uses them for its tab methods too, and the page reads the
  live view's messages through `@demicodes/plugin-browser`. Native commands,
  the plugin, the backend and the page need one contract without importing each
  other's implementations.
- **Must not:** implement operations, transport, components, Host access,
  process management or conversation persistence.

<a id="command-package-claude-code-protocol"></a>

#### `internal/cmdpkg/claudecode/claudecodeop`

- **Owns:** the `demi.claude-code` package's operations, `claude-code.ensure`
  and `claude-code.status`: their arguments and results, the release record and
  the install status.
- **Public boundary:** the types above. The backend builds the release record,
  and `demi-claude-code` decodes it. Behavior: [Claude
  Code](../providers/claude-code.md).
- **Must not:** hold install logic.

<a id="command-sdk"></a>

<a id="libraries"></a>

#### `internal/cmdsdk`

- **Owns:** the SDK that speaks the command wire of `internal/commandwire`:
  - the HTTP/2 client and server, the one invocation exchange (`Exchange`),
    bounded invocation IO and handler cancellation, and the handler's
    conversation numbers (`Numbers`);
  - the invocation edit recorder: bounded file snapshots and a journal
    coordinated across processes by an OS file lock;
  - path resolution against an invocation's working directory, and waiting out
    a lack of open file descriptors;
  - a command program's launch argument, `--command-service`, which the program
    reads (`Launch`), and the handler's artifacts source (`Artifacts`), which
    asks the runner to install an artifact for the invocation or says which of
    a line it has.
- **Public boundary:** the client, the service entry point, the handler and IO
  interfaces, the launch argument, the artifacts source, and the edit recorder;
  test support is in `internal/cmdsdk/cmdsdktest`. The runner and every command
  program use this one SDK; a command program depends on it and on
  `internal/commandwire` without depending on the runner or on Demi's command
  implementations.
- **Must not:** implement commands, download artifacts, start command
  processes, hold credentials, or change the process-wide working directory or
  environment for an invocation.

<a id="command-package-file-demi-file"></a>

#### `internal/cmdpkg/file`

- **Owns:** the independently released `demi.file` resident program: file read,
  create, edit and patch, with the file mutations serialized by one gate and
  recorded through the edit recorder ([Edit
  tracking](../execution/edit-tracking.md)). It holds no conversation state, so
  it answers every conversation status as empty and ends with its last lease.
- **Public boundary:** the service entry point wired by `cmd/demi-file`.
  Behavior: [Commands](../execution/commands.md).
- **Must not:** host a runner connection, define the agent's command tree,
  store conversations or be linked into the runner. The runner shell invokes
  system utilities.

<a id="command-package-browser-demi-browser"></a>

#### `internal/cmdpkg/browser`

- **Owns:** the independently released `demi.browser` resident program: the
  conversations' browsers, one owner per conversation, each with the live view
  hub of the browser it runs, routing each invocation by the package's
  operation list, the `capabilities` command, which is the one place that knows
  every command family, the sweep of orphaned profiles at start, and the
  composition of the Chrome subpackages below.
- **Public boundary:** the service entry point wired by `cmd/demi-browser`.
  Behavior: [Conversation browser](../browser/browser.md), whose
  [catalog](../browser/browser.md#catalog) lists the browser commands, and
  [Live view](../browser/live-view.md). Its acceptance tests and the backend
  browser suite use the Chrome test-support packages and
  `internal/artifacts/artifactstest` to install pinned Chrome.
- **Must not:** host a runner connection, define the agent's command tree,
  store conversations or be linked into the runner.

<a id="command-package-browser-chrome"></a>

<a id="browser-library"></a>

#### `internal/cmdpkg/browser/chrome/cdp`

The package map has no separate driver package. Low-level driver operations
live here, launch and environment ownership in `tabs`, and page actions and
viewer behavior above them. A CDP operation receives a target or executor,
not a tab registry, so the split introduces no dependency cycle.

- **Owns:** Chrome's bounded CDP transport, session router and event pumps
  behind `cdp.Executor`, using cdproto types; low-level driver operations,
  element handles, frames, navigation history, conversation numbers, command
  text and output, cancellation and failures (`Operation`); raw CDP commands,
  pinned-protocol validation, debugging connections and WebMCP validation.
  Callers supply the target and per-tab debugging state, without an import of
  the tab registry. JSON Schema validates pinned and page-declared schemas.
- **Public boundary:** `Executor`, connections, session handles, operation
  context, raw command and WebMCP functions, capabilities and errors.
- **Must not:** own tab state, conversations or viewers. Debugging connections
  belong to the calling agent and are released when their tab ends.

#### `internal/cmdpkg/browser/chrome/tabs`

- **Owns:** the Chrome executable installed as the `chrome` resource, launch
  with the capture extension and capture channel, its extension connection,
  process tree and profile; the environment's CDP pump; the tab registry and
  snapshot; each tab's state and gate, references, asset inventories, WebMCP
  tool sets, console buffer and access to its debugging owner; viewports,
  dialogs, console logs and navigation. This includes the former driver's
  launch, installation and capture connection responsibilities.
- **Public boundary:** `Environment`, `Tab`, registry snapshots, lifecycle,
  navigation and the capture connection used by the live view. Low-level launch
  code knows no tabs, conversations or viewers.
- **Must not:** act on page content except to navigate, or know the live view
  or conversation ownership. Features above it act on its tab data.

#### `internal/cmdpkg/browser/chrome/page`

- **Owns:** element location and state, evaluation, observation, queries and
  probes, content and screenshots, keyboard, pointer, selection and select
  options, combined actions, clipboard, uploads, downloads, fetches and assets,
  and these families' capabilities.
- **Public boundary:** one function per action over a `tabs.Tab`, with
  cancellation and errors from the operation context.
- **Must not:** own tab state or a CDP session; tabs and the executor supply
  them.

#### `internal/cmdpkg/browser/chrome/live`

- **Owns:** the [live view](../browser/live-view.md): capture pipeline, frame
  rate and pacing, user-stream viewers, each watched tab's page observer and
  relay of user input to the tab.
- **Public boundary:** the hub started with an environment and the viewer's
  browser interface, implemented by `internal/cmdpkg/browser`.
- **Must not:** know how conversations are owned or released. The browser
  program starts one hub for each browser it owns.

<a id="command-package-claude-code-demi-claude-code"></a>

#### `internal/cmdpkg/claudecode`

- **Owns:** the independently released `demi.claude-code` package, which
  installs and verifies Demi's copy of the Claude Code CLI on the machine that
  runs it ([The package](../providers/claude-code.md#the-package)).
- **Public boundary:** the service entry point wired by `cmd/demi-claude-code`.
- **Must not:** read release pointers or choose a version, start the CLI, or be
  linked into the runner or another command program.

<a id="shared-gates"></a>

#### `internal/gates`

- **Owns:** the named gates that serialize work across waits: `Activity`
  (demand and maintenance leases, reservations and its `State` snapshot),
  `Hub`, `Serial` and `KeyedSerial`.
- **Public boundary:** the types above. Their semantics are in
  [Locks](concurrency.md#locks). Test support is in `internal/gates/gatestest`.
- **Must not:** know users, conversations, devices or any other domain.

<a id="shared-artifacts"></a>

#### `internal/artifacts`

- **Owns:** verified download over HTTPS with a declared size and SHA-256
  (plain HTTP too for the runner's artifact cache, whose digests come from the
  pinned descriptor), and the measured download that establishes them when a
  release is prepared; digests; atomic publication, durable when asked; release
  publication (a directory of verified files and the record that describes
  them, published once and immutable); the install lock between processes;
  install receipts; and archive installation (a verified zip archive unpacked
  into a directory named by its SHA-256, with the receipt that checks it before
  each use, or that its installer's own cache trusts), which installs a command
  package's resources, such as Chrome for Testing, into a runner's artifact
  cache and into the Cloud image. Every download-and-verify path and every
  atomic publication of a file, durable or not, goes through it: the runner's
  artifact cache, the Claude Code installer, the machine manager's image store,
  the edit recorder's snapshots and journal in `internal/cmdsdk`, and
  `tools/release` release packaging.
- **Public boundary:** the functions and types above. Test support is in
  `internal/artifacts/artifactstest`.
- **Must not:** choose what to install or read release pointers: its callers
  name the location, size and digest they expect.

<a id="shared-cli"></a>

#### `internal/cli`

- **Owns:** what every program's command line shares: a variable under the
  program's prefix that none of its settings reads stops startup, naming it, so
  a misspelt setting is never ignored. The backend refuses an unknown `DEMI_*`
  variable and the machine manager an unknown `DEMI_MANAGED_*` one.
- **Public boundary:** `UnknownVariable`, over the program's declared settings.
- **Must not:** parse a program's settings, which its configuration parser
  does.

<a id="host-interface"></a>

#### `internal/version`

- **Owns:** the release version (`Release`), the one statement of Demi's
  version ([Package versioning](../delivery/package-versioning.md)).
- **Public boundary:** `Release`.
- **Must not:** import anything or compute the version at run time.

#### `internal/host`

- **Owns:** three contracts and nothing that implements them:
  - the Host contract: `Host` (`Key`, `DefaultCWD`, `Identity`, `FS`,
    `Process`), `FS`, `Process` and `Key`, the value identity of an execution
    target (equal keys mean the same Host);
  - the command system: `CommandSet`, which pairs declarations with their
    bindings and is what manifests and help read; the declaration builders; the
    rpc handler interface (`RPCHandler`, `RPCInvocation`, `RPCPort`,
    `PortTransport`, `StorageOp`); the reserved command names;
  - the shell-environment contract behind the `shell_*` tools:
    `ShellEnvironment`, `ExecRequest`, `CommandStatus` and `CommandRecord`,
    which keeps the model's place in each command's output apart from the
    pages' view of it (`PageView`), and the feed through which an environment
    reports that view's changes and learns whether a page watches (`PageFeed`,
    [Live output](../agent/runtime.md#live-output)); the reading of a running
    command's kept output; a command's whole output (`WholeOutput`) and its
    lines of text (`OutputText`), which the result that reports a command's end
    and `demi shell output` read alike; where an environment takes the numbers
    of its commands and shells, the conversation's sequences (`Numbers`,
    [Identifiers the model
    sees](../agent/runtime.md#identifiers-the-model-sees)). The keeper to
    which the remote environment hands what a command leaves when it ends is
    `internal/backend/remotehost`'s: its argument is the runner wire's job file
    change ([The whole output](../agent/runtime.md#the-whole-output)).
- **Public boundary:** the items above; test support is in
  `internal/host/hosttest`. The Host rules are in [Host
  operations](../execution/runner.md#host-operations); the handler interface is
  [the TypeScript boundary](contracts.md#the-typescript-boundary).
- **Must not:** depend on the agent's packages, `internal/provider`, a concrete
  provider, a plugin or a Host implementation.

<a id="plugin-interface"></a>

#### `internal/plugin`

- **Owns:** the [plugin contract](plugins.md#the-contract) and nothing that
  implements it: the factory and instance interfaces (`Factory`, `Plugin`); the
  manifest (`Manifest`): a plugin's id, its command groups and roots as
  declarations with their placement, whether it is a context source, its
  profiles as data, and its page state and page methods with their schemas; the
  requests and replies; the port (`Port`) with its messages and the transport
  they travel through (`Transport`), whose command operations are the rpc
  port's; and the plugin's errors.
- **Public boundary:** the items above; test support is in
  `internal/plugin/plugintest`.
- **Must not:** implement a plugin or a transport to a process, depend on the
  agent's packages or a backend package, or carry a live object in any message.

<a id="plugin-browser"></a>

<a id="plugins"></a>

#### `internal/plugins/browser`

- **Owns:** the `demi browser` group, declared from
  `internal/cmdpkg/browser/browserop` types, every leaf bound to a
  `demi.browser` operation ([Command
  contract](../browser/browser.md#command-contract)); the `browser` user
  stream, bound to `browser.live`, with its messages and frame constants; and
  the tab list, its conversation state, and the tab methods, which call the
  browser's operations as package calls ([The tab
  methods](../browser/live-view.md#the-tab-methods)).
- **Public boundary:** its factory, whose manifest names its page package,
  `@demicodes/plugin-browser`, which `tools/contractgen` generates the page's
  types into.
- **Must not:** implement a browser operation or hold browser state; the
  conversation browser's packages do.

<a id="plugin-changes"></a>

#### `internal/plugins/changes`

- **Owns:** the `changes` plugin's identity: a manifest with its id, name and
  description and no contribution, so the user's switch decides whether the
  page shows the Change view ([Registration](plugin-pages.md#registration)).
- **Public boundary:** its factory.
- **Must not:** hold or serve a change; edit tracking and the file routes do.

<a id="plugin-expose"></a>

#### `internal/plugins/expose`

- **Owns:** the `demi expose` group and its `rpc` handlers, the numbers the
  model sees and the values that keep them, the one-hour policy, and its page
  state and the `Renew` and `remove` methods ([Host
  expose](../execution/expose.md)).
- **Public boundary:** its factory, whose manifest names its page package,
  `@demicodes/plugin-expose`, which `tools/contractgen` generates the page's
  types into.
- **Must not:** hold an expose record or relay a byte;
  `internal/backend/expose` does.

<a id="plugin-file"></a>

#### `internal/plugins/file`

- **Owns:** the `demi file` group, declared from `internal/cmdpkg/file/fileop`
  types, every leaf bound to a `demi.file` operation.
- **Public boundary:** its factory.
- **Must not:** implement a file operation; `demi-file` does.

<a id="plugin-file-browser"></a>

#### `internal/plugins/filebrowser`

- **Owns:** the `file-browser` plugin's identity, as `internal/plugins/changes`
  owns its own, for the File view.
- **Public boundary:** its factory.
- **Must not:** read a Host file; the file routes do.

<a id="plugin-skills"></a>

#### `internal/plugins/skills`

- **Owns:** [skills](../agent/skills.md): sources with their fetch, pin and
  limits, through go-git with cancellation; the skills they hold; the values
  and blobs it keeps of them; the Host directories of the skills that are on;
  its context blocks with the catalog of the skills that are on; the project
  skills it finds in a conversation's repository; and its page state and page
  methods.
- **Public boundary:** its factory, whose manifest names its page package,
  `@demicodes/plugin-skills`, which `tools/contractgen` generates the page's
  types into.
- **Must not:** write to a Host, send a credential when it fetches, or hold the
  shard lock across blocking work.

<a id="plugin-todo"></a>

#### `internal/plugins/todo`

- **Owns:** the `demi todo` group (`list`, `add`, `update`, `done`), its `rpc`
  handlers and the validation of the todo list it keeps in the invoking node's
  command storage under `todos.json` ([Command state
  history](../agent/command-state-history.md)).
- **Public boundary:** its factory.
- **Must not:** keep state outside command storage.

<a id="provider-common"></a>

#### `internal/provider`

- **Owns:**
  - the provider contract: `Provider`, one entry shared by every user and
    request (identity, capabilities, authentication status, models, failure
    reading, quota, accounts and `Runtime`), and `Runtime`, one session's
    runtime (`Run`, `Fresh`, `Close`, and its vendor's request limits for a
    model, `RequestLimits`); `InferenceRequest`, with the transcript as it
    carries it (`InferenceItem`), whose parts hold each medium's bytes, or a
    URL, and never a reference (`UserPart`, `ResultPart`, `MediaBytes`; a tool
    returns its result as `ResultPart`s too), and how it extends the session's
    earlier requests (`PromptCache`), `Event`, `Failure` and its `ErrorCode`;
  - the HTTP failure record and its standard reading (`HTTPFailureRecord`,
    `HTTPFailure`, `ReadHTTPFailure`), and the text of a credential a provider
    holds (`Secret`), which never prints;
  - the vendor-wire building blocks every HTTP provider decodes with:
    server-sent events, the two-step decode of payloads tagged by `type`, and
    the OpenAI-shaped Responses and Chat Completions formats with their stream
    mappers;
  - what every OAuth device login and token refresh meets (`OAuthSeconds`,
    `PollInterval`, `DecodeJSONResponse`, `JWTClaims`), the flows themselves
    being each vendor's; the credential pool contract
    (`CredentialPool`, `AccountDocument`) with one refresh at a time per
    account (`RefreshGates`), the one refresh protocol of every family
    (`Renew`), a pool held in memory for logins and tests
    (`MemoryCredentialPool`), the account operations every subscription family
    shares (`Accounts`, over a family's `AccountKit`), and why an account could
    not be used (`AuthFailure`); quota (`Quota`), token accounting, and the
    models.dev client (`provider.ModelsDevClient`: the one copy of the document
    a backend keeps, `ModelsDevClient`, and its vendors and models as catalog
    models); the catalog, state, account and quota shapes they return are
    `internal/core`'s, because the web app receives them.
- **Public boundary:** the items above; test support is in
  `internal/provider/providertest`. Behavior:
  [Providers](../providers/providers.md), [Models](../providers/models.md),
  [Usage and quota](../providers/usage-and-quota.md) and [Failures and
  recovery](../agent/failures-and-recovery.md).
- **Must not:** depend on concrete providers, the agent runtime,
  `internal/host` or a Host implementation.

<a id="vendor-provider-crates"></a>

#### `internal/providers/anthropicapi`

- **Owns:** The Anthropic Messages API: request and stream mapping, model
  metadata and failure reading.
- **Public boundary:** its `Provider` implementation and configuration.
  Transports, body builders, stream parsers and authentication stores stay
  private. [Providers](../providers/providers.md) defines endpoint rules.
- **Secret boundary:** keys, tokens, custom headers and raw endpoints stay
  inside the provider and never reach web frames or responses.
- **Must not:** depend on agent packages, `internal/host`, a plugin or a Host
  implementation.

#### `internal/providers/openaiapi`

- **Owns:** The OpenAI Responses API, and the Chat Completions wire for
  OpenAI-compatible endpoints with their reasoning deltas and the opt-in replay
  of thinking as `reasoning_content`; model metadata.
- **Public boundary:** its `Provider` implementation and configuration.
  Transports, body builders, stream parsers and authentication stores stay
  private. [Providers](../providers/providers.md) defines endpoint rules.
- **Secret boundary:** keys, tokens, custom headers and raw endpoints stay
  inside the provider and never reach web frames or responses.
- **Must not:** depend on agent packages, `internal/host`, a plugin or a Host
  implementation.

#### `internal/providers/google`

- **Owns:** The Gemini `generateContent` API, the native wire rather than the
  OpenAI-compatible one: request and stream mapping, including thought
  summaries, thought signatures and tool-returned media as inline parts; model
  metadata.
- **Public boundary:** its `Provider` implementation and configuration.
  Transports, body builders, stream parsers and authentication stores stay
  private. [Providers](../providers/providers.md) defines endpoint rules.
- **Secret boundary:** keys, tokens, custom headers and raw endpoints stay
  inside the provider and never reach web frames or responses.
- **Must not:** depend on agent packages, `internal/host`, a plugin or a Host
  implementation.

#### `internal/providers/codex`

- **Owns:** The Codex Responses transport over server-sent events and
  WebSocket, device login, token refresh, reading its failure records
  (usage-limit reset fields), the quota from its usage probe and the
  `x-codex-*` headers, and the model catalog.
- **Public boundary:** its `Provider` implementation and configuration.
  Transports, body builders, stream parsers and authentication stores stay
  private. [Providers](../providers/providers.md) defines endpoint rules.
- **Secret boundary:** keys, tokens, custom headers and raw endpoints stay
  inside the provider and never reach web frames or responses.
- **Must not:** depend on agent packages, `internal/host`, a plugin or a Host
  implementation.

#### `internal/providers/grokbuild`

- **Owns:** RFC 8628 device login against `auth.x.ai`, OIDC token refresh, the
  Chat Completions transport to the Grok Build proxy, the model catalog from
  `/v1/models`, and the quota from the billing and subscription probe and the
  rate-limit headers.
- **Public boundary:** its `Provider` implementation and configuration.
  Transports, body builders, stream parsers and authentication stores stay
  private. [Providers](../providers/providers.md) defines endpoint rules.
- **Secret boundary:** keys, tokens, custom headers and raw endpoints stay
  inside the provider and never reach web frames or responses.
- **Must not:** depend on agent packages, `internal/host`, a plugin or a Host
  implementation.

<a id="provider-claude-code"></a>

#### `internal/providers/claudecode`

- **Owns:** the Claude Code provider: the stream-json exchange with the CLI
  over the Host process interface, the SDK MCP channel (MCP over an in-memory
  transport) with the model's parallel tool batches preserved and a held
  `tools/call` answered in the next run, the model catalog mapping, the OAuth
  usage quota probe, and the account token passed in the CLI's environment at
  spawn.
- **Public boundary:** its `Provider` implementation and configuration type,
  the session runtime it builds over a placement, and the placement contract
  (`Placement`): `start` starts a new CLI process from the spawn request the
  provider builds and answers the `internal/host` process. `Provider.Runtime`
  refuses with `ProcessHostRequired`. Behavior: [Claude
  Code](../providers/claude-code.md#how-a-runtime-gets-its-process).
- **Secret boundary:** OAuth tokens never reach a frame or response the web app
  sees; the only process that receives one is the CLI on the user's Cloud.
- **Must not:** depend on the agent's packages, a plugin or a Host
  implementation. It runs the CLI through the `internal/host` process the
  placement answers; which machine that is, is the backend's placement.

<a id="agent-store"></a>

#### `internal/agent/store`

- **Owns:** what the agent keeps of a conversation and how it keeps it:
  - the tree store contract (`TreeStore`, `SessionStore`, with the node records
    and checkpoints they carry), which also reads the records of commands'
    outputs for `demi shell output` and gives out the numbers of the
    conversation's sequences that the model sees ([Identifiers the model
    sees](../agent/runtime.md#identifiers-the-model-sees));
  - each node's command state history (`CommandStateHistory`, [Command state
    history](../agent/command-state-history.md));
  - the media rules (`media`): a medium stored once when it enters a
    transcript, the bytes a session holds for its requests and the model's view
    of them, over the `BlobStore` its tree store gives it, and the blobs each
    block references, media and edit copies, which a store indexes for
    [retention](../backend/storage.md#retention); the fitting of an image as it
    enters, with Go image codecs (`images`, [Images in the
    transcript](../agent/runtime.md#images-in-the-transcript)); and the blocks
    an upload becomes with its recorded media type and opening (`attachments`).
- **Public boundary:** the items above; test support is in
  `internal/agent/store/storetest`.
- **Must not:** depend on the transcript, the session or the tools, or know a
  provider's runtime. The product's store decides where media bytes go; this
  package defines only what is stored.

<a id="agent-transcript"></a>

#### `internal/agent/transcript`

- **Owns:** the transcript of one session: its log and the patches it sends
  (`Log`), the identities it gives blocks and turns (`IDs`), its estimates
  (`Estimate`), the points it is cut at for a resume, a rewind, an edit or a
  compaction (`Cut`), its replay into a provider request (`Replay`), and the
  rule that retires a tool result's expired images and videos, which the
  backend applies to stored conversations (`Retire`).
- **Public boundary:** the items above; test support is in
  `internal/agent/transcript/transcripttest`.
- **Must not:** run a turn, call a provider or store anything itself.

<a id="agent-session"></a>

#### `internal/agent/session`

- **Owns:** the runtime of one session: `Session`, a handle over one session's
  `Core`, with its turns, input admission, steers and wakeups, cancellation,
  retry and resume, compaction ([Compaction](../agent/compaction.md)), message
  editing ([Message editing](../agent/message-editing.md)), persistence through
  the tree store, and its events and status; and the tool-call contract a
  session runs its tools through (`ToolInvocation`, `ToolOutcome`,
  `ToolEffect`, `ToolFailure`), which it defines and the tools implement.
- **Public boundary:** the handle, its construction and restoration (`Deps`,
  `Init`), its events, status and errors, the editing types (`EditSubmission`,
  `EditCheck` and their outcomes), the command storage a job's calls reach
  (`Session.Storage`, answering the shell's storage port messages), and the
  tool-call contract; test support is in `internal/agent/session/sessiontest`.
  Behavior: [Agent runtime](../agent/runtime.md).
- **Must not:** know tools, nodes, trees or connections; a session runs a tool
  only through the tool-call contract.

<a id="agent-tools"></a>

#### `internal/agent/tools`

- **Owns:** the standard tools (`StandardTool`: `shell_exec`, `shell_status`,
  `shell_write`, `shell_abort` and `yield`), their input and results and the
  rules for them that open every node's system prompt, with the durable
  dispatch of every tool call over each node's shell environment per Host,
  which the product's `ShellEnvironmentFactory` makes; and what the product
  answers for a node: the Host its shell tools reach now (`HostResolver`, with
  its `Host` type) and the context sources asked before each request
  (`ContextSource`), with the node a question is about (`NodeContext`), and the
  subagent profiles as data (`Profile`).
- **Public boundary:** the items above; test support is in
  `internal/agent/tools/toolstest`.
- **Must not:** create sessions, nodes or trees, or own a shell interpreter.

<a id="agent-server"></a>

#### `internal/agent/server`

- **Owns:** the agent server, which assembles sessions and tools into
  conversations:
  - `Server`, one per user shard, which holds each open conversation's `Tree`;
    `Tree`, a conversation's live nodes, its attachment to a connection and the
    supervisor operations on its subagents; `Node`;
  - `Connection`, the frame handling of one conversation socket: the backend
    hands it each decoded client frame, and its bounded outbox
    (`FrameReceiver`) carries every server frame back;
  - each node's command storage as a job's rpc calls reach it
    (`Server.CommandStorage`), at the history generation the job started in
    ([Mutation API and
    concurrency](../agent/command-state-history.md#mutation-api-and-concurrency));
  - the `demi agent` and `demi shell` command groups;
  - where a session's provider runtimes come from (`ProviderResolver`), the
    notice to the product that a live tree started or stopped working or was
    disposed (`Deps.StatusChanged`), the resolution of the files a frame's
    content refers to, which the backend answers (`ContentResolver`), and the
    conversation title request and its rules (`Title`).
- **Public boundary:** the items above; test support is in
  `internal/agent/server/servertest`. A product supplies, in `Deps`, the
  command set nodes start from, its instructions for the system prompt, the
  profiles, the context sources, the Host resolver, the providers, a shell
  environment per Host and a tree store; the agent never knows which shell
  engine runs or which plugin a command, a text or a context source comes from.
  Behavior: [Agent runtime](../agent/runtime.md) and
  [Subagents](../agent/subagents.md).
- **Rules:** the node assembly is the one place that creates a node's session;
  the supervisor asks it for a child and never builds one. A session stores and
  reads media only through its tree store: the product's store decides where
  media bytes go, and `Server` never sees a blob store.
- **Must not:** depend on concrete providers, Host implementations or user
  interfaces; own a shell interpreter; own a socket. The backend owns the
  conversation socket and hands the agent decoded frames. Real-model compaction
  acceptance is run by hand outside this package; automated tests use scripted
  runtimes from the provider contract.

<a id="runner-demi-runner"></a>

#### `internal/runner`

- **Owns:** the execution host's program: registration and the backend
  connection, the Host log, installation, and the composition of the runner
  packages: it gives the jobs the shell, the services and the connection
  handle, and the dispatcher the names the shell reserves with the runner's
  own; its local endpoint answers the management requests (`status`, `drain`)
  beside the dispatcher's commands. It keeps a service resident while it holds
  conversation state and forwards the conversation release, through
  `internal/runner/cmdpkgs`.
- **Public boundary:** the program entry point wired by `cmd/demi-runner`.
  Acceptance tests drive the built program as a backend does; resource-limit
  tests run in isolated subprocesses. Behavior:
  [Runner](../execution/runner.md) and [Native command
  execution](../execution/native-runtime.md).
- **Must not:** own conversations or provider implementations; administer Cloud
  mounts, networking or volumes, which belong to the machine manager; boot as
  PID 1, since init belongs to the image; link Demi's command algorithms.

<a id="runner-process"></a>

<a id="runner-libraries"></a>

#### `internal/runner/process`

- **Owns:** the runner's child processes and their IO: starting a process in a
  process group of its own with the runner's attributes (`ChildAttributes`: the
  umask and resource limits of [Builtins that act on a
  process](../execution/runner.md#builtins-that-act-on-a-process)), killing and
  reaping it; standard IO plumbing; the pipe endpoints that carry file contents
  and output to the backend's pipe routes, and the report of a pipe's outcome
  ([Pipes and output](../execution/runner.md#pipes-and-output)); the split of a
  stream into lines and the kept tail of a stream; private state files written
  atomically, through `internal/artifacts`'s publication; the line counts of a
  change to a file, computed with a Myers diff; the local command client that a
  command alias runs ([External command
  clients](../execution/commands.md#external-command-clients)); and the job
  shell contract (`JobShell`, `ShellJob`): how the runner starts a job's
  script, feeds its input, signals, cancels and awaits it, without knowing
  which shell runs it. A job's declared commands come with it as their root
  names, its execution context's id and the `internal/cmdsdk` `Handler` each
  invocation goes to (`JobCommands`).
- **Public boundary:** the items above.
- **Must not:** know the backend connection, jobs, commands, services or the
  shell that implements the contract.

<a id="runner-host"></a>

#### `internal/runner/host`

- **Owns:** the Host operations the backend asks for ([Host
  operations](../execution/runner.md#host-operations)): filesystem operations
  and file contents through pipes, the working tree with its status, diffs and
  change watch (go-git objects, index and transport, a parallel walk over a
  watched baseline, fsnotify on Linux and Windows, and purego FSEvents on
  macOS), network streams, and the volumes a Host reports. It resolves a
  request's paths and waits out a lack of open files with `internal/cmdsdk`, as
  every native program does, and writes a file's contents through
  `internal/artifacts`'s staged publication.
- **Public boundary:** one function per operation over its wire request, and
  the working tree's watch.
- **Must not:** know jobs, commands or the connection that carries the
  requests.

<a id="runner-jobs"></a>

#### `internal/runner/jobs`

- **Owns:** shell jobs and the commands they run: the job table, each job's
  execution context with its command context, its directory and kept output,
  the edit report at its end, and the command dispatcher (it parses argv with
  `internal/declare`, holds a `--json` command's output until it is checked
  against the leaf's output schema, routes native invocations to their services
  and rpc calls to the backend) with local command forwarding.
- **Conversation scope:** keeps each job's command context and writes it into
  every native invocation; it implements no conversation browser operation.
- **Public boundary:** the job table, the dispatcher, which implements the
  `cmdsdk.Handler` the shell calls, and the connection handle it sends rpc
  calls and reports through (`ConnectionHandle`), whose requests the
  composition's connection owner serves and whose answers it routes (`Relay`).
- **Must not:** know the shell that runs a job's script, beyond the job shell
  contract, or own the backend connection.

<a id="runner-shell"></a>

#### `internal/runner/shell`

- **Owns:** the patched `mvdan.cc/sh` interpreter and system utility execution
  ([Shell jobs](../execution/runner.md#shell-jobs)): the shell options, the
  builtins that act on a process, the declared commands' builtins, which hand
  each invocation to the handler the job supplies, a failed utility contained
  to its job, and the job shell contract's implementation, whose signals and
  output streams are `internal/runnerwire`'s.
- **Public boundary:** the `JobShell` implementation and the names the shell
  reserves (`BuiltinNames`), which the composition gives the command
  dispatcher. The interpreter work lives in
  `internal/runner/shell/internal/engine`, which only the shell and its
  `shelltest` import, so the test support runs the real implementation.
- **Must not:** know jobs, their execution contexts, services or the
  connection: a declared command reaches the dispatcher only through the
  `cmdsdk.Handler` the job gives it.
- **Deferred utility behavior:** edit tracking of writes by `sed -i`, `tee` and
  `sort -o`; paired Macs' BSD utilities; Windows without Unix utilities; the
  Cloud image's GNU utilities, grep, ripgrep and jq; and the model's shell,
  platform and utility report remain open follow-up work under [owner decisions
  3 and 4](../delivery/go-migration.md#owner-decisions). Redirections remain
  tracked by the interpreter's open handler.

 <a id="runner-command-packages"></a>

#### `internal/runner/cmdpkgs`

- **Owns:** the artifact cache with its lines, the preinstalled artifacts of a
  Cloud image, the installs in progress, and the resident service registry:
  installing a pinned executable, starting, checking and reusing a service, its
  leases, conversation status and release, its numbers stream through a source
  the composition supplies (`NumberSource`), its artifacts stream, whose
  installs resolve through the invocation's work, and its retirement ([Native
  command execution](../execution/native-runtime.md)).
- **Public boundary:** the registry and its handles (`ServiceHandle`), the
  cache, the installs a connection reports (`InstallsReceiver`), and
  `NumberSource`. Test support is in `internal/runner/cmdpkgs/cmdpkgstest`.
- **Must not:** implement a command, parse argv or know the connection.

<a id="machine-manager-demi-machine-manager"></a>

#### `internal/machines`

- **Owns:** Linux Cloud machine-manager configuration, preflight, admission,
  lifecycle coordination, recovery and Unix socket server; the host install
  script, Lima configuration and pinned `runsc` build inputs implement [Cloud
  setup](../cloud/setup.md) and [Mac
  development](../guides/mac-development.md), not a second lifecycle.
- **Public boundary:** configuration and manager entry point wired by
  `cmd/demi-machine-manager`, using `machinewire` requests and responses.
- **Must not:** listen on TCP; know users, conversations or the control
  database; link the backend; execute image content on the host; offer
  alternative runtime modes. Workload credentials use `runnerwire.ManagedBoot`.
- **Process boundary:** `runsc`, `mke2fs`, `e2fsck`, `resize2fs` and `bsdtar`
  are infrastructure tools; networking uses netlink and nftables APIs. Other
  operations use syscalls, ioctls or files. No user command becomes a host
  administration command. Docker and containerd are not dependencies.

#### `internal/machines/sandbox`

- **Owns:** Linux gVisor sandboxes through `runsc`, OCI bundles, cgroups,
  private mount namespaces and transient boot files. A namespace job owns its
  locked OS thread, unshares `CLONE_FS`, finishes and exits without unlocking;
  recovery starts `/proc/self/exe` from the entered thread.
- **Public boundary:** sandbox start, stop, status and recovery over explicit
  boot, disk and network inputs from the manager.
- **Must not:** know users, conversations, the backend or control database;
  execute image content on the host; choose another runtime; own the image
  store, firewall policy or machine lifecycle admission.

#### `internal/machines/storage`

- **Owns:** the machine-image store: paired generations, working recovery,
  pinned bases, publication and collection; ext4 creation, checking and
  growth, and sparse copies, over `internal/machines/system`'s mounts, loop
  devices and freezes.
- **Public boundary:** image import, preparation, checkpoint, recovery, growth
  and collection operations over explicit volume handles.
- **Must not:** execute image content on the host, know users, conversations or
  the control database, link the backend, own sandbox lifecycle or network
  policy, or choose workload credentials.

#### `internal/machines/network`

- **Owns:** Linux network namespaces, address slots, veth links and the
  firewall table, through netlink and nftables with explicit namespace FDs.
- **Public boundary:** network allocation, setup, recovery and release.
- **Must not:** execute user commands as administration commands, listen on
  TCP, know users, conversations or the control database, link the backend, own
  machine images or choose a runtime or workload credentials.

#### `internal/machines/system`

- **Owns:** the Linux interfaces the manager uses instead of administration
  tools (`managed-hosts.md` § Linux control): mounts, loop devices, filesystem
  freezes and namespace threads; running the infrastructure tools
  (`runsc`, `mke2fs`, `e2fsck`, `resize2fs`, `bsdtar`) with the kept tail of
  their output; the sandbox's user ID that files given to a sandbox carry; and
  the fault points the manager's tests inject failures at.
- **Public boundary:** the items above, for `sandbox`, `storage`, `network`
  and `internal/machines`.
- **Must not:** decide policy, know images, sandboxes, networks or requests,
  or execute image content on the host.

<a id="backend-demi-backend"></a>

#### `internal/backend`

- **Owns:** the hosted product's server program: its typed configuration,
  validated at startup; the instance secret and the keys derived from it; the
  built-in provider families, one per vendor package (`BuiltinFamilies`), which
  the composition starts with; the plugins, one per plugin package, in their
  order of registration (`BuiltinPlugins`); and the composition of the backend
  packages in `backend.Start`, the one composition root: the storage, the
  shared services, shard routing and the edge, with the command manifest it
  serves to runners, the rpc handlers it runs. Its modules and their packages
  are listed in
  [Backend](../backend/backend.md#request-paths-and-responsibilities).
- **Public boundary:** the entry point wired by `cmd/demi-backend`; the
  built-in families, beside which a test registers scripted ones;
  `backend.Start` and `Config` for tests, with the parts a test replaces: the
  provider families entries are assembled with (`FamilyRegistry`,
  `ProviderFamily` and the arguments a family builds a provider from, or, for a
  provider that needs a process, its session runtime over a placement), the
  login timing (`LoginTiming`), the conversations' bounds
  (`ConversationTuning`), the times of a page's sockets (`PageTuning`), the
  Cloud's and the idle clock's times and limits and the retention pass's
  schedule (`CloudTuning`, `LifecycleTuning`), the command packages their
  commands bind to (`NativeCatalog`, which the executable and the scenarios
  make with `PublishNative` from a `DEMI_NATIVE_CONFIG` file) and the plugins.
  A test imports each of these from the library that owns it
  (`internal/backend/providers`, `internal/backend/usershard`,
  `internal/backend/cloud`, `internal/backend/runners`), never through the
  executable. Test support is in `internal/backend/backendtest`. For suites
  that start the executable, the example program `ScriptedMachines` runs the
  scripted machine manager of its scenarios ([Web app contract
  suite](../delivery/scenarios.md#web-app-contract-suite)).
- **Must not:** be linked by another runtime package; put business logic in the
  HTTP layer beyond routing and validation; return secrets or proxy model
  traffic; spawn `runsc` or image tools itself (every sandbox and disk
  operation goes to the machine manager). The one credential that reaches a
  runner is a Claude Code account's token, in the CLI's environment on the
  user's Cloud.

<a id="backend-accounts"></a>

<a id="backend-libraries"></a>

#### `internal/backend/accounts`

- **Owns:** accounts, password hashing, web sessions, login lockout and
  email-change delivery, and each user's preferences ([Authentication and
  ownership](../backend/backend.md#authentication-and-ownership), [User
  preferences](../product/web-api.md#user-preferences)).
- **Public boundary:** the account, session and preference services. The key
  that email codes are derived from is given to it; it does not derive keys.
- **Must not:** know conversations, devices or providers.

<a id="backend-blobs"></a>

#### `internal/backend/blobs`

- **Owns:** the object store, on local disk or S3 through its storage adapter,
  and the attachment and transcript media over it with the record of each
  blob's uses ([The object store](../backend/storage.md#the-object-store)).
- **Public boundary:** the store and the blob namespaces. Test support is in
  `internal/backend/blobs/blobstest`.
- **Must not:** hold records other than the blobs'.

<a id="backend-cloud"></a>

#### `internal/backend/cloud`

- **Owns:** each user's Cloud: policy and capacity across users, the machine
  transitions, wake, hibernate, checkpoint, reset, growth, recovery and
  maintenance, the machine manager's client, and machine access ([Managed
  hosts](../cloud/managed-hosts.md)), with its times and limits
  (`CloudTuning`); and `CloudShard`, what the Cloud needs of its user's shard:
  the handles its operations use, holding the user's conversations for an idle
  stop or a reset (`ConversationHold`), their activity and whether someone
  attends them, and the notice that a Cloud stopped.
- **Public boundary:** the Cloud component, its operations on `CloudShard`, and
  the manager client.
- **Must not:** see `Shard`, conversations' state or exposes.

<a id="backend-database"></a>

#### `internal/backend/database`

- **Owns:** the SQLite databases ([Storage](../backend/storage.md)): the
  control service and every control record, the conversation index, each
  conversation's database with the tree store over it, its `blob_refs` index
  and its records of commands' outputs, the schemas and their migrations, and
  the encodings of stored values; and the record types it stores, among them
  the hashes and policies of sessions and challenges, a conversation's target
  and settings changes, and catalog records, which the domains above use. The
  tree store reaches the owner's blobs through `OwnerBlobs`, the narrow
  interface of what its commits need of the namespace and the record of blob
  uses that `internal/backend/blobs` keeps, which the shard implements.
- **Public boundary:** the stores and their records. Test support is in
  `internal/backend/database/databasetest`.
- **Must not:** hold a domain's policy or call another backend package.

<a id="backend-expose"></a>

#### `internal/backend/expose`

- **Owns:** the expose mechanism ([Host expose](../execution/expose.md)):
  expose records and their lifetime, and the live relay connections with their
  admission up to the device; and `ExposeShard`, what an expose needs of its
  user's shard: whether a device takes a new expose, its runner connected and,
  for a Cloud, the Cloud running.
- **Public boundary:** the exposes component and its operations on
  `ExposeShard`, which the plugin host's exposes operation reaches through the
  shard. The `demi expose` commands, their numbers and the product surface are
  `internal/plugins/expose`'s; the network stream of an admitted connection is
  opened by `internal/backend/usershard` through device access.
- **Must not:** see `Shard` or reach a Host.

<a id="backend-host-access"></a>

#### `internal/backend/hostaccess`

- **Owns:** the conversation's host access (`WithHost`, the one way to a
  conversation's Host; [Host
  operations](../execution/sessions-and-targets.md#host-operations)), with each
  conversation's slot and its file gate, target resolution and the transitions
  that end a target (switch, archive, detach); file transfers, uploads, remote
  files and user streams, with the leases the edge holds of them; the shell
  environments of agent nodes over it, which keep the commands' outputs and
  edit copies as the user's blobs (`ConversationBlobs`); the product's `demi
  host` group; the installation of the user's [Host
  directories](plugins.md#host-directories) before a job runs, once per runner
  connection, the reads of a conversation's files and the package calls plugins
  make; and `HostShard`, what host access needs of its user's shard: the
  handles its operations use (the control service, the user's devices, pipes,
  command router, blobs and conversation databases, the native catalog and the
  public address, the user's Host directories, and the shard as the Cloud sees
  it) and the conversation's idle watch, which every Host admission starts.
  Uploads read the user's blobs, and the shells store theirs, in
  `internal/backend/blobs`, which is why it depends on it.
- **Public boundary:** host access and its operations on `HostShard`, the
  conversations' slots, the transitions, the leases, the shell environment
  factory and the `demi host` group. A slot's file gate gives its leases, the
  ones a conversation's Host is made against, only to host access; other work
  holds the conversation through the gate itself (`FileGate`), whose leases
  make no Host.
- **Must not:** see `Shard`; or leave a second way to a conversation's Host.

<a id="backend-http"></a>

#### `internal/backend/edge`

- **Owns:** the HTTP edge: the listener and router, the session gate, request
  extractors and body limits, the mapping of errors to `ErrorCode`, the
  installer, native artifact and web app asset routes, the plugins' page call
  routes, runner acceptance, and the byte copies of file transfers, pipes, user
  streams and the expose relay ([Web API](../product/web-api.md)).
- **Public boundary:** the edge the executable starts (`Edge`), with the state
  its routes reach (`AppState`, `Site`). Test support is in
  `internal/backend/edge/edgetest`.
- **Must not:** hold business logic beyond routing and validation.

<a id="backend-idle-watch"></a>

#### `internal/backend/idlewatch`

- **Owns:** the idle rule's mechanism: one idle window, and one watch per
  resource over that resource's gates, which reserves the resource and reads it
  again before it retires it ([Conversation idle and Host resource
  release](../execution/resource-lifecycle.md)).
- **Public boundary:** the watch, its policy and the activity it reads.
- **Must not:** know what the resource is; its callers retire it.

<a id="backend-page-sync"></a>

#### `internal/backend/pagesync`

- **Owns:** the change registry of the pages' synchronization channels: the
  parts of a user's product state (`Part`), and the marks each change leaves on
  that user's open channels (`SyncRegistry`, `UserMarks`) ([Page
  synchronization](../backend/backend.md#page-synchronization)). A channel is
  registered under the hash of the session it opened with, a record of
  `internal/backend/database`, so a sign-out ends that session's channels.
- **Public boundary:** the items above.
- **Must not:** read or build the product state; `internal/backend/usershard`
  does.

<a id="backend-plugins"></a>

#### `internal/backend/plugins`

- **Owns:** the plugin host ([The plugin host](plugins.md#the-plugin-host)):
  the registry of plugin factories with the checks of their manifests; the
  command set every node starts from, with its `demi` root, the plugins' groups
  and roots, and the groups left out for a package the startup catalog does not
  serve; the plugins' profiles and context sources in registration order; each
  user's instances, with the `rpc` handlers that forward a command to its
  plugin; the port's operations over the user's plugin values, blobs, Host
  directory sets, Host file reads, package calls, conversation hosts, exposes
  and page-state marks; the plugins' user stream declarations; the marks of a
  plugin's page state when a product change it follows happens; and page state
  and page calls, with the validation of a call's parameters against its
  method's schema.
- **Public boundary:** the registry, the user's plugins as the shard holds
  them, and the operations on `PluginShard`, what the host needs of its user's
  shard: the control service, the user's blobs, the user's change marks, the
  reads of a conversation's files on its running main Host and the package
  calls, which the shard makes through host access, the conversation's Hosts,
  and the user's exposes.
- **Must not:** see `Shard`, reach a Host, know an agent's session, or hold the
  logic of one plugin.

<a id="backend-providers"></a>

#### `internal/backend/providers`

- **Owns:** provider assembly and model catalogs with the catalog cache; the
  credential vault: its records, their encryption and scope, subscription
  accounts, login flows and quotas; usage metering and the request rate limit;
  and the family contract every provider family implements (`ProviderFamily`,
  `FamilyRegistry`) ([Providers](../providers/providers.md), [Usage and
  quota](../providers/usage-and-quota.md)).
- **Public boundary:** the assembly, the vault, the meter, the family contract.
- **Must not:** know conversations, devices or the Cloud; run a process for a
  provider, which the shard places on the user's Cloud.

<a id="backend-remote-host"></a>

#### `internal/backend/remotehost`

- **Owns:** the backend's end of a runner:
  - the connection engine (`Link`, served by its `LinkDriver`): routing replies
    by id, liveness and rpc plumbing, with the product's decisions on calls
    behind `LinkPolicy`;
  - `Host`, a `host.Host` over a runner connection whose file contents travel
    through pipes, with job, working-tree, network, log and service facets;
  - pipe records (`Pipes`) and their `Send` ends;
  - `ShellEnvironment`, the production `host.ShellEnvironment` over real runner
    jobs, and its factory: at a job's end it reads what the backend does not
    hold of the command's output and its edit copies, hands them to the
    product's keeper, and releases the job's directory;
  - a whole output as the runner wire's kept-output records, in which the
    backend stores it (`EncodeOutput`, `DecodeOutput`);
  - building manifests from a command set.
- **Public boundary:** the items above; test support is in
  `internal/backend/remotehost/remotehosttest`. Behavior:
  [Runner](../execution/runner.md) and [Native command
  execution](../execution/native-runtime.md), which owns [artifact-location
  admission](../execution/native-runtime.md#install-artifacts).
- **Must not:** own sockets or HTTP routes (the backend's connection tasks and
  pipe routes feed it), claim policy, the device registry, credentials or
  conversation state.

<a id="backend-runners"></a>

#### `internal/backend/runners`

- **Owns:** runners and their devices: pairing with its pending claims and
  codes, the installer scripts, each device's runner connection (`Devices`) and
  the Host handles made over it, device access among them, the lease of a
  conversation's file gate (`FileGate`, `FileLease`), the rpc relay's routing
  of a job's calls, the command context a job carries, the file listings and
  text the product reads from a Host, native artifact publication and the
  development store, and the backend's public address that runners reach
  (`PublicURL`) ([Runner](../execution/runner.md), [Native
  runtime](../execution/native-runtime.md#backend-deployment-configuration)).
  Publication uploads to the object store `internal/backend/blobs` configures,
  and the file gate is an `internal/gates` gate, which is why it depends on
  both.
- **Public boundary:** the items above. A Host handle owned by a conversation
  is given out only against that conversation's file-gate lease:
  `Devices.ConversationHost` takes a `FileLease`, which names the conversation
  of its gate, and keys the handle by it. The gates live in the conversations'
  slots of host access, and only host access takes their leases. Device access
  and machine access make handles that touch no conversation's files. Test
  support is in `internal/backend/runners/runnerstest`.
- **Must not:** reach a conversation's Host except through host access, or know
  conversations, the Cloud or exposes.

<a id="backend-user-shard"></a>

#### `internal/backend/usershard`

- **Owns:** the user shard ([The user shard](concurrency.md#the-user-shard)):
  `Shard`, `Shards` and shard routing, with one mutex per shard and short
  critical sections, the shared services a shard is given (`Services`) over the
  storage they open, the times and bounds the shard and the edge run with,
  calls into a shard with the routing of the machine manager's death events,
  socket adoption and the page socket; conversations as the agent sees them:
  agent-tree hosting with the agent server's dependencies composed from the
  plugin host, the product's instructions and the execution context source, the
  conversation socket, history and fork, summaries and titles, the providers a
  session resolves and its failure facts; the conversations' idle watches,
  release and the daily retention pass; the pages' product state and
  synchronization channels; the Claude Code CLI's work on the user's Cloud;
  runner adoption and the runner link's policy; the network stream of a relayed
  expose connection, opened through device access; and the implementations of
  `CloudShard`, `ExposeShard`, `HostShard` and `PluginShard` for `Shard`.
- **Public boundary:** shard routing, the shared services, the times and
  bounds, and the calls the edge makes into a shard. Test support is in
  `internal/backend/usershard/usershardtest`.
- **Must not:** serve HTTP.

<a id="xtask"></a>

#### `tools/release`

- **Owns:** native builds and release packaging for every executable, the
  pinned Chrome for Testing record, Cloud image packaging, fork patch
  comparison, and the development backend with its echo model and development
  store.
- **Public boundary:** release, image, development and fork-review commands.
- **Must not:** hold another implementation of downloads, publication or
  contract records; artifacts owns publication and contract packages own record
  types.

#### `tools/contractgen`

- **Owns:** Go-type discovery and generated decoders, encoders, validation and
  Zod, including plugin page types and the web and gallery plugin registries
  from declared page metadata.
- **Public boundary:** generation invoked by `go generate` and `bun run
  contracts`.
- **Must not:** hold a second contract declaration or a hand-maintained root
  list; roots are markers on Go types.

#### `tools/contractgen/manifests`

- **Owns:** printing the manifests of the backend's built-in plugins that name
  a page, in registration order, for `tools/contractgen -ts`.
- **Public boundary:** a command that writes them to standard output.
- **Must not:** choose or order plugins itself; `BuiltinPlugins` does.

#### `tools/contractgen/pagemeta`

- **Owns:** the tool-internal page metadata shared by the manifest printer and
  generator, including schemas, directions and stream constants.
- **Public boundary:** plain Go types and strict metadata decoding.
- **Must not:** import generated contracts or contain contract markers.

#### `tools/archcheck`

- **Owns:** the import-direction check over this document and `go list -deps
  -json`, including test imports.
- **Public boundary:** the architecture-check command.
- **Must not:** maintain another dependency table or implement runtime
  behavior.

#### `tools/cgocheck`

- **Owns:** the no-cgo check of every shipped program on each target.
- **Public boundary:** the cgo-check command.
- **Must not:** allow cgo in shipped programs or implement runtime behavior.

<a id="executables"></a>

#### `cmd/demi-backend`

- **Owns:** `main.go` wiring for `demi-backend`.
- **Public boundary:** the executable, assembled through `internal/backend`.
- **Must not:** contain business logic or be imported by another package.

#### `cmd/demi-runner`

- **Owns:** `main.go` wiring for `demi-runner`.
- **Public boundary:** the executable, assembled through `internal/runner`.
- **Must not:** contain business logic or be imported by another package.

#### `cmd/demi-file`

- **Owns:** `main.go` wiring for `demi-file`.
- **Public boundary:** the executable, assembled through
  `internal/cmdpkg/file`.
- **Must not:** contain business logic or be imported by another package.

#### `cmd/demi-browser`

- **Owns:** `main.go` wiring for `demi-browser`.
- **Public boundary:** the executable, assembled through
  `internal/cmdpkg/browser`.
- **Must not:** contain business logic or be imported by another package.

#### `cmd/demi-claude-code`

- **Owns:** `main.go` wiring for `demi-claude-code`.
- **Public boundary:** the executable, assembled through
  `internal/cmdpkg/claudecode`.
- **Must not:** contain business logic or be imported by another package.

#### `cmd/demi-machine-manager`

- **Owns:** `main.go` wiring for `demi-machine-manager`.
- **Public boundary:** the executable, assembled through `internal/machines`.
- **Must not:** contain business logic or be imported by another package.

#### `cmd/demi-native-fixture`

- **Owns:** `main.go` wiring for `demi-native-fixture`.
- **Public boundary:** the test executable, whose handlers come from
  `internal/runner/cmdpkgs/cmdpkgstest` and whose service uses `internal/cmdsdk`.
- **Must not:** contain business logic or be imported by another package.

#### `internal/programtest`

- **Owns:** the repository's programs for tests: it builds each program a
  test asks for once per test binary, with `go build` into a temporary
  directory it removes when the test binary ends, or takes it from the
  directory `DEMI_TEST_PROGRAMS` names.
- **Public boundary:** a function that returns a program's path for a test.
- **Must not:** be imported by production code, or rebuild a program that
  `DEMI_TEST_PROGRAMS` supplies.

#### `internal/commandwire/commandwiretest`

- **Owns:** the operations of the runner's native fixture service
  (`FixtureOperations`), which the tests at both ends of the wire use.
  Finding built programs is `internal/programtest`'s.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/cmdsdk/cmdsdktest`

- **Owns:** a service process driven with a client
  (`ServiceProcess`), gives a handler numbers from counters that start at 1
  (`CountingNumbers`) or answers a service's numbers stream from them
  (`AnswerNumbers`), answers a service's artifacts stream or gives a handler
  artifacts from a function (`AnswerArtifacts`, `ArtifactsFrom`), and counts
  the process's pauses before trying an operation again (`Pauses`), which show
  an operation waiting out a lack of open files.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/cmdpkg/claudecode/claudecodetest`

- **Owns:** the test-support boundary reserved by the command package's testing
  feature; it currently supplies no exported fixtures.
- **Public boundary:** no exports until the package has shared test fixtures.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/gates/gatestest`

- **Owns:** `gatestest.Waiting`, how many entrants wait behind a reservation,
  so a test waits for an operation to be held instead of for time.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/artifacts/artifactstest`

- **Owns:** a fixture HTTP server on `127.0.0.1` that downloads can reach, the
  count of waits for install locks, and `InstallUnpacked`, which installs a
  release a test was given unpacked, such as the Chrome suite's
  `DEMI_TEST_CHROME`, as archive installation installs a download.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/host/hosttest`

- **Owns:** the Host conformance cases, an in-memory port for rpc handler tests
  (`MemoryPort`) and sequences that count from 1 (`CountingNumbers`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/plugin/plugintest`

- **Owns:** the loopback transport that encodes every request, reply and port
  message to JSON and decodes it again, which every plugin's tests run through,
  and an in-memory port.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/plugins/skills/skillstest`

- **Owns:** fixture Git repositories, commits and a factory fetching them, a
  fixed fetch clock and skill documents.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/provider/providertest`

- **Owns:** scripted runtimes (`ScriptedRuntime`), a scripted vendor server
  (`MockVendor`), waits for a run's events that fail a test instead of hanging
  it (`NextEvent`, `AllEvents`), the check of an API-key entry's built-in
  catalog (`AssertBuiltInCatalog`), a fixed clock (`FixedClock`) and a wall
  clock that moves with `testing/synctest` time (`SynctestClock`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/providers/codex/codextest`

- **Owns:** a scripted backend WebSocket (`FakeWebSocket`) recording each
  connection's received messages.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/agent/store/storetest`

- **Owns:** an in-memory tree store (`MemoryTreeStore`, whose calls a test can
  hold with `StoreGate`) with an in-memory blob namespace (`MemoryBlobs`), the
  tree store contract's cases that every realization passes (`StoreContract`),
  and the model selections, texts and images the agent packages' tests build on
  (`TestModel`, `ModelOf`, `ModelReading`, `Text`, `SentText`, `PNG`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/agent/transcript/transcripttest`

- **Owns:** predictable identities (`SequentialIDs`) and the texts the model
  receives for a resume, a fired wakeup and an agent message (`ResumeText`,
  `WakeupText`, `AgentMessageEnvelope`), which a session's tests compare its
  requests with.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/agent/session/sessiontest`

- **Owns:** the compaction request's instruction
  (`CompactionSummaryInstruction`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/agent/tools/toolstest`

- **Owns:** a Host type for agents without shell tools (`NoHost`, `NoShells`)
  and the readers of a shell tool's result text (`Field`, `ShownOutput`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/agent/server/servertest`

- **Owns:** provider runtimes that play scripts (`ScriptedProviders`), uploads
  a frame's files resolve to (`TestFiles`) and a test client that drives a
  connection (`TestClient`, and `WaitingFrames` for what an outbox holds).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/runner/jobs/jobstest`

- **Owns:** a dispatcher and local endpoint with live execution contexts and
  channel-backed connection ownership (`Dispatch`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/runner/shell/shelltest`

- **Owns:** shell execution and owned job scopes, with event-based observation
  of units waiting; embedded utility fixtures have no successor because
  utilities are external.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/runner/cmdpkgs/cmdpkgstest`

- **Owns:** the service handlers of `demi-native-fixture`, installed and
  started by runner and backend tests, and a number source that refuses
  (`NoNumbers`). The fixture command wires these handlers with `cmdsdk`.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/machines/machinestest`

- **Owns:** fixture Cloud image archives and manifests (`CloudImage`, `Entry`),
  including invalid-manifest cases.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/backend/backendtest`

- **Owns:** `backendtest.HoldCommits`, which holds the commits of the
  conversations' checkpoints (`CommitHold`) for the scenarios that stop a save
  at its commit ([Message
  editing](../agent/message-editing.md#durability-and-failure-boundaries)),
  `backendtest.FileGate`, a conversation's file gate, whose lease is the
  conversation's work to the idle rules and whose waiting entrants show an
  operation a transition holds, `backendtest.RunRetention`, which runs one user's
  retention pass at once and answers when it has ended, for the scenarios of
  [Retention](../backend/storage.md#acceptance), and two holds of a flow at one
  of its steps until the test releases it (`StepHold`): `backendtest.HoldHellos`
  holds runners' hellos (`HelloStep`: the token's lookup, or the shard's bind)
  for the scenarios that race a hello against its runner going away and against
  shutdown ([Runner](../execution/runner.md#connection-and-identity)), and
  `backendtest.HoldSync` holds the pages' synchronization channels (`SyncStep`:
  once a channel has read its snapshot, before it sends it; or once a change
  woke it, before it takes and reads the parts that changed) for the scenarios
  that change the state while a channel waits ([Page
  synchronization](../backend/backend.md#page-synchronization)).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/backend/blobs/blobstest`

- **Owns:** an S3 fake and a store that counts its operations (`ObjectCounts`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/backend/database/databasetest`

- **Owns:** a control database with a test key and raw-statement access
  behind the service's back, for tests of what the service must refuse, and
  holds the commits of the conversations' checkpoints (`CommitHold`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/backend/edge/edgetest`

- **Owns:** a hold of runner hellos at token lookup, used by
  `backendtest.HoldHellos`.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/machines/storage/storagetest`

- **Owns:** the Cloud image fixtures storage's tests and the manager's tests
  build images from (`CloudImage`).
- **Public boundary:** these fixtures, for the tests of `storage` and
  `internal/machines`.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/machines/system/systemtest`

- **Owns:** running a test job as root in private mount and network
  namespaces with a fresh `/run` (`Isolate`), and tool sets for tests that
  only build command lines (`Placeholder`) or run the installed filesystem
  and firewall programs (`OnPath`).
- **Public boundary:** these fixtures, for the tests of `system`, `sandbox`,
  `storage`, `network` and `internal/machines`.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/backend/remotehost/remotehosttest`

- **Owns:** a real runner process for a backend at any address, with a home and
  state of its own (`RunnerProcess`), such a runner connected to a backend end
  of the fixture's own for one device (`RunnerFixture`), an in-process fake
  runner (`TestDevice`, whose connections are `TestLink`s), a command package
  the workspace built (`NativeFixture`, such as the runner's native fixture
  package) and a policy that runs every call in one command set
  (`CommandPolicy`).
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/backend/runners/runnerstest`

- **Owns:** a bound connection's driver without a socket, for tests that
  play a runner.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/backend/usershard/usershardtest`

- **Owns:** shared services for tests (`usershardtest.StartServices`), on
  which a test starts shard routing, and holds of a flow at one of its
  steps (`StepHold`) returned by `backendtest.HoldHellos` and
  `backendtest.HoldSync`.
- **Public boundary:** these fixtures for black-box tests of the owner and its
  consumers. Every worker and resource has test cleanup.
- **Must not:** implement product behavior or import a consumer of its owner.

#### `internal/cmdpkg/browser/chrome/cdp/cdptest`

- **Owns:** evaluation in targets no command addresses.
- **Public boundary:** Chrome fixtures for browser acceptance tests.
- **Must not:** own conversation state or implement another browser driver.

#### `internal/cmdpkg/browser/chrome/tabs/tabstest`

- **Owns:** the capture extension ID, tab numbers without a numbers source and
  Chrome's own view of its targets.
- **Public boundary:** Chrome fixtures for browser acceptance tests.
- **Must not:** own conversation state or implement another browser driver.

#### `internal/cmdpkg/browser/chrome/page/pagetest`

- **Owns:** page interaction by CSS selector.
- **Public boundary:** Chrome fixtures for browser acceptance tests.
- **Must not:** own conversation state or implement another browser driver.

### Third-party shell fork

`third_party/mvdan-sh` is the patched `mvdan.cc/sh` module, selected by a root
`go.mod` replacement and maintained by `internal/runner/shell`. Keep its
upstream metadata and licenses, and its patch as a file in that directory so an
upgrade can reapply and review every change. The patch provides nested task
joining, process-substitution cancellation and writable-redirection
interception; Demi adapters stay in the runner shell. The fork is an external
module, not a first-party graph entry.

`go run ./tools/release fork diff` shows how the fork differs from the upstream
release it was taken from, so an upgrade or a review sees every local change,
not only the ones `demi.patch` describes. The release is the version the root
`go.mod` requires before its replacement (`mvdan.cc/sh/v3 v3.14.1`). The
command downloads it from the Go module proxy, checks it against the hash the
Go checksum database records, and compares it with the fork file by file:
it lists each changed, added and removed file with its added and removed line
counts, and the upstream files the fork leaves out. `--patch` prints the unified diff instead,
which is what `demi.patch` holds: the command fails when the file differs from
it, so the patch never drifts from the fork.

The other Rust vendored crates have no Go successor. System utilities replace
embedded uutils, findutils, diffutils, sed, grep, ripgrep and jaq; cdproto with
Demi's transport replaces the vendored Chromium client. Follow [owner decisions
3 and 4](../delivery/go-migration.md#owner-decisions), including the deferred
utility behavior named there.

## TypeScript packages

The web app and its libraries are TypeScript and Vue, as workspace packages
under `packages/`.

#### `@demicodes/protocol`

- **Published** to npm.
- **Owns:** the TypeScript form of the Go contracts the web app reads: Zod
  schemas and `z.infer` types for agent frames, transcript blocks and patches,
  content, tool views, models and failure facts, and
  the file-type table with its lookups. `tools/contractgen` generates all of it
  into `src/generated/`, which the package's entry re-exports; nothing else is
  written, and nothing generated is committed.
- **Public boundary:** the generated schemas, types and lookups.
- **Must not:** declare a schema by hand or import another workspace package.

#### `@demicodes/conversation-client`

- **Published** to npm.
- **Owns:** `ConversationClient` and its waiters, the conversation WebSocket
  transport, client-side session events, and `applyTranscriptPatches`, the one
  transcript patch applier. It validates every frame it receives with the
  generated schemas.
- **Public boundary:** the client, the transport and the patch applier.
- **Must not:** import user-interface packages or declare frame shapes; they
  come from `@demicodes/protocol`.

#### `@demicodes/utils`

- **Published** to npm.
- **Owns:** generic web app helper functions shared by `conversation-client`,
  `web-ui`, `web` and `web-gallery`.
- **Public boundary:** pure functions; no domain types or runtime services.
- **Must not:** contain domain logic or domain types, import Node, or hold
  package-specific behavior.

#### `@demicodes/web-ui`

- **Published** to npm as a source-form package: `.vue` and `.ts` source that
  the consumer's bundler compiles.
- **Owns:** the reusable Vue component library: the agent Tab, List (with its
  blocks) and Input surfaces and the assembled ChatSession page; the message
  editor and its draft and submission lifecycle (`agent/message-editor/`, the
  tiptap editor a user message is written and shown in); the user Markdown
  dialect; sidebar layout and list interaction; workspace and remote-file
  selection; file previews and the file tree as primitives; the work panel
  frame; the settings surface as presentation over host-mapped models; the
  device pairing dialog over a host-provided claim adapter; the sign-in page;
  shared UI primitives, Markdown rendering and theme; the summary text of
  queued messages, derived from their content; and the liveness of a page's
  WebSockets to the backend: the silence watch and the waits before
  connecting again, which the synchronization channel, the conversation
  sockets and the live views share, and the check of every socket when the
  page comes back (`transport/liveness.ts`,
  [Liveness and reconnection](../product/web-application.md#liveness-and-reconnection)).
  It consumes an injected `ConversationClient` and injected plugin
  services, and ships no control-plane transport of its own.
- **Public boundary:** source-path exports (`./*`) consumed by `web`,
  `web-gallery` and `plugin-sdk`.
- **Must not:** import Node, `web`, `web-gallery`, `plugin-sdk` or a plugin
  package, or know a plugin, a plugin's kind or a plugin's types.

#### `@demicodes/plugin-sdk`

- **Private** until pages from outside the repository load
  ([Pages from outside the repository](plugin-pages.md#pages-from-outside-the-repository)).
- **Owns:** the page API ([Plugin pages](plugin-pages.md)): `definePage` with
  the page object's types, the page context `usePage()`, the intents with
  their payloads, the conversation files service's interface, and the plugin
  kit, re-exported from `web-ui`. Its major version is the page API's
  version.
- **Public boundary:** its entry and subpath exports, which plugin packages,
  `web` and `web-gallery` import.
- **Must not:** know a plugin, or export a `web-ui` module that is not part of
  the page API.

#### `@demicodes/web`

- **Private.**
- **Owns:** the Vue single-page application: the application frame, route
  navigation, product state and backend request handlers, with the REST types
  generated from `internal/webapi` into `src/api/generated/`. Vue, TypeScript and Vite,
  with vue-router, Pinia and Tailwind.
- **Layout:** `main.ts` is the only composition root (app, router and
  account-scoped stores); `App.vue` is the application frame; `conversation/`
  owns chat state and containers; `targets/` environment selection;
  `settings/` settings containers; `auth/` cookie-session state and entry
  containers; `api/` validated HTTP and agent wire adapters and upload
  requests; `state/` the product state its synchronization channel keeps,
  preferences and per-user local state;
  `devices/` pairing and filesystem adapters; `plugins/` the page context
  over the page call routes, the conversation state route, the
  synchronization channel and the file routes, and the registry `tools/contractgen` generates into `plugins/generated/` from the backend's plugins
  whose page package it depends on. Reusable UI belongs to `web-ui`, a
  plugin's feature UI to its package.
- **Integration boundary:** authentication calls the backend over same-origin
  HTTP; the product state follows the synchronization channel, a WebSocket;
  chat uses the agent client over WebSocket; operations on resources and
  settings use REST. [Web application](../product/web-application.md) owns
  state ownership and operation contracts. The web app contract suite in this
  package drives the backend binary ([Scenarios](../delivery/scenarios.md)).
- **Public boundary:** `bun run web:dev` and the production build; no library
  API.
- **Must not:** import `web-gallery`, import a package by a computed name, or
  import Node outside the web app contract suite, which starts the backend, runner and scripted
  machine manager processes.

#### `@demicodes/web-gallery`

- **Private.**
- **Owns:** the Vite component catalog for `web-ui`. It remaps `web-ui` tokens
  so paradigms (tone, accent, density, radius, shadow, light and dark) can be
  compared across the catalog. Pages are vue-router paths; Markdown is a
  full-pane message route. It is not a product surface and ships no themes into
  the product.
- **Public boundary:** `bun run web:gallery`. It shows each plugin package's
  page over a fixture page context, from the registry `tools/contractgen`
  generates into `src/generated/`.
- **Must not:** import Node or `web`, or be imported by another package.

#### `@demicodes/plugin-<name>`

Each plugin page package is **private** and has the same boundary:

- **Public boundary:** its page object, the default export of its entry,
  which the generated registry imports because its plugin's manifest names
  the package ([Registration](plugin-pages.md#registration)); and
  source-path exports (`./*`), which only the gallery's specimens import. Its
  plugin's id and types are generated from that manifest into
  `src/generated/`.
- **Must not:** import a workspace package other than `plugin-sdk` and
  `utils`, know a route, or hold a primitive another page could use; that
  belongs to `web-ui`, exposed through the SDK.

| Package | Owns |
| --- | --- |
| `@demicodes/plugin-browser` | The `browser` work panel kind and the live view ([Live view](../browser/live-view.md#responsibilities)): frames, input, clipboard and pictures over its page context, with the plugin's page types and the live view's messages generated into `src/generated/` |
| `@demicodes/plugin-expose` | The conversation header's expose menu and the `page` work panel kind ([Host expose](../execution/expose.md#product-surface)), with the plugin's page types generated into `src/generated/` |
| `@demicodes/plugin-skills` | The Skills settings section with its dialogs ([Skills](../agent/skills.md#the-page)), with the plugin's page types generated into `src/generated/` |
| `@demicodes/plugin-changes` | The pinned `change` kind, the Change view, over the conversation files service ([Changes](../product/file-previews.md#changes)) |
| `@demicodes/plugin-file-browser` | The pinned `file` kind, the File view, over the conversation files service ([File previews](../product/file-previews.md)) |

#### `cloud-guest-image` (not a workspace package)

- **Owns:** the chroot steps of the Linux Cloud image build (debootstrap, apt,
  the `demi` user and sudo configuration, init and the shell skeleton), written
  as shell, and the pin of uv. `tools/release` packages the result: it embeds the
  verified runner and native releases, Chrome and uv, and writes
  `rootfs.tar.zst` with its manifest. [Cloud images](../cloud/images.md) owns
  the artifact and build contract.
- **Build boundary:** filesystem assembly runs at build time; native binaries
  come from the machine's cross tools
  ([Builds and releases](../delivery/builds-and-releases.md)). No guest kernel,
  bootloader or hypervisor configuration.
- **Must not:** own runtime OCI policy, mounts, networking, device credentials,
  lifecycle or backend configuration.

## Dependency graphs

Each graph lists every member once, as `path -> dependency, dependency`, with
`none` for no first-party dependencies. Each stays one fenced `text` block.
External libraries and the third-party fork are outside these graphs.

<a id="rust-crates"></a>

### Go packages

Paths are relative to `github.com/wspl/demi`. The table maps the crate graph,
adds the contract runtime used by generated code, and distributes split
packages' edges within their boundary. Every package may import `internal/contract`, the runtime of generated
contract code, whether or not its line names it. Test support has its own lines.
Production imports follow the listed edges. For `TestImports` and
`XTestImports`, an owner may also import its own support and a consumer may
import support of a listed dependency. The check resolves such test-only
edges to the support's owner; it permits neither runtime production imports
of test support nor upward implementation imports. Support packages may
import other support packages only through their explicitly listed edges. This keeps production and support
graphs acyclic without treating a black-box test as a production dependency.
The native fixture is a test program and may import its listed test support.

```text
internal/core -> internal/contract
internal/contract -> none
internal/framewire -> internal/core, internal/contract
internal/commandwire -> internal/contract
internal/declare -> internal/contract
internal/runnerwire -> internal/commandwire, internal/declare, internal/core, internal/contract
internal/machinewire -> internal/commandwire, internal/runnerwire, internal/contract
internal/webapi -> internal/framewire, internal/commandwire, internal/core, internal/runnerwire, internal/contract
internal/cmdpkg/file/fileop -> internal/core, internal/contract
internal/cmdpkg/browser/browserop -> internal/core, internal/contract
internal/cmdpkg/claudecode/claudecodeop -> internal/contract
internal/cmdsdk -> internal/artifacts, internal/commandwire
internal/cmdpkg/file -> internal/artifacts, internal/commandwire, internal/cmdsdk, internal/core, internal/cmdpkg/file/fileop, internal/gates, internal/contract
internal/cmdpkg/browser -> internal/cmdpkg/browser/chrome/cdp, internal/cmdpkg/browser/chrome/tabs, internal/cmdpkg/browser/chrome/page, internal/cmdpkg/browser/chrome/live, internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk
internal/cmdpkg/browser/chrome/cdp -> internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk, internal/artifacts, internal/core, internal/contract
internal/cmdpkg/browser/chrome/tabs -> internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk, internal/artifacts, internal/core, internal/cmdpkg/browser/chrome/cdp, internal/contract
internal/cmdpkg/browser/chrome/page -> internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk, internal/artifacts, internal/core, internal/cmdpkg/browser/chrome/cdp, internal/cmdpkg/browser/chrome/tabs, internal/contract
internal/cmdpkg/browser/chrome/live -> internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk, internal/artifacts, internal/core, internal/cmdpkg/browser/chrome/cdp, internal/cmdpkg/browser/chrome/tabs, internal/cmdpkg/browser/chrome/page, internal/contract
internal/cmdpkg/claudecode -> internal/cmdpkg/claudecode/claudecodeop, internal/commandwire, internal/cmdsdk
internal/gates -> none
internal/artifacts -> internal/contract
internal/cli -> none
internal/version -> none
internal/host -> internal/commandwire, internal/declare, internal/core, internal/contract
internal/plugin -> internal/declare, internal/core, internal/host, internal/webapi, internal/contract
internal/plugins/browser -> internal/plugin, internal/declare, internal/cmdpkg/browser/browserop, internal/host, internal/webapi, internal/contract
internal/plugins/changes -> internal/plugin, internal/declare, internal/contract
internal/plugins/expose -> internal/plugin, internal/host, internal/core, internal/webapi, internal/declare, internal/contract
internal/plugins/file -> internal/plugin, internal/declare, internal/cmdpkg/file/fileop, internal/host, internal/contract
internal/plugins/filebrowser -> internal/plugin, internal/declare, internal/contract
internal/plugins/skills -> internal/plugin, internal/core, internal/declare, internal/contract
internal/plugins/todo -> internal/plugin, internal/host, internal/declare, internal/contract
internal/provider -> internal/core, internal/gates, internal/contract
internal/providers/anthropicapi -> internal/core, internal/provider, internal/contract
internal/providers/openaiapi -> internal/core, internal/provider, internal/contract
internal/providers/google -> internal/core, internal/provider, internal/contract
internal/providers/codex -> internal/core, internal/provider, internal/contract, internal/version
internal/providers/grokbuild -> internal/core, internal/provider, internal/contract
internal/providers/claudecode -> internal/core, internal/provider, internal/host, internal/contract, internal/version
internal/agent/store -> internal/framewire, internal/core, internal/provider, internal/host, internal/contract
internal/agent/transcript -> internal/framewire, internal/agent/store, internal/core, internal/provider
internal/agent/session -> internal/framewire, internal/agent/store, internal/agent/transcript, internal/commandwire, internal/core, internal/gates, internal/provider, internal/host, internal/contract
internal/agent/tools -> internal/framewire, internal/agent/session, internal/agent/store, internal/agent/transcript, internal/core, internal/provider, internal/host, internal/contract
internal/agent/server -> internal/framewire, internal/agent/session, internal/agent/store, internal/agent/tools, internal/agent/transcript, internal/core, internal/gates, internal/provider, internal/host, internal/declare
internal/runner -> internal/commandwire, internal/cmdsdk, internal/runner/host, internal/runner/jobs, internal/runner/process, internal/runnerwire, internal/runner/cmdpkgs, internal/runner/shell
internal/runner/process -> internal/artifacts, internal/commandwire, internal/cmdsdk, internal/runnerwire, internal/contract
internal/runner/host -> internal/artifacts, internal/cmdsdk, internal/runner/process, internal/runnerwire
internal/runner/jobs -> internal/commandwire, internal/cmdsdk, internal/declare, internal/runner/process, internal/runnerwire, internal/runner/cmdpkgs, internal/contract
internal/runner/shell -> internal/commandwire, internal/cmdsdk, internal/runner/process, internal/runnerwire, internal/runner/shell/internal/engine
internal/runner/shell/internal/engine -> internal/commandwire, internal/cmdsdk, internal/runner/process, internal/runnerwire
internal/runner/cmdpkgs -> internal/artifacts, internal/commandwire, internal/cmdsdk, internal/runner/process, internal/runnerwire, internal/contract
internal/machines -> internal/artifacts, internal/cli, internal/machinewire, internal/runnerwire, internal/machines/sandbox, internal/machines/storage, internal/machines/network, internal/machines/system, internal/version
internal/machines/sandbox -> internal/artifacts, internal/cli, internal/machinewire, internal/runnerwire, internal/machines/system, internal/contract
internal/machines/storage -> internal/artifacts, internal/cli, internal/machinewire, internal/runnerwire, internal/machines/system, internal/contract
internal/machines/network -> internal/artifacts, internal/cli, internal/machinewire, internal/runnerwire, internal/machines/system
internal/machines/system -> internal/artifacts, internal/cli, internal/contract
internal/backend -> internal/backend/accounts, internal/backend/blobs, internal/backend/cloud, internal/backend/database, internal/backend/expose, internal/backend/hostaccess, internal/backend/edge, internal/backend/providers, internal/backend/runners, internal/backend/usershard, internal/declare, internal/cmdpkg/browser/browserop, internal/plugins/browser, internal/plugins/changes, internal/plugins/file, internal/plugins/filebrowser, internal/plugin, internal/plugins/todo, internal/providers/anthropicapi, internal/providers/claudecode, internal/providers/codex, internal/provider, internal/providers/google, internal/providers/grokbuild, internal/providers/openaiapi, internal/artifacts, internal/cli, internal/gates, internal/core, internal/webapi, internal/plugins/expose, internal/plugins/skills, internal/agent/session, internal/agent/store, internal/agent/transcript, internal/backend/plugins, internal/host, internal/runnerwire, internal/backend/remotehost, internal/commandwire, internal/framewire, internal/cmdpkg/claudecode/claudecodeop, internal/runner/cmdpkgs
internal/backend/accounts -> internal/backend/database, internal/commandwire, internal/core, internal/webapi
internal/backend/blobs -> internal/agent/store, internal/core, internal/webapi, internal/contract
internal/backend/cloud -> internal/backend/idlewatch, internal/backend/providers, internal/backend/runners, internal/backend/database, internal/backend/pagesync, internal/gates, internal/backend/remotehost, internal/machinewire, internal/runnerwire, internal/host, internal/webapi
internal/backend/database -> internal/agent/store, internal/agent/transcript, internal/core, internal/gates, internal/backend/remotehost, internal/machinewire, internal/runnerwire, internal/host, internal/webapi, internal/plugin
internal/backend/expose -> internal/backend/database, internal/core, internal/webapi
internal/backend/hostaccess -> internal/agent/store, internal/agent/tools, internal/backend/cloud, internal/backend/blobs, internal/backend/runners, internal/backend/database, internal/commandwire, internal/declare, internal/core, internal/gates, internal/backend/remotehost, internal/runnerwire, internal/host, internal/webapi, internal/plugin
internal/backend/edge -> internal/framewire, internal/agent/store, internal/artifacts, internal/backend/accounts, internal/backend/cloud, internal/backend/expose, internal/backend/hostaccess, internal/backend/blobs, internal/backend/providers, internal/backend/runners, internal/backend/usershard, internal/backend/database, internal/backend/pagesync, internal/commandwire, internal/core, internal/backend/remotehost, internal/provider, internal/runnerwire, internal/host, internal/webapi, internal/backend/plugins, internal/plugin
internal/backend/idlewatch -> internal/gates
internal/backend/pagesync -> internal/backend/database, internal/webapi
internal/backend/plugins -> internal/plugin, internal/declare, internal/core, internal/host, internal/backend/database, internal/backend/pagesync, internal/webapi
internal/backend/providers -> internal/backend/database, internal/backend/pagesync, internal/cmdpkg/claudecode/claudecodeop, internal/core, internal/provider, internal/providers/anthropicapi, internal/providers/claudecode, internal/providers/openaiapi, internal/webapi, internal/gates
internal/backend/remotehost -> internal/commandwire, internal/declare, internal/core, internal/gates, internal/runnerwire, internal/host
internal/backend/runners -> internal/artifacts, internal/backend/blobs, internal/backend/database, internal/backend/pagesync, internal/commandwire, internal/gates, internal/backend/remotehost, internal/runnerwire, internal/host, internal/webapi
internal/backend/usershard -> internal/agent/server, internal/framewire, internal/agent/session, internal/agent/store, internal/agent/tools, internal/agent/transcript, internal/backend/accounts, internal/backend/cloud, internal/backend/expose, internal/backend/hostaccess, internal/backend/idlewatch, internal/backend/blobs, internal/backend/plugins, internal/backend/providers, internal/backend/runners, internal/backend/database, internal/backend/pagesync, internal/cmdpkg/claudecode/claudecodeop, internal/commandwire, internal/declare, internal/core, internal/gates, internal/plugin, internal/backend/remotehost, internal/machinewire, internal/provider, internal/providers/claudecode, internal/runnerwire, internal/host, internal/webapi
tools/release -> internal/framewire, internal/artifacts, internal/cmdpkg/browser/browserop, internal/cmdpkg/claudecode/claudecodeop, internal/commandwire, internal/core, internal/cmdpkg/file/fileop, internal/machinewire, internal/runnerwire, internal/webapi, internal/backend, internal/plugin, internal/version, internal/backend/runners
tools/contractgen -> internal/contract, tools/contractgen/pagemeta
tools/contractgen/manifests -> internal/backend, internal/plugin, tools/contractgen/pagemeta
tools/contractgen/pagemeta -> none
tools/archcheck -> none
tools/cgocheck -> none
scripts/gomig/accept -> none
internal/programtest -> none
cmd/demi-backend -> internal/backend, internal/version
cmd/demi-runner -> internal/runner, internal/version
cmd/demi-file -> internal/cmdpkg/file
cmd/demi-browser -> internal/cmdpkg/browser
cmd/demi-claude-code -> internal/cmdpkg/claudecode
cmd/demi-machine-manager -> internal/machines
cmd/demi-native-fixture -> internal/cmdsdk, internal/commandwire/commandwiretest, internal/runner/cmdpkgs/cmdpkgstest
internal/commandwire/commandwiretest -> internal/commandwire, internal/contract
internal/cmdsdk/cmdsdktest -> internal/cmdsdk, internal/artifacts, internal/commandwire, internal/commandwire/commandwiretest
internal/cmdpkg/claudecode/claudecodetest -> internal/cmdpkg/claudecode, internal/cmdpkg/claudecode/claudecodeop, internal/commandwire, internal/cmdsdk
internal/gates/gatestest -> internal/gates
internal/artifacts/artifactstest -> internal/artifacts
internal/host/hosttest -> internal/host, internal/commandwire, internal/declare, internal/core, internal/contract
internal/plugin/plugintest -> internal/plugin, internal/declare, internal/core, internal/host, internal/webapi
internal/plugins/skills/skillstest -> internal/plugins/skills, internal/plugin, internal/core
internal/provider/providertest -> internal/provider, internal/core, internal/gates, internal/contract
internal/providers/codex/codextest -> internal/providers/codex, internal/core, internal/provider
internal/agent/store/storetest -> internal/agent/store, internal/framewire, internal/core, internal/provider, internal/host
internal/agent/transcript/transcripttest -> internal/agent/transcript, internal/framewire, internal/agent/store, internal/core, internal/provider, internal/agent/store/storetest
internal/agent/session/sessiontest -> internal/agent/session, internal/framewire, internal/agent/store, internal/agent/transcript, internal/commandwire, internal/core, internal/gates, internal/provider, internal/host, internal/agent/transcript/transcripttest, internal/provider/providertest
internal/agent/tools/toolstest -> internal/agent/tools, internal/framewire, internal/agent/session, internal/agent/store, internal/agent/transcript, internal/core, internal/provider, internal/host, internal/agent/session/sessiontest
internal/agent/server/servertest -> internal/agent/server, internal/framewire, internal/agent/session, internal/agent/store, internal/agent/tools, internal/agent/transcript, internal/core, internal/gates, internal/provider, internal/host, internal/agent/tools/toolstest, internal/gates/gatestest, internal/provider/providertest, internal/agent/store/storetest
internal/runner/jobs/jobstest -> internal/runner/jobs, internal/commandwire, internal/cmdsdk, internal/declare, internal/runner/process, internal/runnerwire, internal/runner/cmdpkgs
internal/runner/shell/shelltest -> internal/runner/shell, internal/commandwire, internal/cmdsdk, internal/runner/process, internal/runnerwire, internal/runner/shell/internal/engine
internal/runner/cmdpkgs/cmdpkgstest -> internal/runner/cmdpkgs, internal/artifacts, internal/commandwire, internal/cmdsdk, internal/runner/process, internal/runnerwire, internal/commandwire/commandwiretest, internal/contract
internal/machines/system/systemtest -> internal/machines/system
internal/machines/storage/storagetest -> internal/machines/storage, internal/machinewire, internal/runnerwire, internal/contract
internal/machines/machinestest -> internal/machines, internal/artifacts, internal/cli, internal/machinewire, internal/runnerwire, internal/machines/sandbox, internal/machines/storage, internal/machines/network, internal/machines/system, internal/machines/storage/storagetest
internal/backend/backendtest -> internal/backend, internal/backend/accounts, internal/backend/blobs, internal/backend/cloud, internal/backend/database, internal/backend/expose, internal/backend/hostaccess, internal/backend/edge, internal/backend/providers, internal/backend/runners, internal/backend/usershard, internal/declare, internal/cmdpkg/browser/browserop, internal/plugins/browser, internal/plugins/changes, internal/plugins/file, internal/plugins/filebrowser, internal/plugin, internal/plugins/todo, internal/providers/anthropicapi, internal/providers/claudecode, internal/providers/codex, internal/provider, internal/providers/google, internal/providers/grokbuild, internal/providers/openaiapi, internal/artifacts, internal/cli, internal/gates, internal/core, internal/webapi, internal/plugins/expose, internal/plugins/skills, internal/backend/edge/edgetest, internal/backend/blobs/blobstest, internal/backend/usershard/usershardtest, internal/backend/database/databasetest, internal/gates/gatestest, internal/backend/remotehost/remotehosttest, internal/programtest, internal/commandwire, internal/runnerwire, internal/machinewire, internal/host, internal/cmdpkg/file/fileop, internal/cmdpkg/claudecode/claudecodeop, internal/provider/providertest, internal/framewire
internal/backend/blobs/blobstest -> internal/backend/blobs, internal/agent/store, internal/core, internal/webapi
internal/backend/database/databasetest -> internal/backend/database, internal/agent/store, internal/agent/transcript, internal/core, internal/gates, internal/backend/remotehost, internal/machinewire, internal/runnerwire, internal/host, internal/webapi, internal/plugin
internal/backend/edge/edgetest -> internal/backend/edge, internal/framewire, internal/agent/store, internal/artifacts, internal/backend/accounts, internal/backend/cloud, internal/backend/expose, internal/backend/hostaccess, internal/backend/blobs, internal/backend/providers, internal/backend/runners, internal/backend/usershard, internal/backend/database, internal/backend/pagesync, internal/commandwire, internal/core, internal/backend/remotehost, internal/provider, internal/runnerwire, internal/host, internal/webapi, internal/backend/plugins, internal/plugin, internal/backend/usershard/usershardtest
internal/backend/remotehost/remotehosttest -> internal/backend/remotehost, internal/commandwire, internal/declare, internal/core, internal/gates, internal/runnerwire, internal/host, internal/commandwire/commandwiretest, internal/host/hosttest, internal/programtest
internal/backend/runners/runnerstest -> internal/backend/runners, internal/artifacts, internal/backend/blobs, internal/backend/database, internal/backend/pagesync, internal/commandwire, internal/gates, internal/backend/remotehost, internal/runnerwire, internal/host, internal/webapi
internal/backend/usershard/usershardtest -> internal/backend/usershard, internal/agent/server, internal/framewire, internal/agent/session, internal/agent/store, internal/agent/tools, internal/agent/transcript, internal/backend/accounts, internal/backend/cloud, internal/backend/expose, internal/backend/hostaccess, internal/backend/idlewatch, internal/backend/blobs, internal/backend/plugins, internal/backend/providers, internal/backend/runners, internal/backend/database, internal/backend/pagesync, internal/cmdpkg/claudecode/claudecodeop, internal/commandwire, internal/declare, internal/core, internal/gates, internal/plugin, internal/backend/remotehost, internal/machinewire, internal/provider, internal/providers/claudecode, internal/runnerwire, internal/host, internal/webapi
internal/cmdpkg/browser/chrome/cdp/cdptest -> internal/cmdpkg/browser/chrome/cdp, internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk, internal/artifacts, internal/core
internal/cmdpkg/browser/chrome/tabs/tabstest -> internal/cmdpkg/browser/chrome/tabs, internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk, internal/artifacts, internal/core, internal/cmdpkg/browser/chrome/cdp
internal/cmdpkg/browser/chrome/page/pagetest -> internal/cmdpkg/browser/chrome/page, internal/cmdpkg/browser/browserop, internal/commandwire, internal/cmdsdk, internal/artifacts, internal/core, internal/cmdpkg/browser/chrome/cdp, internal/cmdpkg/browser/chrome/tabs
```

### TypeScript packages

A line names a workspace package by its name without the `@demicodes/` scope.

```text
protocol -> none
utils -> none
conversation-client -> protocol, utils
web-ui -> conversation-client, protocol, utils
plugin-sdk -> web-ui
plugin-browser -> plugin-sdk, utils
plugin-changes -> plugin-sdk
plugin-expose -> plugin-sdk
plugin-file-browser -> plugin-sdk
plugin-skills -> plugin-sdk
web -> plugin-browser, plugin-changes, plugin-expose, plugin-file-browser, plugin-sdk, plugin-skills, protocol, utils, web-ui
web-gallery -> plugin-browser, plugin-changes, plugin-expose, plugin-file-browser, plugin-sdk, plugin-skills, protocol, utils, web-ui
```

## Module layout

- **One module.** `github.com/wspl/demi` contains `cmd/<program>/main.go`
  wiring, implementation packages under `internal/`, and build tools under
  `tools/`. The fork is under `third_party/`; TypeScript stays under
  `packages/`. Several Go contract packages can generate into one TypeScript
  package; paths need not mirror across languages.
- **Names and boundaries.** Use the paths above, short package names, exported
  CamelCase with Go initialisms, and no stutter (`gates.Activity`,
  `provider.Runtime`, `framewire.DecodeClientFrame`). Other details are
  unexported. Small consumer-owned interfaces describe the consumer's needs;
  constructors return concrete types.
- **Separate by responsibility.** Independent use, distribution, dependency
  isolation or build isolation justify a package, not line count. Changing
  code belongs above stable shared contracts. A sibling isolates builds; a
  chain still rebuilds its dependents.
- **One composition root per executable.** `main.go` wires the implementation
  entry point (`backend.Start` for the backend); `main.ts` wires the web app.
  Assembly constructs, injects and returns; it does not branch on business
  state. Backend domains declare narrow shard interfaces; only
  `internal/backend/usershard` implements them with `Shard`. Packages below
  it receive handles, never `Shard` or shared `Services`.
- **Files carry one responsibility.** Separate routing, policy, storage and
  wire adaptation. No catch-all `misc`, `helpers` or `utils` packages:
  generic Go comes from the standard library or an established dependency,
  generic frontend code from `@demicodes/utils`, and domain helpers live
  beside their owner.
- **Tests.** Tests live beside code in `_test.go` files, black-box
  `package x_test` by default. Internal tests are for behavior not observable
  at the public boundary. Test support is the owner's `<name>test` subpackage,
  never one that imports its consumers. Tests that start the repository's own
  programs get them from `internal/programtest` and run in the default
  `go test ./...`; only suites that need resources outside the repository
  (real Chrome, the Claude Code CLI, a real Cloud) use `//go:build acceptance`
  ([Testing](../delivery/testing.md)). Linux-only code uses
  `//go:build linux`. Process-wide resource changes run in isolated
  subprocesses.
  Shipped JavaScript, such as the capture extension, has adjacent Bun tests
  run by `bun run test`. Tests wait for events or `testing/synctest`, never
  wall-time sleeps, and never call real models. Real-model acceptance runs
  separately by hand.
- **Generated code.** Go types and `+demi:` markers are the only contract
  definitions; `tools/contractgen` generates and commits Go codecs and
  validation. Generated TypeScript stays uncommitted in
  `packages/protocol/src/generated/`, `packages/web/src/api/generated/`,
  plugin packages' `src/generated/` and the page registries described above.
- **Versions.** Dependency versions and source identifiers belong in
  `go.mod`, `go.sum` and fork metadata, not architecture inventories or
  one-off validation logs.

## Boundary checks

`tools/archcheck` reads the Go graph above, never another table, and compares
it with `go list -deps -json`, including `Imports`, `TestImports` and
`XTestImports`, across supported target and acceptance build contexts. It
fails for an undeclared first-party package or import, a missing entry, an
upward test dependency or a dependency cycle. Test-support resolution follows
the rule above. The graph is the permitted direct import set; a build-tag
variant need not use every edge. A transitive dependency does not authorize a
direct import.

`tools/cgocheck` checks each shipped program's full graph on every target
with `CGO_ENABLED=0`; no shipped graph contains cgo.
`go-check-sumtype -default-signifies-exhaustive=false` checks union switches,
and `golangci-lint` checks Go conventions and correctness. During migration,
`scripts/gomig/check.sh` runs these checks with builds, vet and tests, always
with `GOFLAGS=-mod=readonly`. Linux race tests alone use `CGO_ENABLED=1` and
`-tags netgo,osusergo`; this does not change shipped graphs. macOS FSEvents
uses purego only in the tree watch's Darwin file.

The package check reads the `text` block under
[TypeScript packages](#typescript-packages-1) and the npm workspace, and fails
unless:

- every workspace package has exactly one line, and every line names a
  package;
- each `package.json` declares exactly its line's packages as `@demicodes/*`
  entries of `dependencies`, never hidden in `devDependencies` or reached
  through another package;
- production `.ts` imports stay within the importing package's line; `.vue`
  files are covered at the manifest level;
- no production source imports a Node built-in;
- a package whose `exports` point at built files names each entry's source
  file under a `development` condition, the root `tsconfig.json` `paths`
  mirror every subpath entry, and the root `test` script names every package
  that has tests;
- no production `.ts` source outside `@demicodes/utils` declares at its top
  level a function, a function-valued variable, a class or a type under a name
  `@demicodes/utils` exports, which it imports instead; the names are read
  from `@demicodes/utils` itself, so a new helper is covered once exported;
- the graph is acyclic.

It is a test of the repository's scripts (`scripts/__tests__`), and the
frontend test suite (`bun run test`) runs it.

Other rules are enforced where they apply: Go visibility keeps internals
behind a package's exported items, and review enforces the rules in
[Concurrency](concurrency.md) and the "must not" rules of the entries above.
