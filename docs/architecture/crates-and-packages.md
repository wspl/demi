# Crates and packages

This document is the boundary contract for every Rust crate and every
TypeScript package in the repository, and the highest architectural constraint
on changes to them. It names each crate and package, its public items, what it
owns and what it must not do. Other documents describe behavior and link here
for ownership.

The two dependency graphs in [Dependency graphs](#dependency-graphs) are the
required set of crates and packages and the edges between them. Manifests
follow the graphs: when a `Cargo.toml` or `package.json` disagrees with its
line, the manifest changes, or this document changes first through a design
change. The [boundary checks](#boundary-checks) fail until both agree.

## Extension principle

Host capabilities are plugins behind one narrow port, and that port is all the
runner and the backend know about them. The runner and the command-sdk
supply mechanism: the trusted conversation and caller identity on every
invocation, and the generic conversation release with its status query
([Conversation-scoped state](../execution/native-runtime.md#conversation-scoped-state)).
The backend owns one policy: when a conversation is idle
([Conversation idle and Host resource release](../execution/resource-lifecycle.md)).
Each tool owns its own state and cleanup.

For example, the conversation browser keeps its tabs between shell jobs. Its
operations are types in `command-package-browser-protocol`, the `demi.browser` package
(`command-package-browser` with `command-package-browser-chrome`) implements them, and
`agent-coding-harness` declares them as `demi browser` commands. The tabs are keyed by
the conversation the runner names on every invocation, and they end when the
runner forwards the generic conversation release. No browser state or
browser-specific message exists in the runner's crates, `runner-protocol`,
`backend-remote-host`, the agent's crates or the backend's conversation lifecycle.
The backend names the browser only where the user's page reaches it directly:
it declares the live view's `browser` user stream and serves the conversation
browser tab routes, and neither holds browser logic or state.

A new conversation-scoped capability is therefore added as a native package
of its own: its operation types in a contract crate, its commands in its
program, and its declarations in `agent-coding-harness`; the runner, `runner-protocol`,
`backend-remote-host`, the agent and the backend's conversation lifecycle do not
change. A proposal that adds a tool-specific message, adapter or lifecycle
hook to any of those crates violates this contract and is redesigned at the
port instead. A generic Host mechanism is different: a
message the runner answers without knowing who asked or why, such as a
filesystem operation, a working-tree listing or a network stream, belongs to
the runner wire like the others.

## Dependency direction

Dependency direction is an architecture invariant. A lower-level crate or
package does not know the products, adapters, user interfaces, concrete
providers or machine implementations above it.

- The entries below are the single source of each crate's and package's
  responsibilities and boundaries. Do not scatter crate-specific rules across
  other sections. The graphs are the single source of dependencies; the
  entries do not repeat them.
- When a crate or package is added, removed, renamed or split, its entry and
  its graph line change in the same change.
- Production code never depends upward, and neither do tests: a test reaches a
  fake through the `testing` feature of the crate that owns what it fakes
  ([Module layout](#module-layout)), and no crate has a dev-dependency on
  another crate that depends on it.
- A crate's public items are the ones its entry names; everything else is
  `pub(crate)` or private. A TypeScript package exposes only what its entry
  names as its public boundary.
- Do not keep compatibility shims when a split moves an implementation to its
  final crate or package.

## Crates

Every first-party Rust crate is a member of one Cargo workspace. Vendored
third-party crates are not members ([Module layout](#module-layout)).

### Contract crates

A contract crate holds the types of one wire or data family, their serde,
schemars and garde attributes, and the decode function of each boundary that
receives them. Both ends of a wire link it, or the generator reads it to write
the browser's TypeScript. A contract crate has no async runtime and no IO.
[Contracts](contracts.md) owns the rules these crates follow.

#### `shared-types`

- **Owns:** the data the browser, the agent and the backend share:
  - transcript blocks (`Block`): the explicit submission `User`, the only
    editable type; `Context`, `Wakeup`, `Steer` and `AgentMessage` inputs;
    `Resume` and `Abort`; provider output; `CompactionBoundary` and
    `CompactionMarker`;
  - user and tool content (`UserContentBlock`, `MediaSource`,
    `DocumentSource`, `ToolResultContentBlock`, `ToolMediaSource`), whose
    media are blob references (`BlobRef`), or a URL, and never bytes; a tool
    result's medium that is gone, with why (`GoneCause`); the tag that names
    an attachment to a model
    (`attachment_tag`), the one test of a blank text with the trim that
    goes with it (`is_blank`, `trim`), and where a cut after a number of
    characters falls in a text (`char_offset`), which the transcript, the
    title and attachments cut texts at;
  - models and their selection (`Model`, `ModelSelection`, `ThinkingConfig`)
    and token usage (`TokenUsage`);
  - tool views (`ToolView`, `ShellToolView`, `OutputChunk`, `EditedFile`) and
    a command's output views, which the shell status frame and the shell
    contract are built from (`StreamView`, `OutputView`, `BinaryStdout`);
  - agent messages (`AgentMessage`, `CompletionId`);
  - the session phase, queued messages and pending steers;
  - provider failure facts (`ProviderFailureFacts`, `ProviderErrorDiagnostics`);
  - what the product shows of a provider entry: its model catalog
    (`ProviderModelList`, `ProviderModel`, `ServiceTier`, `ModelCost`), which
    carries portable facts only and never a source label such as
    `codex-backend`, `models.dev` or `cache`, with the one conversion of a
    catalog model into a selection (`ProviderModel::selection`, with the
    attachment types it derives, `ATTACHMENT_FILE_EXTENSIONS`); the wire
    format an `openai` entry speaks (`WireApi`), which its configuration names
    and its provider sends; its authentication and runtime states
    (`AuthState`, `RuntimeState`); its subscription accounts as the browser
    sees them (`AccountInfo`, `LoginPending`); and an account's quota snapshot
    (`QuotaSnapshot`, `QuotaWindow`, `QuotaPlan` and their sets);
  - the identities blocks and frames name (`BlockId`, `TurnId`, `NodeId`,
    `WakeupId`, `ShellId`, `CommandId`, `OperationId`), and the macro every
    crate declares a checked identity with (`id!`): a string newtype that
    serializes as itself and holds only the strings its check accepts;
  - the file-type table the product previews by (`preview_media_type`,
    `shows_in_place`), and the media types a model accepts with their sniffing
    (`sniff_model_media_type`);
  - base64 bytes (`B64Bytes`), times (`Timestamp`), the wall clock times are
    read from (`Clock`, `SystemClock`), the schema marker of nullable fields
    (`Nullable`), and the one decode of a boundary value with its error
    (`decode`, `decode_slice`, `DecodeError`), which every contract crate's
    boundaries use.
- **Public boundary:** the types, functions and macro above.
- **Must not:** contain concrete provider names, catalog source names, shell
  runtime details, Host details, user-interface concepts, transport URLs or
  backend identifiers.

#### `conversation-socket-protocol`

- **Owns:** the conversation WebSocket's frames: `ClientFrame` and its content
  (`ClientContent`, whose `upload`, `remote_file`, `media` and `attachment`
  variants refer to files rather than carry them) with the edit request
  (`EditRequest`), `ServerFrame`, `TranscriptPatch`, `TranscriptVersion`, the
  nested edit and steer outcomes (`EditOutcome`, `SteerOutcome`),
  `SubagentJob` and `ShellStatus`; and the decode function of client frames
  (`decode_client_frame`), which tells a message that is not JSON from an
  invalid frame.
- **Public boundary:** the types above. The agent resolves the files a
  frame's content refers to through the backend (`agent-server`'s
  `ContentResolver`) before the session sees the content. The protocol's
  behavior is in [Frame protocol](../agent/runtime.md#frame-protocol).
- **Must not:** hold session logic or a transport, or carry file bytes inside a
  frame.

#### `command-protocol`

- **Owns:** the wire between an execution host and a command program:
  - invocation metadata (`Invocation`, `LocalInvocation`), the command context
    (`CommandContext`, `CommandCaller`, `CommandLocale`), completion
    (`Completion`), service information (`ServiceInfo`), the conversation
    release and status requests, and the conversation numbers stream's
    messages (`NumbersRequest`, `NumbersAnswer`);
  - record framing (`Record`, `RecordDecoder`);
  - package descriptors and their identities (`PackageDescriptor`), artifact
    locations and target triples (`TargetTriple`), and the one canonical
    digest of a JSON value (`canonical_digest`: the SHA-256 of its RFC 8785
    form), which also identifies the agent's edit requests;
  - the edit journal and its context (`EditContext`, `EditJournal`), and the
    one test of whether bytes are text (`is_text`), which edit tracking and
    line counts read at both ends.
- **Public boundary:** the types and functions above;
  `command_protocol::testing` finds the programs a test starts beside it
  (`built_program`) and names the operations of the runner's native fixture
  service (`FIXTURE_OPERATIONS`), which the tests at both ends of the wire
  use.
- **Must not:** speak the wire: the client, the server and the recorder are
  `command-sdk`'s.

#### `command-declarations`

- **Owns:** command declarations, which are also the manifest's nodes (`Node`,
  `Group`, `Leaf`, `LeafKind` with its `Rpc` and `Native` bindings: a
  declaration names a native command's package and operation
  (`NativeOperation`), and `Node::pin` pins each to the descriptor a manifest
  carries, as a `Binding`); a declaration's schemas, compiled once, and the
  one check of a value against a schema with its one wording
  (`Schema::check`); the command input subset (`check_input_subset`, which
  registration runs); argv parsing (`Node::select`, `Selected::parse`,
  `Parsed::validate`), which reads each field's schema to convert its tokens;
  the argument check both ends run (`Leaf::check_arguments`); help rendering
  (`Node::help`, `HELP_DEFAULTS`); and the settings every declaration's JSON
  Schema is generated with (`command_schema_settings`).
- **Public boundary:** the items above. This crate is the single
  implementation of argv parsing, help and the input subset: the runner parses
  argv and renders `--help` with it, and the backend renders the model's
  command help and checks registrations with it. Argument validation is
  `jsonschema` over the declaration's schema at both ends, through the one
  check and its one wording; the runner checks `--json` output against the
  leaf's output schema with the same check.
  Behavior: [Commands](../execution/commands.md).
- **Must not:** hold handlers or their bindings (`host-interface` pairs declarations
  with handlers), read stdin, or perform any other IO.

#### `command-package-file-protocol`

- **Owns:** the `demi.file` package's id (`PACKAGE`, which its release
  descriptor names) and operations (`Operation`): the arguments and results of
  `file.read`, `file.create`, `file.edit` and `file.patch`, with their limits.
- **Public boundary:** the types above. `agent-coding-harness` declares the `demi file`
  commands from them and `demi-file` decodes invocations with them.
  Behavior: [Commands](../execution/commands.md).
- **Must not:** implement operations or perform IO.

#### `command-package-browser-protocol`

- **Owns:** the `demi.browser` package's id (`PACKAGE`) and operations
  (`Operation`): the arguments and results of `browser.*`, browser targets,
  queries, tab state, observations and resource event payloads, the live
  view's messages and frame header, the capture extension's events and
  commands, and the pinned Chrome release record. Limits are the crate's
  constants, shared by the inputs they bound; each input type carries its
  default timeout.
- **Public boundary:** the types above. `agent-coding-harness` declares the commands
  from these types, the browser's crates decode invocations with them, the
  backend uses them for the conversation browser tab routes, and the page
  reads the live view's messages through `@demicodes/protocol`. Native
  commands, the coding harness, the backend and the page need one contract
  without importing each other's implementations.
- **Must not:** implement operations, transport, components, Host access,
  process management or conversation persistence.

#### `command-package-claude-code-protocol`

- **Owns:** the `demi.claude-code` package's operations, `claude-code.ensure` and
  `claude-code.status`: their arguments and results, the release record and the
  install status.
- **Public boundary:** the types above. The backend builds the release record,
  and `demi-claude-code` decodes it. Behavior:
  [Claude Code](../providers/claude-code.md).
- **Must not:** hold install logic.

#### `runner-protocol`

- **Owns:** the wire between the backend and a runner:
  - `Inbound` and `Outbound` messages and their codec
    (`codec::{decode_inbound, decode_outbound, encode}`);
  - the manifest envelope (`Manifest`): its verification and canonical hash,
    and `Manifest::build`, so both ends compute the hash one way;
  - the managed boot record (`ManagedBoot`) and the runner release record
    (`RunnerRelease`);
  - `Signal`, and the platform a runner reports in its hello
    (`RunnerPlatform`);
  - the protocol constants: `VERSION`, `MAX_MESSAGE_BYTES`, `JOB_VIEW_BYTES`,
    `JOB_LIVE_BYTES`, `JOB_LIVE_INTERVAL`, `JOB_GROWTH_INTERVAL`,
    `STDIN_CHUNK_BYTES`, `LOG_READ_LINES` and `SERVICE_STDERR_CHARS`;
  - where a Cloud image embeds the command package executables that the
    runner starts in place of downloading them (`image::ARTIFACTS_PATH`).
- **Conversation scope:** jobs and service streams carry the
  [command context](../execution/native-runtime.md#command-context);
  `conversation_release` is the one generic release message, and
  `numbers_reserve` the one generic request for a conversation's numbers; the
  wire carries no browser policy.
- **Public boundary:** the items above. The runner and the backend link it; the
  machine manager uses `ManagedBoot`, and `machine-manager-protocol`'s image manifest
  check and `xtask`'s image build use `ARTIFACTS_PATH`. Behavior:
  [Runner](../execution/runner.md).
- **Must not:** contain network IO, a Host implementation, a shell environment,
  the job table, credentials, claim policy, the device registry or conversation
  state.

#### `machine-manager-protocol`

- **Owns:** the machine-manager socket: requests (`MachineRequest`,
  `MachineCall` with one parameter type per operation), responses
  (`MachineResponse`: `ok`, `error` and `death`), each operation's result
  (`Operation::Output`) and the line codec (`decode_request`,
  `decode_response`, `encode_line`, `MAX_LINE_BYTES`); machine image state
  (`MachineImageState`, `RuntimeState`, `Volume`) and the names of stored
  images (`DeviceId`, `GenerationId`, `BaseVersion`: one path component
  each); and the Cloud image manifest (`image::CloudImageManifest`), which
  embeds `runner-protocol`'s runner release and `command-sdk`'s package
  descriptors.
- **Public boundary:** the items above. The backend's machine-manager client
  and the manager link it; `xtask` writes the image manifest with it. Behavior:
  [Managed Cloud hosts](../cloud/managed-hosts.md).
- **Must not:** hold the manager's policy or IO.

#### `web-api-protocol`

- **Owns:** every REST request and response body, such as `ProductState`,
  `ConversationPatch` and `ProviderDto`; the messages of the page's
  synchronization channel (`SyncEvent`); the error body (`ErrorBody`) and
  `ErrorCode`, the one list of every error code the browser can see; and the
  identifier and text types those bodies use. It reuses the runner's
  working-tree change types (`GitChanges`) and platform (`RunnerPlatform`, a
  device's platform), `command-package-browser-protocol`'s browser tab types and
  `command-sdk`'s command locale (`CommandLocale`, which the browser
  reports as a preference) instead of declaring them again. A Host
  log line is its own type: the runner wire carries its time as integer
  milliseconds, the browser as an RFC 3339 time.
- **Public boundary:** the types above; their TypeScript form is generated into
  `web`. Behavior: [Web API](../product/web-api.md).
- **Must not:** hold route handling or domain logic.

### Libraries

#### `command-sdk`

- **Owns:** the SDK that speaks the command wire of `command-protocol`:
  - the HTTP/2 client and server, the one invocation exchange (`Exchange`),
    bounded invocation IO and handler cancellation, and the handler's
    conversation numbers (`Numbers`);
  - the invocation edit recorder: bounded file snapshots and a journal
    coordinated across processes by an OS file lock;
  - path resolution against an invocation's working directory, and waiting out
    a lack of open file descriptors.
- **Public boundary:** the client, the service entry point, the handler and IO
  traits, and the edit recorder; `command_sdk::testing` starts a service
  binary and drives it with a client (`ServiceProcess`), gives a handler
  numbers from counters that start at 1 (`counting_numbers`) or answers a
  service's numbers stream from them (`answer_numbers`), and counts the
  process's pauses before trying an operation again (`pauses`), which show an
  operation waiting out a lack of open files. The runner and every command program use
  this one SDK; a command program depends on it and on `command-protocol`
  without depending on the runner or on Demi's command implementations.
- **Must not:** implement commands, download artifacts, start command
  processes, hold credentials, or change the process-wide working directory or
  environment for an invocation.

#### `shared-gates`

- **Owns:** the named gates that serialize work across awaits: `ActivityGate`
  (demand and maintenance leases, reservations and its `GateState` snapshot),
  `ActivityHub`, `SerialGate` and `KeyedSerialGate`.
- **Public boundary:** the types above. Their semantics are in
  [Locks](concurrency.md#locks). Its `testing` feature adds
  `ActivityGate::waiting`, how many entrants wait behind a reservation, so a
  test waits for an operation to be held instead of for time.
- **Must not:** know users, conversations, devices or any other domain.

#### `shared-artifacts`

- **Owns:** verified download over HTTPS with a declared size and SHA-256
  (plain HTTP too for the runner's artifact cache, whose digests come from the
  pinned descriptor), and the measured download that establishes them when a
  release is prepared; digests; atomic publication, durable when asked; release
  publication (a directory of verified files and the record that describes
  them, published once and immutable); the install lock between processes;
  install receipts; and archive installation (a verified zip archive unpacked
  into a directory named by its SHA-256, with the receipt that checks it
  before each use), which installs Chrome for Testing on a paired device and
  into the Cloud image. Every download-and-verify path and every atomic
  publication of a file, durable or not, goes through it: the runner's
  artifact cache, the Chrome and Claude Code installers, the machine
  manager's image store, the edit recorder's snapshots and journal in
  `command-sdk`, and `xtask` release packaging.
- **Public boundary:** the functions and types above. Its `testing` feature
  adds a fixture HTTP server on `127.0.0.1` that downloads can reach, the count
  of waits for install locks, and `install_unpacked`, which installs a release
  a test was given unpacked, such as the Chrome suite's `DEMI_TEST_CHROME`, as
  archive installation installs a download.
- **Must not:** choose what to install or read release pointers: its callers
  name the location, size and digest they expect.

#### `provider-common`

- **Owns:**
  - the provider contract: `Provider`, one entry shared by every user and
    request (identity, capabilities, authentication status, models, failure
    reading, quota, accounts and `runtime`), and `ProviderRuntime`, one
    session's runtime (`run`, `fresh`, `close`, and its vendor's request
    limits for a model, `RequestLimits`); `InferenceRequest`, with the
    transcript as it carries it (`InferenceItem`), whose parts hold each
    medium's bytes, or a URL, and never a reference (`UserPart`, `ResultPart`,
    `MediaBytes`; a tool returns its result as `ResultPart`s too), and how it
    extends the session's earlier requests (`PromptCache`), `ProviderEvent`,
    `ProviderFailure` and its `ErrorCode`;
  - the HTTP failure record and its standard reading (`HttpFailureRecord`,
    `http_failure`, `read_http_failure`), and the text of a credential a
    provider holds (`Secret`), which never prints;
  - the vendor-wire building blocks every HTTP provider decodes with:
    server-sent events, the two-step decode of payloads tagged by `type`, and
    the OpenAI-shaped Responses and Chat Completions formats with their stream
    mappers;
  - OAuth device flows (`provider_common::oauth`); the credential pool contract
    (`CredentialPool`, `AccountDocument`) with one refresh at a time per
    account (`RefreshGates`), the one refresh protocol of every family
    (`renew`), a pool held in memory for logins and tests
    (`MemoryCredentialPool`), the account operations every subscription
    family shares (`Accounts`, over a family's `AccountKit`), and why an
    account could not be used (`AuthFailure`); quota (`ProviderQuota`), token
    accounting, and the models.dev client (`provider_common::models_dev`: the one
    copy of the document a backend keeps, `ModelsDevClient`, and its vendors
    and models as catalog models); the catalog, state, account and quota
    shapes they return are `shared-types`'s, because the browser receives them.
- **Public boundary:** the items above; `provider_common::testing` supplies scripted
  runtimes (`ScriptedRuntime`), a scripted vendor server (`MockVendor`), waits
  for a run's events that fail a test instead of hanging it (`next_event`,
  `all_events`), the check of an API-key entry's built-in catalog
  (`assert_built_in_catalog`), a fixed clock (`FixedClock`) and a wall clock
  that moves with Tokio's paused clock (`TokioClock`). Behavior:
  [Providers](../providers/providers.md),
  [Models](../providers/models.md),
  [Usage and quota](../providers/usage-and-quota.md) and
  [Failures and recovery](../agent/failures-and-recovery.md).
- **Must not:** depend on concrete providers, the agent runtime, `host-interface` or a
  Host implementation.

#### Vendor provider crates

Each crate implements the provider contract for one vendor family.

| Crate | Owns |
|---|---|
| `provider-anthropic-api` | The Anthropic Messages API: request and stream mapping, model metadata and failure reading |
| `provider-openai-api` | The OpenAI Responses API, and the Chat Completions wire for OpenAI-compatible endpoints with their reasoning deltas and the opt-in replay of thinking as `reasoning_content`; model metadata |
| `provider-google` | The Gemini `generateContent` API, the native wire rather than the OpenAI-compatible one: request and stream mapping, including thought summaries, thought signatures and tool-returned media as inline parts; model metadata |
| `provider-codex` | The Codex Responses transport over server-sent events and WebSocket, device login, token refresh, reading its failure records (usage-limit reset fields), the quota from its usage probe and the `x-codex-*` headers, and the model catalog |
| `provider-grok-build` | RFC 8628 device login against `auth.x.ai`, OIDC token refresh, the Chat Completions transport to the Grok Build proxy, the model catalog from `/v1/models`, and the quota from the billing and subscription probe and the rate-limit headers |

- **Public boundary:** each crate's `Provider` implementation and its
  configuration type; transports, body builders, stream parsers and
  authentication stores stay private. `provider_codex::testing` supplies a
  scripted Codex backend WebSocket (`FakeWebSocket`), which records what each
  connection receives. Endpoint rules are in
  [Providers](../providers/providers.md).
- **Secret boundary:** keys, tokens, custom headers and raw endpoint values stay
  inside the provider and never reach a frame or response the browser sees.
- **Must not:** depend on the agent's crates, `host-interface`, `agent-coding-harness` or a
  Host implementation.

#### `provider-claude-code`

- **Owns:** the Claude Code provider: the stream-json exchange with the CLI
  over the Host process interface, the SDK MCP channel (rmcp over an in-memory
  transport) with the model's parallel tool batches preserved and a held
  `tools/call` answered in the next run, the model catalog mapping, the OAuth
  usage quota probe, and the account token passed in the CLI's environment at
  spawn.
- **Public boundary:** its `Provider` implementation and configuration type,
  the session runtime it builds over a placement, and the placement contract
  (`Placement`): `start` starts a new CLI process from the spawn request the
  provider builds and answers the `host-interface` process. `Provider::runtime` refuses
  with `ProcessHostRequired`. Behavior:
  [Claude Code](../providers/claude-code.md#how-a-runtime-gets-its-process).
- **Secret boundary:** OAuth tokens never reach a frame or response the browser
  sees; the only process that receives one is the CLI on the user's Cloud.
- **Must not:** depend on the agent's crates, `agent-coding-harness` or a Host
  implementation. It runs the CLI through the `host-interface` process the placement
  answers; which machine that is, is the backend's placement.

#### `host-interface`

- **Owns:** three contracts and nothing that implements them:
  - the Host contract: `Host` (`key`, `default_cwd`, `identity`, `fs`,
    `process`), `HostFs`, `HostProcess` and `HostKey`, the value identity of an
    execution target (equal keys mean the same Host);
  - the command system: `CommandSet`, which pairs declarations with their
    bindings and is what manifests and help read; the declaration builders; the
    rpc handler interface (`RpcHandler`, `RpcInvocation`, `RpcPort`,
    `PortTransport`, `StorageOp`); the reserved command names;
  - the shell-environment contract behind the `shell_*` tools:
    `ShellEnvironment`, `ExecRequest`, `CommandStatus` and `CommandRecord`,
    which keeps the model's place in each command's output apart from the
    pages' view of it (`PageView`), and the feed through which an
    environment reports that view's changes and learns whether a page
    watches (`PageFeed`,
    [Live output](../agent/runtime.md#live-output)); the reading of a
    running command's kept output; a command's whole output
    (`WholeOutput`) and its lines of text (`OutputText`), which the result
    that reports a command's end and `demi shell output` read alike; where an
    environment takes the numbers of its commands and shells, the
    conversation's sequences (`Numbers`,
    [Identifiers the model sees](../agent/runtime.md#identifiers-the-model-sees));
    and the keeper to which an environment hands what a command leaves when it ends,
    its whole output and its edit copies, which the product implements over
    its storage ([The whole output](../agent/runtime.md#the-whole-output)).
- **Public boundary:** the items above; `host_interface::testing` supplies the Host
  conformance cases, an in-memory port for rpc handler tests
  (`MemoryPort`) and sequences that count from 1 (`CountingNumbers`). The Host rules are in
  [Host operations](../execution/runner.md#host-operations); the handler
  interface is [the TypeScript boundary](contracts.md#the-typescript-boundary).
- **Must not:** depend on the agent's crates, `provider-common`, a concrete provider,
  `agent-coding-harness` or a Host implementation.

#### `agent-store`

- **Owns:** what the agent keeps of a conversation and how it keeps it:
  - the tree store contract (`AgentTreeStore`, `SessionStore`, with the node
    records and checkpoints they carry), which also reads the records of
    commands' outputs for `demi shell output` and gives out the numbers of the
    conversation's sequences that the model sees
    ([Identifiers the model sees](../agent/runtime.md#identifiers-the-model-sees));
  - each node's command state history (`CommandStateHistory`,
    [Command state history](../agent/command-state-history.md));
  - the media rules (`media`): a medium stored once when it enters a
    transcript, the bytes a session holds for its requests and the model's
    view of them, over the `BlobStore` its tree store gives it, and the blobs
    each block references, media and edit copies, which a store indexes for
    [retention](../backend/storage.md#retention); the fitting of an image as
    it enters, with the `image` crate (`images`,
    [Images in the transcript](../agent/runtime.md#images-in-the-transcript));
    and the blocks an upload becomes with its recorded media type and opening
    (`attachments`).
- **Public boundary:** the items above; `agent_store::testing` supplies an
  in-memory tree store (`MemoryTreeStore`, whose calls a test can hold with
  `StoreGate`) with an in-memory blob namespace (`MemoryBlobs`), the tree
  store contract's cases that every realization passes (`store_contract`),
  and the model selections, texts and images the agent crates' tests build on
  (`test_model`, `model_of`, `model_reading`, `text`, `sent_text`, `png`).
- **Must not:** depend on the transcript, the session or the tools, or know a
  provider's runtime. The product's store decides where media bytes go; this
  crate defines only what is stored.

#### `agent-transcript`

- **Owns:** the transcript of one session: its log and the patches it sends
  (`TranscriptLog`), the identities it gives blocks and turns (`ids`), its
  estimates (`estimate`), the points it is cut at for a resume, a rewind, an
  edit or a compaction (`cut`), its replay into a provider request
  (`replay`), and the rule that retires a tool result's expired images and
  videos, which the backend applies to stored conversations (`retire`).
- **Public boundary:** the items above; `agent_transcript::testing` supplies
  predictable identities (`SequentialIds`) and the texts the model receives
  for a resume, a fired wakeup and an agent message (`RESUME_TEXT`,
  `WAKEUP_TEXT`, `agent_message_envelope`), which a session's tests compare
  its requests with.
- **Must not:** run a turn, call a provider or store anything itself.

#### `agent-session`

- **Owns:** the runtime of one session: `AgentSession`, a handle over one
  session's `SessionCore`, with its turns, input admission, steers and
  wakeups, cancellation, retry and resume, compaction
  ([Compaction](../agent/compaction.md)), message editing
  ([Message editing](../agent/message-editing.md)), persistence through the
  tree store, and its events and status; and the tool-call contract a session
  runs its tools through (`ToolInvocation`, `ToolOutcome`, `ToolEffect`,
  `ToolFailure`), which it defines and the tools implement.
- **Public boundary:** the handle, its construction and restoration
  (`SessionDeps`, `SessionInit`), its events, status and errors, the editing
  types (`EditSubmission`, `EditCheck` and their outcomes), the command
  storage a job's calls reach (`AgentSession::storage`, answering the shell's
  storage port messages), and the tool-call contract;
  `agent_session::testing` supplies the compaction request's instruction
  (`COMPACTION_SUMMARY_INSTRUCTION`). Behavior:
  [Agent runtime](../agent/runtime.md).
- **Must not:** know tools, nodes, trees or connections; a session runs a
  tool only through the tool-call contract.

#### `agent-tools`

- **Owns:** the standard tools (`StandardTool`: `shell_exec`, `shell_status`,
  `shell_write`, `shell_abort` and `yield`), their input and results, with the
  durable dispatch of every tool call over each node's shell environment per
  Host, which the product's `ShellEnvironmentFactory` makes; and the harness
  trait (`AgentHarness`) with its subagent profiles (`Profile`), its prompt
  context and the Host its nodes' shell tools reach (`AgentHarness::Host`).
- **Public boundary:** the items above; `agent_tools::testing` supplies a
  Host type for agents without shell tools (`NoHost`, `NoShells`) and the
  readers of a shell tool's result text (`field`, `shown_output`).
- **Must not:** create sessions, nodes or trees, or own a shell interpreter.

#### `agent-server`

- **Owns:** the agent server, which assembles sessions and tools into
  conversations:
  - `AgentServer`, one per user shard, which holds each open conversation's
    `Tree`; `Tree`, a conversation's live nodes, its attachment to a
    connection and the supervisor operations on its subagents; `Node`;
  - `Connection`, the frame handling of one conversation socket: the backend
    hands it each decoded client frame, and its bounded outbox (`FrameRx`)
    carries every server frame back;
  - each node's command storage as a job's rpc calls reach it
    (`AgentServer::command_storage`), at the history generation the job
    started in ([Mutation API and concurrency](../agent/command-state-history.md#mutation-api-and-concurrency));
  - the `demi agent` and `demi shell` command groups;
  - where a session's provider runtimes come from (`ProviderResolver`), the
    notice to the product that a live tree started or stopped working or was
    disposed (`ServerDeps::status_changed`), the resolution of the files a
    frame's content refers to, which the backend answers
    (`ContentResolver`), and the conversation title request and its rules
    (`title`).
- **Public boundary:** the items above; `agent_server::testing` supplies provider
  runtimes that play scripts (`ScriptedProviders`), uploads a frame's files
  resolve to (`TestFiles`) and a test client that drives a connection
  (`TestClient`, and `waiting_frames` for what an outbox holds). A product supplies the harness, the providers, a shell environment
  per Host and a tree store; the agent never knows which shell engine runs.
  Behavior: [Agent runtime](../agent/runtime.md) and
  [Subagents](../agent/subagents.md).
- **Rules:** the node assembly is the one place that creates a node's session;
  the supervisor asks it for a child and never builds one. A session stores
  and reads media only through its tree store: the product's store decides
  where media bytes go, and `AgentServer` never sees a blob store.
- **Must not:** depend on concrete providers, Host implementations or user
  interfaces; own a shell interpreter; own a socket. The backend owns the
  conversation socket and hands the agent decoded frames. The compaction
  fixture's harness, a program run by hand against a real model, is an example
  whose dev-dependencies include the OpenAI-compatible provider; every build of
  the one selection compiles it, and no test runs it.

#### `agent-coding-harness`

- **Owns:** the coding harness (`CodingHarness`), its system prompt, and the
  `demi` command root: `file` (native, declared from `command-package-file-protocol` types),
  `todo` (rpc) and `browser` (native, declared from `command-package-browser-protocol`
  types), with product groups such as the backend's `host` group composed in.
- **Public boundary:** the harness (`CodingHarness`) over the product's
  answer to where a node runs (`HostResolver`: a node's Host and the text that
  announces a change of it) and the `demi` root (`demi_root`, with the
  product's groups in `DemiOptions`). Its native groups bind to the `PACKAGE`
  of `command-package-file-protocol` and of `command-package-browser-protocol`, which the backend imports from
  there too.
- **Native binding:** the native groups declare package and operation ids. The
  backend supplies exact release descriptors at runtime; declarations import no
  compiled-in release catalog. The implementations live in `demi-file` and
  the browser crates.
- **Must not:** create an `AgentSession`, an `AgentServer`, a shell
  environment, a concrete provider or a Host implementation, or replace the
  shell mechanism or the standard tools.

#### `backend-remote-host`

- **Owns:** the backend's end of a runner:
  - the connection engine (`Link`, served by its `LinkDriver`): routing
    replies by id, liveness and rpc plumbing, with the product's decisions on
    calls behind `LinkPolicy`;
  - `RemoteHost`, a `Host` over a runner connection whose file contents travel
    through pipes, with job, working-tree, network, log and service facets;
  - pipe records (`Pipes`) and their `Send` ends;
  - `RemoteShellEnvironment`, the production `ShellEnvironment` over real
    runner jobs, and its factory: at a job's end it reads what the backend
    does not hold of the command's output and its edit copies, hands them to
    the product's keeper, and releases the job's directory;
  - a whole output as the runner wire's kept-output records, in which the
    backend stores it (`encode_output`, `decode_output`);
  - building manifests from a command set.
- **Public boundary:** the items above; `backend_remote_host::testing` supplies a real
  runner process for a backend at any address, with a home and state of its
  own (`RunnerProcess`), such a runner connected to a backend end of the
  fixture's own for one device (`RunnerFixture`), an in-process fake runner
  (`TestDevice`, whose connections are `TestLink`s), a native package the
  workspace built (`NativeFixture`, such as the runner's native fixture
  package) and a policy that runs every call in one command set
  (`CommandPolicy`). Behavior: [Runner](../execution/runner.md) and
  [Native command execution](../execution/native-runtime.md), which owns
  [artifact-location admission](../execution/native-runtime.md#install-the-selected-executable).
- **Must not:** own sockets or HTTP routes (the backend's connection tasks and
  pipe routes feed it), claim policy, the device registry, credentials or
  conversation state.

### Runner libraries

The runner is one executable built from six crates, so that a change to one
part of it recompiles only that part and what composes it, and a test of one
part links only that part's dependencies. The standard utilities are the
heaviest of them: only `runner-shell` links brush, the uutils, ripgrep, jaq,
sed, findutils and diffutils, and nothing below the executable depends on it.

```text
demi-runner (executable: connection, registration, Host log, composition)
   |-- runner-jobs ------> runner-command-packages ---.
   |-- runner-host ---------------------------+--> runner-process
   |-- runner-shell --------------------------'
   `-- (all of the above)
```

#### `runner-process`

- **Owns:** the runner's child processes and their IO: starting a process in
  a process group of its own with the runner's attributes (`ChildAttributes`:
  the umask and resource limits of
  [Builtins that act on a process](../execution/runner.md#builtins-that-act-on-a-process)),
  killing and reaping it; standard IO plumbing; the pipe endpoints that carry
  file contents and output to the backend's pipe routes, and the report of a
  pipe's outcome ([Pipes and output](../execution/runner.md#pipes-and-output));
  the split of a stream into lines and the kept tail of a stream; private
  state files written atomically, through `shared-artifacts`'s publication; the line
  counts of a change to a file; the
  local command client that a command alias runs
  ([External command clients](../execution/commands.md#external-command-clients));
  and the job shell contract (`JobShell`, `ShellJob`): how the runner starts
  a job's script, feeds its input, signals, cancels and awaits it, without
  knowing which shell runs it. A job's declared commands come with it as
  their root names, its execution context's id and the `command-sdk`
  `Handler` each invocation goes to (`JobCommands`).
- **Public boundary:** the items above.
- **Must not:** know the backend connection, jobs, commands, services or the
  shell that implements the contract.

#### `runner-host`

- **Owns:** the Host operations the backend asks for
  ([Host operations](../execution/runner.md#host-operations)): filesystem
  operations and file contents through pipes, the working tree with its
  status, diffs and change watch (gix and notify), network streams, and the
  volumes a Host reports. It resolves a request's paths and waits out a lack
  of open files with `command-sdk`, as every native program does, and
  writes a file's contents through `shared-artifacts`'s staged publication.
- **Public boundary:** one function per operation over its wire request, and
  the working tree's watch.
- **Must not:** know jobs, commands or the connection that carries the
  requests.

#### `runner-command-packages`

- **Owns:** the artifact cache, the preinstalled executables of a Cloud
  image, and the resident service registry: installing a pinned executable,
  starting, checking and reusing a service, its leases, conversation status
  and release, its numbers stream through a source the composition supplies
  (`NumberSource`), and its retirement
  ([Native command execution](../execution/native-runtime.md)).
- **Public boundary:** the registry and its handles (`ServiceHandle`), the
  cache, and `NumberSource`. Its `testing` feature builds
  `demi-native-fixture`, a command program with fixture operations that the
  runner's and the backend's tests install and start, and adds a number
  source that refuses (`NoNumbers`).
- **Must not:** implement a command, parse argv or know the connection.

#### `runner-shell`

- **Owns:** the embedded brush shell and the standard utilities, which run in
  process ([Shell jobs](../execution/runner.md#shell-jobs)): the shell
  options, the builtins that act on a process, the declared commands'
  builtins, which hand each invocation to the handler the job supplies, a
  utility's panic contained to its job, and the job shell contract's
  implementation, whose signals and output streams are `runner-protocol`'s.
- **Public boundary:** the `JobShell` implementation and the names the shell
  reserves (`builtin_names`), which the composition gives the command
  dispatcher.
- **Must not:** know jobs, their execution contexts, services or the
  connection: a declared command reaches the dispatcher only through the
  command-sdk `Handler` the job gives it.

#### `runner-jobs`

- **Owns:** shell jobs and the commands they run: the job table, each job's
  execution context with its command context, its directory and kept output,
  the edit report at its end, and the command dispatcher (it parses argv with
  `command-declarations`, holds a `--json` command's output until it is checked
  against the leaf's output schema, routes native invocations to their
  services and rpc calls to the backend) with local command forwarding.
- **Conversation scope:** keeps each job's command context and writes it into
  every native invocation; it implements no browser operation.
- **Public boundary:** the job table, the dispatcher, which implements the
  command-sdk `Handler` the shell calls, and the connection handle it
  sends rpc calls and reports through (`ConnectionHandle`), whose requests
  the composition's connection owner serves and whose answers it routes
  (`Relay`).
- **Must not:** know the shell that runs a job's script, beyond the job shell
  contract, or own the backend connection.

### Backend libraries

The backend is one executable built from layered crates. Each layer knows
only the layers beneath it. A domain whose operations need another domain's
state does not reach for the user's shard: it defines the narrow trait of what
it needs (`CloudShard`, `ExposeShard`, `HostShard`), writes its operations as
methods of that trait object, and `backend-user-shard` implements the trait for
`Shard` ([Composition](concurrency.md#the-user-shard)). No crate below
`backend-user-shard` sees `Shard` or `Services`; each receives the handles it uses,
such as the control service or its user's change marks.

```text
demi-backend (executable: configuration, composition)
  |-- (families) ---------> backend-providers
  `-- backend-http (HTTP)
        `-- backend-user-shard (the user's shard, conversations)
              |-- backend-host-access --> backend-cloud, backend-expose, backend-runners
              |-- backend-cloud --------> backend-runners, backend-providers, backend-idle-watch
              |-- backend-accounts
              `-- (each as it needs) --> backend-database, backend-blobs, backend-page-sync
```

#### `backend-page-sync`

- **Owns:** the change registry of the pages' synchronization channels: the
  parts of a user's product state (`Part`), and the marks each change leaves
  on that user's open channels (`SyncRegistry`, `UserMarks`)
  ([Browser synchronization](../backend/backend.md#browser-synchronization)).
  A channel is registered under the hash of the session it opened with, a
  record of `backend-database`, so a sign-out ends that session's channels.
- **Public boundary:** the items above.
- **Must not:** read or build the product state; `backend-user-shard` does.

#### `backend-database`

- **Owns:** the SQLite databases ([Storage](../backend/storage.md)): the
  control service and every control record, the conversation index, each
  conversation's database with the tree store over it, its `blob_refs` index
  and its records of commands' outputs, the schemas and their migrations, and
  the encodings of stored values; and the record types it stores, among them
  the hashes and policies of sessions and challenges, a conversation's target
  and settings changes, and catalog records, which the domains above use. The
  tree store reaches the owner's blobs through `OwnerBlobs`, the narrow trait
  of what its commits need of the namespace and the record of blob uses that
  `backend-blobs` keeps, which the shard implements.
- **Public boundary:** the stores and their records. Its `testing` feature
  opens a control database with a test key and runs raw statements behind the
  service's back, for tests of what the service must refuse, and holds the
  commits of the conversations' checkpoints (`CommitHold`).
- **Must not:** hold a domain's policy or call another backend crate.

#### `backend-blobs`

- **Owns:** the object store, on local disk or S3 through `object_store`, and
  the attachment and transcript media over it with the record of each blob's
  uses ([The object store](../backend/storage.md#the-object-store)).
- **Public boundary:** the store and the blob namespaces. Its `testing`
  feature adds an S3 fake and a store that counts its operations
  (`ObjectCounts`).
- **Must not:** hold records other than the blobs'.

#### `backend-accounts`

- **Owns:** accounts, password hashing, web sessions, login lockout and
  email-change delivery, and each user's preferences
  ([Authentication and ownership](../backend/backend.md#authentication-and-ownership),
  [User preferences](../product/web-api.md#user-preferences)).
- **Public boundary:** the account, session and preference services. The key
  that email codes are derived from is given to it; it does not derive keys.
- **Must not:** know conversations, devices or providers.

#### `backend-providers`

- **Owns:** provider assembly and model catalogs with the catalog cache; the
  credential vault: its records, their encryption and scope, subscription
  accounts, login flows and quotas; usage metering and the request rate limit;
  and the family contract every provider family implements (`ProviderFamily`,
  `FamilyRegistry`) ([Providers](../providers/providers.md),
  [Usage and quota](../providers/usage-and-quota.md)).
- **Public boundary:** the assembly, the vault, the meter, the family contract.
- **Must not:** know conversations, devices or the Cloud; run a process for a
  provider, which the shard places on the user's Cloud.

#### `backend-runners`

- **Owns:** runners and their devices: pairing with its pending claims and
  codes, the installer scripts, each device's runner connection (`Devices`)
  and the Host handles made over it, device access among them, the lease of
  a conversation's file gate (`FileGate`, `FileLease`), the rpc relay's
  routing of a job's calls, the command context a job carries, the file
  listings and text the product reads from a Host, native artifact
  publication and the development store, and the backend's public address
  that runners reach (`PublicUrl`)
  ([Runner](../execution/runner.md),
  [Native runtime](../execution/native-runtime.md#backend-deployment-configuration)).
  Publication uploads to the object store `backend-blobs` configures, and
  the file gate is a `shared-gates` gate, which is why it depends on both.
- **Public boundary:** the items above. A Host handle owned by a conversation
  is given out only against that conversation's file-gate lease:
  `Devices::conversation_host` takes a `FileLease`, which names the
  conversation of its gate, and keys the handle by it. The gates live in the
  conversations' slots of host access, and only host access takes their
  leases. Device access and machine access make handles that touch no
  conversation's files. Its `testing` feature gives a bound connection's
  driver without a socket, for tests that play a runner.
- **Must not:** reach a conversation's Host except through host access, or
  know conversations, the Cloud or exposes.

#### `backend-idle-watch`

- **Owns:** the idle rule's mechanism: one idle window, and one watch per
  resource over that resource's gates, which reserves the resource and reads
  it again before it retires it
  ([Conversation idle and Host resource release](../execution/resource-lifecycle.md)).
- **Public boundary:** the watch, its policy and the activity it reads.
- **Must not:** know what the resource is; its callers retire it.

#### `backend-cloud`

- **Owns:** each user's Cloud: policy and capacity across users, the machine
  transitions, wake, hibernate, checkpoint, reset, growth, recovery and
  maintenance, the machine manager's client, and machine access
  ([Managed hosts](../cloud/managed-hosts.md)), with its times and limits
  (`CloudTuning`); and `CloudShard`, what the Cloud needs of its user's
  shard: the handles its operations use, holding the user's conversations
  for an idle stop or a reset (`ConversationHold`), their activity and
  whether someone attends them, and the notice that a Cloud stopped.
- **Public boundary:** the Cloud component, its operations on
  `dyn CloudShard`, and the manager client.
- **Must not:** see `Shard`, conversations' state or exposes.

#### `backend-expose`

- **Owns:** expose records and their lifetime, and the live relay connections
  with their admission up to the device
  ([Host expose](../execution/expose.md)); and `ExposeShard`, what an expose
  needs of its user's shard: whether a device takes a new expose, its runner
  connected and, for a Cloud, the Cloud running.
- **Public boundary:** the exposes component and its operations on
  `dyn ExposeShard`. The `demi host expose` leaves are host commands and live
  in `backend-host-access`; the network stream of an admitted connection is
  opened by `backend-user-shard` through device access.
- **Must not:** see `Shard` or reach a Host.

#### `backend-host-access`

- **Owns:** the conversation's host access (`with_host`, the one way to a
  conversation's Host; [Host operations](../execution/sessions-and-targets.md#host-operations)),
  with each conversation's slot and its file gate, target resolution and the
  transitions that end a target (switch, archive, detach); file transfers,
  uploads, remote files and user streams, with the leases the edge holds of
  them; the shell environments of agent nodes over it, which keep the
  commands' outputs and edit copies as the user's blobs (`ConversationBlobs`);
  the product's `demi host` group with its `expose` leaves; and `HostShard`,
  what host access needs of its user's shard: the handles its operations use
  (the control service, the user's devices, pipes, command router, blobs and
  conversation databases, the native catalog and the public address, and
  the shard as the Cloud and the exposes see it) and the conversation's idle
  watch, which every Host admission starts. Uploads read the user's blobs,
  and the shells store theirs, in `backend-blobs`, which is why it depends
  on it.
- **Public boundary:** host access and its operations on `dyn HostShard`, the
  conversations' slots, the transitions, the leases, the shell environment
  factory and the `demi host` group. A slot's file gate gives its leases, the
  ones a conversation's Host is made against, only to host access; other
  work holds the conversation through the gate itself (`file_gate`), whose
  leases make no Host.
- **Must not:** see `Shard`; or leave a second way to a conversation's Host.

#### `backend-user-shard`

- **Owns:** the user shard ([The user shard](concurrency.md#the-user-shard)):
  shard threads, `Shard`, `Shards` and the shard pool, the shared services a
  shard is given (`Services`) over the storage they open, the times and
  bounds the shard and the edge run with, calls into a shard with the routing
  of the machine manager's death events, socket adoption and the page
  socket; conversations as the agent sees them: agent-tree hosting
  with the coding harness, the conversation socket, history and fork,
  summaries and titles, the providers a session resolves and its failure
  facts; the conversations' idle watches, release and the daily retention
  pass; the pages' product state and synchronization channels; the Claude Code
  CLI's work on the user's Cloud; runner adoption and the runner link's
  policy; the network stream of a relayed expose connection, opened through
  device access; and the implementations of `CloudShard`, `ExposeShard` and
  `HostShard` for `Shard`.
- **Public boundary:** the shard pool, the shared services, the times and
  bounds, and the calls the edge makes into a shard. Its `testing` feature
  starts shared services for tests (`Services::start_for_tests`), on which a
  test starts the shard pool, and adds the holds of a flow at one of its
  steps (`StepHold`) that `Backend::hold_hellos` and `Backend::hold_sync`
  return.
- **Must not:** serve HTTP.

#### `backend-http`

- **Owns:** the HTTP edge: the listener and router, the session gate, request
  extractors and body limits, the mapping of errors to `ErrorCode`, the
  installer, native artifact and browser-asset routes, runner acceptance, and
  the byte copies of file transfers, pipes, user streams and the expose relay
  ([Web API](../product/web-api.md)).
- **Public boundary:** the edge the executable starts (`Edge`), with the
  state its routes reach (`AppState`, `Site`). Its `testing` feature holds
  the runners' hellos at the token's lookup (`Backend::hold_hellos`).
- **Must not:** hold business logic beyond routing and validation.

### Browser library

#### `command-package-browser-chrome`

- **Owns:** the conversation browser's Chrome, in five modules that are
  layers: each tab's state is data in `tabs`, and what acts on a tab is
  written above it, as functions of the module that owns the action.

  ```text
  command-package-browser (program: conversations, composition)
     `-- command-package-browser-chrome
           |-- live --> page --.
           `-- cdp ------------+--> tabs --> driver
  ```

  - `driver`: Chrome and the operations run on it: the installation of the
    pinned Chrome for Testing release, launch with the capture extension and
    the capture channel, the extension's connection, which the live view
    starts captures over, the Chrome process and its profile, the operation
    type every command runs as with its cancellation and failure
    (`Operation`) and the agent it acts for, element handles, frames,
    navigation history, the conversation numbers stream, and the text and
    output a command answers with
    ([Native driver](../browser/browser.md#native-driver));
  - `tabs`: a conversation browser's environment and its tabs: the Chrome
    process tree, profile and CDP event pump of each environment, the tab
    registry and its snapshot, each tab's state and gate, the data each
    feature keeps for a tab (references, asset inventories, WebMCP tool sets,
    the console buffer, and the way to the tab's debugging owner), viewports,
    dialogs, console logs and navigation;
  - `page`: what the commands do to a page: element location and state,
    evaluation, observation, queries and probes, content and screenshots,
    keyboard, pointer, selection and select options, the actions that combine
    them, the clipboard, uploads, downloads, fetches and assets, and the
    capabilities of these families, one function per action over a
    `BrowserTab`;
  - `cdp`: raw CDP commands with their validation against the pinned
    protocol, each tab's debugging owner, which the tab's first `cdp` command
    starts, with its connections and their event pumps, and WebMCP;
    `jsonschema` validates both the pinned protocol and the schemas pages
    declare;
  - `live`: the live view ([Live view](../browser/live-view.md)): the capture
    pipeline, frame rate and pacing, the viewer connections of user streams,
    the page observer of each watched tab, and the relay of the user's input
    to the tab; and the trait through which a viewer reaches its
    conversation's browser, which `command-package-browser` implements. The
    hub starts with an environment, and the program starts it for each
    browser it runs.
- **Public boundary:** the five modules and the items above. Its `testing`
  feature adds the capture extension's id and tab numbers at hand without a
  numbers source (`driver`), Chrome's own view of its targets (`tabs`), the
  helpers that drive a page by CSS selector (`page`) and evaluation in a
  target no command addresses (`cdp`). Its tests are unit tests, since they
  read the layers' internals; the tests of the whole program are
  `command-package-browser`'s.
- **Rules:** `driver` knows no tabs, conversations or viewers; `tabs` acts on
  a page's content only to navigate, and knows no live view; `page` and `cdp`
  own no tab state, and `page` owns no CDP session; `live` knows nothing of
  how conversations are owned or released.

### Executables

#### `backend` (`demi-backend`)

- **Owns:** the hosted product's server program: its typed configuration,
  validated at startup; the instance secret and the keys derived from it; the
  built-in provider families, one per vendor crate (`families::builtin`),
  which the composition starts with; and the composition of the
  [backend libraries](#backend-libraries) in
  `Backend::start`, the one composition root: the storage, the shared services,
  the shard pool and the edge, with the command manifest it serves to runners,
  the rpc handlers it runs and the
  [user stream](../execution/native-runtime.md#user-streams) declarations,
  such as the live view's `browser` stream. Its modules and their crates are
  listed in [Backend](../backend/backend.md#request-paths-and-responsibilities).
- **Public boundary:** the `demi-backend` executable; the built-in families,
  beside which a test registers scripted ones; `Backend::start` and
  `BackendConfig` for tests, with the parts a test replaces: the provider
  families entries are assembled with (`FamilyRegistry`, `ProviderFamily` and
  the arguments a family builds a provider from, or, for a provider that needs
  a process, its session runtime over a placement), the login timing
  (`LoginTiming`), the conversations' bounds (`ConversationTuning`), the
  times of a page's sockets (`PageTuning`), the
  Cloud's and the idle clock's times and limits and the retention pass's
  schedule (`CloudTuning`, `LifecycleTuning`), the native command packages their commands bind to
  (`NativeCatalog`, which the executable and the scenarios make with
  `publish_native` from a `DEMI_NATIVE_CONFIG` file) and the user stream
  declarations. A test imports each of these from the library that owns it
  (`backend-providers`, `backend-user-shard`, `backend-cloud`, `backend-runners`),
  never through the executable. Its `testing` feature adds `Backend::hold_commits`, which
  holds the commits of the conversations' checkpoints (`CommitHold`) for the
  scenarios that stop a save at its commit
  ([Message editing](../agent/message-editing.md#durability-and-failure-boundaries)),
  `Backend::file_gate`, a conversation's file gate, whose lease is the
  conversation's work to the idle rules and whose waiting entrants show an
  operation a transition holds, `Backend::run_retention`, which runs one
  user's retention pass at once and answers when it has ended, for the
  scenarios of [Retention](../backend/storage.md#acceptance), and two holds of a flow at one of its steps
  until the test releases it (`StepHold`): `Backend::hold_hellos` holds
  runners' hellos (`HelloStep`: the token's lookup, or the shard's bind) for
  the scenarios that race a hello against its runner going away and against
  shutdown ([Runner](../execution/runner.md#connection-and-identity)), and
  `Backend::hold_sync` holds the pages' synchronization channels
  (`SyncStep`: once a channel has read its snapshot, before it sends it; or
  once a change woke it, before it takes and reads the parts that changed)
  for the scenarios that change the state while a channel waits
  ([Browser synchronization](../backend/backend.md#browser-synchronization)).
  For suites that start the executable, the example program
  `scripted_machines` runs the scripted machine manager of its scenarios
  ([Browser-contract suite](../delivery/scenarios.md#browser-contract-suite)).
- **Must not:** be linked by another crate; put business logic in the HTTP
  layer beyond routing and validation; return secrets or proxy model traffic;
  spawn `runsc` or image tools itself (every sandbox and disk operation goes to
  the machine manager). The one credential that reaches a runner is a Claude
  Code account's token, in the CLI's environment on the user's Cloud.

#### `machine-manager` (`demi-machine-manager`)

- **Owns:** the Cloud machine manager on Linux: gVisor sandboxes run through
  `runsc` and their OCI bundles, private mounts and loop devices, network
  namespaces and the firewall table, cgroups, transient boot files, the
  machine-image store (paired generations, working recovery, pinned bases,
  publication and collection) and the socket server. The crate also holds the
  host install script, the Lima configuration and the pinned `runsc` build
  inputs; they implement [Cloud setup](../cloud/setup.md) and
  [Develop on a Mac with Lima](../guides/mac-development.md), not another
  lifecycle.
- **Process boundary:** `runsc`, `mke2fs`, `e2fsck`, `resize2fs`, `bsdtar` and
  `nft` are its only external programs, an intentional infrastructure
  exception; everything else is a syscall, ioctl, netlink message or file
  write. No user command becomes a host administration command. Docker and
  containerd are not dependencies.
- **Public boundary:** the `demi-machine-manager` executable; a library target serves
  its integration tests.
- **Must not:** listen on TCP; know users, conversations or the control
  database; link the backend; execute image content on the host; offer
  alternative runtime modes. Workload credentials use `runner-protocol`'s
  `ManagedBoot`.

#### `runner` (`demi-runner`)

- **Owns:** the execution host's program: registration and the backend
  connection, the Host log, installation, and the composition of the
  [runner libraries](#runner-libraries): it gives the jobs the shell, the
  services and the connection handle, and the dispatcher the names the shell
  reserves with the runner's own; its local endpoint answers the management
  requests (`status`, `drain`) beside the dispatcher's commands. It keeps a
  service resident while it holds conversation state and forwards the
  conversation release, through `runner-command-packages`.
- **Public boundary:** the executable; the crate has no library. Its test
  binary drives the built program, as a backend does; the open-file test, a
  binary of its own, runs the runner libraries in its process. Behavior:
  [Runner](../execution/runner.md) and
  [Native command execution](../execution/native-runtime.md).
- **Must not:** own conversations or provider implementations; administer
  Cloud mounts, networking or volumes, which belong to the machine manager;
  boot as PID 1, since init belongs to the image; link Demi's command
  algorithms.

#### `command-package-file` (`demi-file`)

- **Owns:** the independently released `demi.file` resident program: file
  read, create, edit and patch, with the file mutations serialized by one
  gate and recorded through the edit recorder
  ([Edit tracking](../execution/edit-tracking.md)). It holds no conversation
  state, so it answers every conversation status as empty and ends with its
  last lease.
- **Public boundary:** the executable. Behavior:
  [Commands](../execution/commands.md).
- **Must not:** host a runner connection, define the agent's command tree,
  store conversations or be linked into the runner. Standard shell utilities
  belong to the runner.

#### `command-package-browser` (`demi-browser`)

- **Owns:** the independently released `demi.browser` resident program: the
  conversations' browsers, one owner per conversation, each with the live
  view hub of the browser it runs, routing each invocation by the package's
  operation list, the `capabilities` command, which is the one place that
  knows every command family, the sweep of orphaned profiles at start, and the
  composition of
  [`command-package-browser-chrome`](#command-package-browser-chrome).
- **Public boundary:** the executable. Behavior:
  [Conversation browser](../browser/browser.md), whose
  [catalog](../browser/browser.md#catalog) lists the browser commands, and
  [Live view](../browser/live-view.md). Its `testing` feature turns on
  `command-package-browser-chrome`'s for the Chrome tests.
- **Must not:** host a runner connection, define the agent's command tree,
  store conversations or be linked into the runner.

#### `command-package-claude-code` (`demi-claude-code`)

- **Owns:** the independently released `demi.claude-code` package, which installs
  and verifies Demi's copy of the Claude Code CLI on the machine that runs it
  ([The package](../providers/claude-code.md#the-package)).
- **Public boundary:** the executable.
- **Must not:** read release pointers or choose a version, start the CLI, or be
  linked into the runner or another command program.

#### `xtask`

- **Owns:** the repository's development commands: `xtask contracts` (the
  TypeScript emitter, which `bun run contracts` runs after it builds the
  workspace; [Contracts](contracts.md#generated-typescript)), native build and
  release packaging for every executable, the pinned Chrome for Testing
  release record, Cloud image packaging, and the comparison of the vendored
  crates with their upstream releases (`xtask vendor diff`); and the crate
  boundary check, one
  of its tests ([Boundary checks](#boundary-checks)).
  The checks and tests are plain Cargo and bun commands
  ([Validation](../delivery/builds-and-releases.md#validation)).
- **Public boundary:** its commands; no crate links it.
- **Must not:** hold a second implementation of something a crate owns: it
  downloads and publishes through `shared-artifacts` and writes every record with its
  contract crate's type.

## TypeScript packages

The browser application and its libraries are TypeScript and Vue, as workspace
packages under `packages/`.

#### `@demicodes/protocol`

- **Published** to npm.
- **Owns:** the TypeScript form of the Rust contracts the browser reads: Zod
  schemas and `z.infer` types for agent frames, transcript blocks and patches,
  content, tool views, models, failure facts and the live view's messages, and
  the file-type table with its lookups. `xtask contracts` generates all of it
  into `src/generated/`, which the package's entry re-exports; nothing else is
  written, and nothing generated is committed.
- **Public boundary:** the generated schemas, types and lookups.
- **Must not:** declare a schema by hand or import another workspace package.

#### `@demicodes/conversation-client`

- **Published** to npm.
- **Owns:** `AgentClient` and its waiters, the conversation WebSocket
  transport, client-side session events, and `applyTranscriptPatches`, the one
  transcript patch applier. It validates every frame it receives with the
  generated schemas.
- **Public boundary:** the client, the transport and the patch applier.
- **Must not:** import user-interface packages or declare frame shapes; they
  come from `@demicodes/protocol`.

#### `@demicodes/utils`

- **Published** to npm.
- **Owns:** generic browser helper functions shared by `conversation-client`,
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
  selection; the live browser view over an injected stream source; file
  previews; the settings surface as presentation over host-mapped models; the
  device pairing dialog over a host-provided claim adapter; the sign-in page;
  shared UI primitives, Markdown rendering and theme; the summary text of
  queued messages, derived from their content; and the liveness of a page's
  WebSockets to the backend: the silence watch and the waits before
  connecting again, which the synchronization channel, the conversation
  sockets and the live views share, and the check of every socket when the
  page comes back (`transport/liveness.ts`,
  [Liveness and reconnection](../product/web-application.md#liveness-and-reconnection)).
  It consumes an injected `AgentClient` and ships no control-plane transport
  of its own.
- **Public boundary:** source-path exports (`./*`) consumed by `web` and
  `web-gallery`.
- **Must not:** import Node, `web` or `web-gallery`.

#### `@demicodes/web`

- **Private.**
- **Owns:** the Vue single-page application: the application frame, route
  navigation, product state and backend request handlers, with the REST types
  generated from `web-api-protocol` into `src/api/generated/`. Vue, TypeScript and Vite,
  with vue-router, Pinia and Tailwind.
- **Layout:** `main.ts` is the only composition root (app, router and
  account-scoped stores); `App.vue` is the application frame; `conversation/`
  owns chat state and containers; `targets/` environment selection;
  `settings/` settings containers; `auth/` cookie-session state and entry
  containers; `api/` validated HTTP and agent wire adapters and upload
  requests; `state/` the product state its synchronization channel keeps,
  preferences and per-user local state;
  `devices/` pairing and filesystem adapters. Reusable UI belongs to `web-ui`.
- **Integration boundary:** authentication calls the backend over same-origin
  HTTP; the product state follows the synchronization channel, a WebSocket;
  chat uses the agent client over WebSocket; operations on resources and
  settings use REST. [Web application](../product/web-application.md) owns
  state ownership and operation contracts. The browser-contract suite in this
  package drives the backend binary ([Scenarios](../delivery/scenarios.md)).
- **Public boundary:** `bun run web:dev` and the production build; no library
  API.
- **Must not:** import `web-gallery`, or import Node outside the
  browser-contract suite, which starts the backend, runner and scripted
  machine manager processes.

#### `@demicodes/web-gallery`

- **Private.**
- **Owns:** the Vite component catalog for `web-ui`. It remaps `web-ui` tokens
  so paradigms (tone, accent, density, radius, shadow, light and dark) can be
  compared across the catalog. Pages are vue-router paths; Markdown is a
  full-pane message route. It is not a product surface and ships no themes into
  the product.
- **Public boundary:** `bun run web:gallery`.
- **Must not:** import Node or `web`, or be imported by another package.

#### `cloud-guest-image` (not a workspace package)

- **Owns:** the chroot steps of the Linux Cloud image build (debootstrap, apt,
  the `demi` user and sudo configuration, init and the shell skeleton), written
  as shell, and the pin of uv. `xtask` packages the result: it embeds the
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

Each graph lists every member once, as `name -> dependency, dependency`, with
`none` for a member without first-party dependencies. The
[boundary checks](#boundary-checks) read these blocks, so each stays one fenced
`text` block in this form. Both graphs stay acyclic; external libraries and
vendored crates are outside them.

### Rust crates

A line names a workspace member by its directory.

```text
shared-types -> none
command-protocol -> none
conversation-socket-protocol -> shared-types
command-sdk -> shared-artifacts, command-protocol
command-declarations -> none
command-package-file-protocol -> shared-types
command-package-browser-protocol -> shared-types
command-package-claude-code-protocol -> none
runner-protocol -> command-protocol, command-declarations, shared-types
machine-manager-protocol -> command-protocol, runner-protocol
web-api-protocol -> conversation-socket-protocol, command-package-browser-protocol, command-protocol, shared-types, runner-protocol
shared-gates -> none
shared-artifacts -> none
provider-common -> shared-types, shared-gates
provider-anthropic-api -> shared-types, provider-common
provider-openai-api -> shared-types, provider-common
provider-google -> shared-types, provider-common
provider-codex -> shared-types, provider-common
provider-grok-build -> shared-types, provider-common
provider-claude-code -> shared-types, provider-common, host-interface
host-interface -> command-protocol, command-declarations, shared-types
agent-store -> conversation-socket-protocol, shared-types, provider-common, host-interface
agent-transcript -> conversation-socket-protocol, agent-store, shared-types, provider-common
agent-session -> conversation-socket-protocol, agent-store, agent-transcript, command-protocol, shared-types, shared-gates, provider-common, host-interface
agent-tools -> conversation-socket-protocol, agent-session, agent-store, agent-transcript, shared-types, provider-common, host-interface
agent-server -> conversation-socket-protocol, agent-session, agent-store, agent-tools, agent-transcript, shared-types, shared-gates, provider-common, host-interface
agent-coding-harness -> agent-tools, command-package-browser-protocol, command-declarations, shared-types, command-package-file-protocol, host-interface
backend-remote-host -> command-protocol, command-declarations, shared-types, shared-gates, runner-protocol, host-interface
runner-process -> shared-artifacts, command-protocol, command-sdk, runner-protocol
runner-host -> shared-artifacts, command-sdk, runner-process, runner-protocol
runner-command-packages -> shared-artifacts, command-protocol, command-sdk, runner-process, runner-protocol
runner-shell -> command-protocol, command-sdk, runner-process, runner-protocol
runner-jobs -> command-protocol, command-sdk, command-declarations, runner-process, runner-protocol, runner-command-packages
command-package-browser-chrome -> command-package-browser-protocol, command-protocol, command-sdk, shared-artifacts, shared-types
backend-page-sync -> backend-database, web-api-protocol
backend-database -> agent-store, agent-transcript, shared-types, shared-gates, backend-remote-host, machine-manager-protocol, runner-protocol, host-interface, web-api-protocol
backend-blobs -> agent-store, shared-types, web-api-protocol
backend-accounts -> backend-database, command-protocol, shared-types, web-api-protocol
backend-providers -> backend-database, backend-page-sync, command-package-claude-code-protocol, shared-types, provider-common, provider-anthropic-api, provider-claude-code, provider-openai-api, web-api-protocol
backend-runners -> shared-artifacts, backend-blobs, backend-database, backend-page-sync, command-protocol, shared-gates, backend-remote-host, runner-protocol, host-interface, web-api-protocol
backend-idle-watch -> shared-gates
backend-cloud -> backend-idle-watch, backend-providers, backend-runners, backend-database, backend-page-sync, shared-gates, backend-remote-host, machine-manager-protocol, runner-protocol, host-interface, web-api-protocol
backend-expose -> backend-database, backend-page-sync, shared-types, web-api-protocol
backend-host-access -> agent-store, agent-tools, backend-cloud, backend-expose, backend-blobs, backend-runners, backend-database, command-protocol, command-declarations, shared-types, shared-gates, backend-remote-host, runner-protocol, host-interface, web-api-protocol
backend-user-shard -> agent-server, conversation-socket-protocol, agent-session, agent-store, agent-tools, agent-transcript, backend-accounts, backend-cloud, backend-expose, backend-host-access, backend-idle-watch, backend-blobs, backend-providers, backend-runners, backend-database, backend-page-sync, command-package-browser-protocol, command-package-claude-code-protocol, agent-coding-harness, command-protocol, command-declarations, shared-types, command-package-file-protocol, shared-gates, backend-remote-host, machine-manager-protocol, provider-common, provider-claude-code, runner-protocol, host-interface, web-api-protocol
backend-http -> conversation-socket-protocol, agent-store, shared-artifacts, backend-accounts, backend-cloud, backend-expose, backend-host-access, backend-blobs, backend-providers, backend-runners, backend-user-shard, backend-database, backend-page-sync, command-package-browser-protocol, command-protocol, shared-types, backend-remote-host, provider-common, runner-protocol, host-interface, web-api-protocol
backend -> backend-accounts, backend-blobs, backend-cloud, backend-database, backend-expose, backend-host-access, backend-http, backend-providers, backend-runners, backend-user-shard, command-declarations, command-package-browser-protocol, provider-anthropic-api, provider-claude-code, provider-codex, provider-common, provider-google, provider-grok-build, provider-openai-api, shared-artifacts, shared-gates, shared-types, web-api-protocol
machine-manager -> shared-artifacts, machine-manager-protocol, runner-protocol
runner -> command-protocol, command-sdk, runner-host, runner-jobs, runner-process, runner-protocol, runner-command-packages, runner-shell
command-package-file -> shared-artifacts, command-protocol, command-sdk, shared-types, command-package-file-protocol, shared-gates
command-package-browser -> command-package-browser-chrome, command-package-browser-protocol, command-protocol, command-sdk
command-package-claude-code -> shared-artifacts, command-package-claude-code-protocol, command-protocol, command-sdk
xtask -> conversation-socket-protocol, shared-artifacts, command-package-browser-protocol, command-package-claude-code-protocol, command-protocol, shared-types, command-package-file-protocol, machine-manager-protocol, runner-protocol, web-api-protocol
```

### TypeScript packages

A line names a workspace package by its name without the `@demicodes/` scope.

```text
protocol -> none
utils -> none
conversation-client -> protocol, utils
web-ui -> conversation-client, protocol, utils
web -> protocol, utils, web-ui
web-gallery -> protocol, utils, web-ui
```

## Module layout

These rules decide where code goes. They are design rules, enforceable in
review.

- **Source organization.** Rust crates live under `crates/`, TypeScript
  packages under `packages/`, and vendored third-party crates under
  `vendor/<upstream>/<crate>/`, one directory per upstream repository named
  after it (`uutils-coreutils`, `ripgrep`), since the crates of one repository
  are released and upgraded together. A type's correspondence across languages does not
  require matching packages: several contract crates generate into one
  `@demicodes/protocol`.
- **Names.** A crate's directory is `<owner>-<part>`: the owner says where
  the crate belongs and the part what it holds, so each name has one reading
  inside the repository and out of it. A program is named by its owner
  alone.

  | Owner | Belongs to | Program |
  | --- | --- | --- |
  | `backend-` | The server | `backend` |
  | `agent-` | The conversation runtime the backend links | |
  | `provider-` | The model providers the backend links | |
  | `runner-` | The runner on each Host | `runner` |
  | `machine-manager-` | The machine manager on a Cloud host | `machine-manager` |
  | `command-` | The command wire between a runner and a command package | |
  | `command-package-<name>-` | One command package, released as `demi.<name>` and started by a runner | `command-package-<name>` |
  | `host-` | The Host, the execution target, as the agent and the backend see it | |
  | `shared-` | No one owner: several components use it | |

  A suffix has one meaning: `-protocol` holds the types of one wire and no IO,
  `-sdk` implements one end of a wire, and `-interface` holds traits and
  nothing that implements them. Words whose meaning outside the repository
  differs stay out of names, such as `service`, `plugin`, `extension`, `edge`
  and `core`; `browser` appears only in `command-package-browser`, the
  conversation browser, and `shell` only for the shell interpreter. A
  library's package is `demi-<directory>`; a program's package is its
  executable's name (`demi-backend`, `demi-runner`, `demi-machine-manager`,
  `demi-file`, `demi-browser`, `demi-claude-code`), which releases and
  installers use.
- **When a part is its own crate.** A separate crate or package requires
  independent use, distribution, dependency isolation or build isolation;
  responsibilities that change together belong in modules of one crate. Size
  is not a reason: a crate is never split because it is long.
- **Build isolation.** Cargo compiles a crate as one unit, and it rebuilds
  every crate above a changed crate even when the change leaves the crate's
  public items as they were. A split therefore saves build time only when the
  part split off is not above the code that changes: when it is a sibling of
  that code, or below it and stable.
  - A sibling saves time. The runner's shell with its standard utilities is
    `runner-shell`, beside the runner's backend connection: a change to the
    connection recompiles neither the shell nor the utilities, and the
    connection's tests link none of them.
  - A stable contract below the code that changes saves time. The command
    wire's types are `command-protocol`, apart from the SDK in
    `command-sdk`: most crates read only the types, so a change to the SDK
    recompiles only the runner and the command programs, not the backend or
    the agent.
  - A chain costs time. A part split into crates that each depend on the one
    before still recompiles all of them when the bottom one changes, and each
    crate adds a compiler run and a test binary to link.

  So code that changes often belongs above, and the types many crates read
  belong below. A crate that many crates depend on holds only what they need
  of it.
- **Crate shape.** A crate keeps its `Cargo.toml` at its root, its source under
  `src/` and Cargo's standard library and executable entry points.
- **One composition root per executable.** Exactly one place assembles a
  program (`main.rs`, or `Backend::start` for the backend; `main.ts` for the
  web application). It may construct, inject, mount and return; it never
  branches on business state.
- **Modules mirror design modules, and files carry one responsibility.** A
  module must be nameable as a part of its crate's design, such as the
  backend's modules in [Backend](../backend/backend.md). There are no
  catch-all modules (`misc`, `helpers`, `utils`): generic Rust comes from the
  standard library or an established crate, generic browser code goes to
  `@demicodes/utils`, and domain helpers sit next to their module. A crate
  small enough to be one module needs no subdirectories.
- **Split by responsibility, not by line count.** A file that carries two of
  route handling, domain logic, storage access and wire adaptation is split,
  whatever its size; a long file with one responsibility may stay.
- **Tests.** Each crate has exactly one test binary. Cargo links a separate
  program for every test target, and each links the crate with all of its
  dependencies, so a crate's library, its executables and its integration
  tests would otherwise be three links of the same code, and a crate with
  both unit and integration tests is compiled twice. The one binary is the
  crate's integration test binary, `tests/<crate>/main.rs` with a module per
  area, which tests the crate's public boundary; the library and every
  executable target set `test = false`, and every library `doctest = false`
  and `bench = false`. The workspace has no benchmarks, and without
  `bench = false` the `--all-targets` check compiles every library a second
  time as a benchmark harness: after an edit to `command-sdk`, that
  second pass was a quarter of the check's time (18 s of 66 s on 4 x86-64
  cores). Cargo's metadata does not report the setting, so review enforces
  it.
  A crate whose program another test starts has that binary in any case:
  Cargo builds a package's programs only for its integration tests. A crate
  without programs whose behavior is observable only inside it keeps its
  tests beside the code as unit tests and has no `tests/` directory instead.
  A test gets a binary of its own only when it changes or exhausts
  process-wide state, such as the open-file limit, or needs what the others
  must not have, such as the machine manager's tests that run the manager as
  root in namespaces of their own; its file says so at the top ("its own
  binary"). The crate boundary check
  enforces these rules ([Boundary checks](#boundary-checks)). Test support is
  a `testing` cargo feature of the crate that owns the thing being faked
  (`agent_server::testing`, `provider_common::testing`, `provider_codex::testing`,
  `backend_remote_host::testing`, `command_sdk::testing`), never a crate that
  depends upward. A test reaches another program as the
  binary Cargo built into the target directory the test runs from
  (`command_protocol::testing::built_program`), and a TypeScript test through
  `DEMI_TEST_PROGRAMS` ([Validation](../delivery/builds-and-releases.md#validation)).
  JavaScript that a crate ships, such as the capture extension of
  `browser-driver`, is tested by a Bun test beside it, which `bun run test`
  runs.
- **Generated code.** The TypeScript generated from contract crates lives in
  `packages/protocol/src/generated/` and `packages/web/src/api/generated/` and
  is not committed ([Contracts](contracts.md#generated-typescript)).
- **Vendored crates.** A vendored crate keeps its upstream metadata and
  licenses, among them `.cargo_vcs_info.json`, which names the upstream commit
  of the release. It leaves out what a dependency never builds or reads:
  packaging residue (`.cargo-ok`, `Cargo.toml.orig`), its lockfile, upstream's
  CI, and its tests, benchmarks, fuzz targets and examples; its manifest stays
  as published. The root `Cargo.toml` declares its `[patch.crates-io]` path and
  excludes it from the workspace, so the workspace's formatting and tests
  leave it as upstream wrote it. `cargo xtask vendor diff` downloads each
  vendored crate's release from crates.io, checks it against the index's
  SHA-256 and lists every change from it (`--patch <crate>` prints the diff),
  which an upgrade reapplies and a review checks against the `patches`
  below.
  Demi's adapters of a vendored crate stay in the responsible Demi crate. Its
  `[package.metadata.demi]` names the Demi crate that maintains it
  (`maintainer`) and each change from the upstream release with its reason
  (`patches`); a patch compiles without warnings on every target.
- **Versions and inventories.** Dependency versions and source identifiers
  stay in manifests, lockfiles and vendor metadata. `docs/` describes
  architecture and usage, not dependency inventories, artifact hashes, CI run
  logs or one-off acceptance records.

## Boundary checks

Two checks read the graph blocks above rather than a copy of them.

The crate check reads the `text` block under [Rust crates](#rust-crates) and
the workspace's `cargo metadata`, and fails unless:

- every workspace member has exactly one line, named by its directory, and
  every line names a member;
- each crate's first-party normal and build dependencies equal its line;
- no crate has a first-party dev-dependency on another crate that depends on
  it, directly or through other crates; a crate's dev-dependency on itself,
  through which its own tests turn on its `testing` feature, is allowed;
- the graph is acyclic;
- each crate has one test binary ([Module layout](#module-layout)): of its
  library, its executables and its integration tests, one target at most
  builds tests, besides integration tests whose file says at its top why it
  is its own binary; every library has `doctest = false`.

It is a test of `xtask`, and the Rust tests run it.

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
- the graph is acyclic.

It is a test of the repository's scripts (`scripts/__tests__`), and the
frontend test suite (`bun run test`) runs it.

Other rules are enforced where they apply: Rust visibility keeps internals
behind a crate's public items, and review enforces the rules in
[Concurrency](concurrency.md) and the "must not" rules of the entries above.
