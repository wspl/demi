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
runner and the backend know about them. The runner and the command-service SDK
supply mechanism: the trusted conversation and caller identity on every
invocation, and the generic conversation release with its status query
([Conversation-scoped state](../execution/native-runtime.md#conversation-scoped-state)).
The backend owns one policy: when a conversation is idle
([Conversation idle and Host resource release](../execution/resource-lifecycle.md)).
Each tool owns its own state and cleanup.

For example, the conversation browser keeps its tabs between shell jobs. Its
operations are types in `builtin-protocol`, `demi-commands` implements them,
and `coding-agent` declares them as `demi browser` commands. The tabs are
keyed by the conversation the runner names on every invocation, and they end
when the runner forwards the generic conversation release. No browser state or
browser-specific message exists in `runner`, `runner-protocol`, `host-remote`,
`agent` or the backend's conversation lifecycle. The backend names the browser
only where the user's page reaches it directly: it declares the live view's
`browser` user stream and serves the conversation browser tab routes, and
neither holds browser logic or state.

A new conversation-scoped capability is therefore added by defining its
operation types in `builtin-protocol`, implementing its commands in
`demi-commands` and declaring them in `coding-agent`; `runner`,
`runner-protocol`, `host-remote`, `agent` and the backend's conversation
lifecycle do not change. A proposal that adds a tool-specific message, adapter
or lifecycle hook to any of those crates violates this contract and is
redesigned at the port instead. A generic Host mechanism is different: a
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
  ([Module layout](#module-layout)), and no crate has a dev-dependency on a
  crate that depends on it.
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
the browser's TypeScript. A contract crate has no async runtime and no IO, with
one exception: `command-service` carries the SDK that speaks the command wire
next to the wire's types, so that a command program depends on one crate.
[Contracts](contracts.md) owns the rules these crates follow.

#### `core`

- **Owns:** the data the browser, the agent and the backend share:
  - transcript blocks (`Block`): the explicit submission `User`, the only
    editable type; `Context`, `Wakeup`, `Steer` and `AgentMessage` inputs;
    `Resume` and `Abort`; provider output; `CompactionBoundary` and
    `CompactionMarker`;
  - user and tool content (`UserContentBlock`, `MediaSource`,
    `ToolResultContentBlock`), the blob references stored media travels by
    (`BlobRef`), the tag that names an attachment to a model
    (`attachment_tag`), and the one test of a blank text (`is_blank`);
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
    `codex-backend`, `models.dev` or `cache`; its authentication and runtime
    states (`AuthState`, `RuntimeState`); its subscription accounts as the
    browser sees them (`AccountInfo`, `LoginPending`); and an account's quota
    snapshot (`QuotaSnapshot`, `QuotaWindow`, `QuotaPlan` and their sets);
  - the identities blocks and frames name (`BlockId`, `TurnId`, `NodeId`,
    `WakeupId`, `ShellId`, `CommandId`, `OperationId`);
  - the file-type table the product previews by (`preview_media_type`,
    `shows_in_place`), and the media types a model accepts with their sniffing
    (`sniff_model_media_type`);
  - base64 bytes (`B64Bytes`), times (`Timestamp`), the wall clock times are
    read from (`Clock`, `SystemClock`), the schema marker of nullable fields
    (`Nullable`), and the decode function of every boundary that receives
    these types (`decode`).
- **Public boundary:** the types and functions above.
- **Must not:** contain concrete provider names, catalog source names, shell
  runtime details, Host details, user-interface concepts, transport URLs or
  backend identifiers.

#### `agent-protocol`

- **Owns:** the conversation WebSocket's frames: `ClientFrame` and its content
  (`ClientContent`, whose `upload`, `remote_file`, `media` and `attachment`
  variants refer to files rather than carry them) with the edit request
  (`EditRequest`), `ServerFrame`, `TranscriptPatch`, `TranscriptVersion`, the
  nested edit and steer outcomes (`EditOutcome`, `SteerOutcome`),
  `SubagentJob` and `ShellStatus`; and the decode function of client frames
  (`decode_client_frame`), which tells a message that is not JSON from an
  invalid frame.
- **Public boundary:** the types above. `ClientFrame` is generic over its
  content, so the backend hands the agent the frame with the content it
  resolved (`ClientFrame::map_content`). The protocol's behavior is in
  [Frame protocol](../agent/runtime.md#frame-protocol).
- **Must not:** hold session logic or a transport, or carry file bytes inside a
  frame.

#### `command-service`

- **Owns:** the wire between an execution host and a command program, and the
  SDK that speaks it:
  - invocation metadata (`Invocation`, `LocalInvocation`), the command context
    (`CommandContext`, `CommandCaller`, `CommandLocale`), completion
    (`Completion`), service information (`ServiceInfo`) and the conversation
    release and status requests;
  - package descriptors and their identities (`PackageDescriptor`: canonical
    JSON and SHA-256 digests), artifact locations and target triples
    (`TargetTriple`);
  - the edit journal and its context (`EditContext`, `EditJournal`);
  - record framing (`Record`, `RecordDecoder`), the HTTP/2 client and server,
    the one invocation exchange (`Exchange`), bounded invocation IO and handler
    cancellation;
  - the invocation edit recorder: bounded file snapshots and a journal
    coordinated across processes by an OS file lock;
  - path resolution against an invocation's working directory, and waiting out
    a lack of open file descriptors.
- **Public boundary:** the protocol types, the client, the service entry point,
  the handler and IO traits, and the edit recorder; `command_service::testing`
  starts a service binary and drives it with a client. The runner and every
  command program use this one SDK; a command program depends on it without
  depending on the runner or on Demi's command implementations.
- **Must not:** implement commands, download artifacts, start command
  processes, hold credentials, or change the process-wide working directory or
  environment for an invocation.

#### `command-tree`

- **Owns:** command declarations, which are also the manifest's nodes (`Node`,
  `Group`, `Leaf`, `LeafKind` with its `Rpc` and `Native` bindings: a
  declaration names a native command's package and operation
  (`NativeOperation`), and `Node::pin` pins each to the descriptor a manifest
  carries, as a `Binding`); the command input subset and field table (`InputSpec::from_schema` is both the subset
  check and the table argv parsing reads); argv parsing (`Node::select`,
  `Selected::parse`, `Parsed::finish`); help rendering (`Node::help`,
  `HELP_DEFAULTS`); the settings every declaration's JSON Schema is generated
  with (`command_schema_settings`); and bounded capture and validation of
  `--json` output (`JsonCapture`).
- **Public boundary:** the items above. This crate is the single
  implementation of argv parsing, help and the input subset: the runner parses
  argv and renders `--help` with it, and the backend renders the model's
  command help and checks registrations with it. Argument validation is
  `jsonschema` over the declaration's schema at both ends, with one wording.
  Behavior: [Commands](../execution/commands.md).
- **Must not:** hold handlers or their bindings (`shell` pairs declarations
  with handlers), read stdin, or perform any other IO.

#### `builtin-protocol`

- **Owns:** the `demi.builtin` package's operations: the arguments and results
  of `file.*` and `browser.*`, the package's operation list, browser targets,
  queries, tab state, observations and resource event payloads, the live
  view's messages and frame header, the capture extension's events and
  commands, and the pinned Chrome release and receipt records. Limits are
  the crate's constants, shared by the inputs they bound; each input type
  carries its default timeout.
- **Public boundary:** the types above. `coding-agent` declares the commands
  from these types, `demi-commands` decodes invocations with them, the backend
  uses them for the conversation browser tab routes, and the page reads the
  live view's messages through `@demicodes/protocol`. Native commands, the
  coding harness, the backend and the page need one contract without importing
  each other's implementations.
- **Must not:** implement operations, transport, components, Host access,
  process management or conversation persistence.

#### `claude-protocol`

- **Owns:** the `demi.claude` package's operations, `claude.ensure` and
  `claude.status`: their arguments and results, the release record and the
  install status.
- **Public boundary:** the types above. The backend builds the release record,
  and `demi-claude` decodes it. Behavior:
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
  - `Signal`;
  - the protocol constants: `VERSION`, `MAX_MESSAGE_BYTES`, `JOB_VIEW_BYTES`,
    `STDIN_CHUNK_BYTES`, `LOG_READ_LINES` and `SERVICE_STDERR_CHARS`.
- **Conversation scope:** jobs and service streams carry the
  [command context](../execution/native-runtime.md#command-context);
  `conversation_release` is the one generic release message; the wire carries
  no browser policy.
- **Public boundary:** the items above. The runner and the backend link it; the
  machine manager uses `ManagedBoot`. Behavior:
  [Runner](../execution/runner.md).
- **Must not:** contain network IO, a Host implementation, a shell environment,
  the job table, credentials, claim policy, the device registry or conversation
  state.

#### `machines-protocol`

- **Owns:** the machine-manager socket: requests (`MachineRequest`,
  `MachineCall` with one parameter type per operation), responses
  (`MachineResponse`: `ok`, `error` and `death`), each operation's result
  (`Operation::Output`) and the line codec (`decode_request`,
  `decode_response`, `encode_line`, `MAX_LINE_BYTES`); machine image state
  (`MachineImageState`, `RuntimeState`, `Volume`) and the names of stored
  images (`DeviceId`, `GenerationId`, `BaseVersion`: one path component
  each); and the Cloud image manifest (`image::CloudImageManifest`), which
  embeds `runner-protocol`'s runner release and `command-service`'s package
  descriptors.
- **Public boundary:** the items above. The backend's machine-manager client
  and the manager link it; `xtask` writes the image manifest with it. Behavior:
  [Managed Cloud hosts](../cloud/managed-hosts.md).
- **Must not:** hold the manager's policy or IO.

#### `web-api`

- **Owns:** every REST request and response body, such as `ProductState`,
  `ConversationPatch` and `ProviderDto`; the error body (`ErrorBody`) and
  `ErrorCode`, the one list of every error code the browser can see; and the
  identifier and text types those bodies use. It reuses the runner's
  working-tree change and log-line types and `builtin-protocol`'s browser tab
  types instead of declaring them again.
- **Public boundary:** the types above; their TypeScript form is generated into
  `web`. Behavior: [Web API](../product/web-api.md).
- **Must not:** hold route handling or domain logic.

### Libraries

#### `gates`

- **Owns:** the named gates that serialize work across awaits: `ActivityGate`
  (demand and maintenance leases, reservations and its `GateState` snapshot),
  `ActivityHub`, `SerialGate` and `KeyedSerialGate`.
- **Public boundary:** the types above. Their semantics are in
  [Locks](concurrency.md#locks).
- **Must not:** know users, conversations, devices or any other domain.

#### `artifact`

- **Owns:** verified download over HTTPS with a declared size and SHA-256,
  digests, durable atomic publication, the install lock between processes,
  install receipts and archive installation. Every download-and-verify path
  and every durable atomic write goes through it: the runner's artifact cache,
  the Chrome and Claude Code installers, the machine manager's image store and
  `xtask` release packaging.
- **Public boundary:** the functions and types above.
- **Must not:** choose what to install or read release pointers: its callers
  name the location, size and digest they expect.

#### `provider`

- **Owns:**
  - the provider contract: `Provider`, one entry shared by every user and
    request (identity, capabilities, authentication status, models, failure
    reading, quota, accounts and `runtime`), and `ProviderRuntime`, one
    session's runtime (`run`, `fresh`, `close`); `InferenceRequest`,
    `ProviderEvent`, `ProviderFailure` and its `ErrorCode`;
  - the HTTP failure record and its standard reading (`HttpFailureRecord`,
    `http_failure`, `read_http_failure`), and the text of a credential a
    provider holds (`Secret`), which never prints;
  - the vendor-wire building blocks every HTTP provider decodes with:
    server-sent events, the two-step decode of payloads tagged by `type`, and
    the OpenAI-shaped Responses and Chat Completions formats with their stream
    mappers;
  - OAuth device flows (`provider::oauth`), the credential pool contract with
    single-flight refresh, quota (`ProviderQuota`), token accounting and the
    models.dev client; the catalog, state, account and quota shapes they
    return are `core`'s, because the browser receives them.
- **Public boundary:** the items above; `provider::testing` supplies scripted
  runtimes (`ScriptedRuntime`), a scripted vendor server (`MockVendor`) and a
  fixed clock (`FixedClock`). Behavior: [Providers](../providers/providers.md),
  [Models](../providers/models.md),
  [Usage and quota](../providers/usage-and-quota.md) and
  [Failures and recovery](../agent/failures-and-recovery.md).
- **Must not:** depend on concrete providers, the agent runtime, `shell` or a
  Host implementation.

#### Vendor provider crates

Each crate implements the provider contract for one vendor family.

| Crate | Owns |
|---|---|
| `provider-anthropic-api` | The Anthropic Messages API: request and stream mapping, model metadata and failure reading |
| `provider-openai-api` | The OpenAI Responses API, and the Chat Completions wire for OpenAI-compatible endpoints with their reasoning deltas and the opt-in replay of thinking as `reasoning_content`; model metadata |
| `provider-google` | The Gemini `generateContent` API, the native wire rather than the OpenAI-compatible one: request and stream mapping, including thought summaries, thought signatures and tool-returned media as inline parts; model metadata |
| `provider-codex` | The Codex Responses transport over server-sent events and WebSocket, device login, reading its failure records (usage-limit reset fields), the quota read from `x-codex-*` headers, and the model catalog |
| `provider-grok-build` | RFC 8628 device login against `auth.x.ai`, OIDC token refresh, the Chat Completions transport to the Grok Build proxy, the model catalog from `/v1/models`, and the billing and subscription quota probe |

- **Public boundary:** each crate's `Provider` implementation and its
  configuration type; transports, body builders, stream parsers and
  authentication stores stay private. Endpoint rules are in
  [Providers](../providers/providers.md).
- **Secret boundary:** keys, tokens, custom headers and raw endpoint values stay
  inside the provider and never reach a frame or response the browser sees.
- **Must not:** depend on `agent`, `shell`, `coding-agent` or a Host
  implementation.

#### `provider-claude-code`

- **Owns:** the Claude Code provider: the stream-json exchange with the CLI
  over the Host process interface, the SDK MCP channel (rmcp over an in-memory
  transport) with the model's parallel tool batches preserved and a held
  `tools/call` answered in the next run, the model catalog mapping, the OAuth
  usage quota probe, and the account token passed in the CLI's environment at
  spawn.
- **Public boundary:** its `Provider` implementation and configuration type.
  Behavior: [Claude Code](../providers/claude-code.md).
- **Secret boundary:** OAuth tokens never reach a frame or response the browser
  sees; the only process that receives one is the CLI on the user's Cloud.
- **Must not:** depend on `agent`, `coding-agent` or a Host implementation. It
  runs the CLI through the `shell` process interface it is given; which machine
  that is, is the backend's placement.

#### `shell`

- **Owns:** three contracts and nothing that implements them:
  - the Host contract: `Host` (`key`, `default_cwd`, `identity`, `fs`,
    `process`), `HostFs`, `HostProcess` and `HostKey`, the value identity of an
    execution target (equal keys mean the same Host);
  - the command system: `CommandSet`, which pairs declarations with their
    bindings and is what manifests and help read; the declaration builders; the
    rpc handler interface (`RpcHandler`, `RpcInvocation`, `RpcPort`,
    `PortTransport`, `StorageOp`); the reserved command names;
  - the shell-environment contract behind the `shell_*` tools:
    `ShellEnvironment`, `ExecRequest`, `CommandStatus` and `CommandRecord`, the
    model's status view of a command.
- **Public boundary:** the items above; `shell::testing` supplies the Host
  conformance cases and an in-memory port for rpc handler tests
  (`MemoryPort`). The Host rules are in
  [Host operations](../execution/runner.md#host-operations); the handler
  interface is [the TypeScript boundary](contracts.md#the-typescript-boundary).
- **Must not:** depend on `agent`, `provider`, a concrete provider,
  `coding-agent` or a Host implementation.

#### `agent`

- **Owns:** the agent runtime:
  - `AgentServer`, one per user shard, which holds each open conversation's
    `Tree`; `Tree`, a conversation's live nodes, its attachment to a
    connection and the supervisor operations on its subagents; `Node`;
  - `Connection`, the frame handling of one conversation socket: the backend
    hands it each decoded client frame, and its bounded outbox (`FrameRx`)
    carries every server frame back;
  - `AgentSession`, a handle over one session's `SessionCore`;
  - the transcript and its estimates (`transcript::estimate`) and compaction;
  - the standard tools (`StandardTool`: `shell_exec`, `shell_status`,
    `shell_write`, `shell_abort` and `yield`), with the durable dispatch of
    every tool call;
  - the `demi agent` command group;
  - the harness trait (`AgentHarness`), where a session's provider runtimes
    come from (`ProviderResolver`), and the tree store contract
    (`AgentTreeStore`, `SessionStore`, with the node records and checkpoints
    they carry in `store`).
- **Public boundary:** the items above; `agent::testing` supplies an in-memory
  tree store (`MemoryTreeStore`), predictable identities (`SequentialIds`) and
  a test client that drives a connection (`TestClient`). A product supplies
  the harness, the providers, a shell environment per Host and a tree store;
  the agent never knows which shell engine runs. Behavior:
  [Agent runtime](../agent/runtime.md), [Subagents](../agent/subagents.md)
  and [Compaction](../agent/compaction.md).
- **Rules:** the node assembly is the one place that creates a node's session;
  the supervisor asks it for a child and never builds one. Media persistence
  goes through the tree store: the product's store decides where media bytes
  go, and `AgentServer` never sees a blob store.
- **Must not:** depend on concrete providers, Host implementations or user
  interfaces; own a shell interpreter; own a socket. The backend owns the
  conversation socket and hands the agent decoded frames.

#### `coding-agent`

- **Owns:** the coding harness (`CodingHarness`), its system prompt, and the
  `demi` command root: `file` (native, declared from `builtin-protocol` types),
  `todo` (rpc) and `browser` (native), with product groups such as the
  backend's `host` group composed in.
- **Public boundary:** the harness and the command groups it builds.
- **Native binding:** the native groups declare package and operation ids. The
  backend supplies exact release descriptors at runtime; declarations import no
  compiled-in release catalog. The implementations live in `demi-commands`.
- **Must not:** create an `AgentSession`, an `AgentServer`, a shell
  environment, a concrete provider or a Host implementation, or replace the
  shell mechanism or the standard tools.

#### `host-remote`

- **Owns:** the backend's end of a runner:
  - the connection engine (`LinkEngine`): routing replies by id, liveness and
    rpc plumbing;
  - `RemoteHost`, a `Host` over a runner connection whose file contents travel
    through pipes, with job, working-tree, network, log and service facets;
  - pipe records (`Pipes`) and their `Send` ends;
  - `RemoteShellEnvironment`, the production `ShellEnvironment` over real
    runner jobs, and its factory;
  - building manifests from a command set.
- **Public boundary:** the items above; `host_remote::testing` supplies a real
  runner for one device (`RunnerFixture`) and an in-process fake runner
  (`TestLink`). Behavior: [Runner](../execution/runner.md) and
  [Native command execution](../execution/native-runtime.md), which owns
  [artifact-location admission](../execution/native-runtime.md#install-the-selected-executable).
- **Must not:** own sockets or HTTP routes (the backend's connection tasks and
  pipe routes feed it), claim policy, the device registry, credentials or
  conversation state.

### Executables

#### `backend` (`demi-backend`)

- **Owns:** the hosted product's server: the HTTP edge and the per-user shards
  ([Concurrency](concurrency.md)), authentication and sessions, storage, the
  credential vault, provider assembly and model catalogs, usage accounting,
  runner pairing and connections, conversation targets and host access, the
  Cloud lifecycle through the machine manager, exposes, the command manifest it
  serves to runners, the rpc handlers it runs, and the
  [user stream](../execution/native-runtime.md#user-streams) declarations, such
  as the live view's `browser` stream. Its modules are listed in
  [Backend](../backend/backend.md#request-paths-and-responsibilities).
- **Public boundary:** the `demi-backend` executable; `Backend::start` and
  `BackendConfig` for tests.
- **Must not:** be linked by another crate; put business logic in the HTTP
  layer beyond routing and validation; return secrets or proxy model traffic;
  spawn `runsc` or image tools itself (every sandbox and disk operation goes to
  the machine manager). The one credential that reaches a runner is a Claude
  Code account's token, in the CLI's environment on the user's Cloud.

#### `machines` (`demi-machines`)

- **Owns:** the Cloud machine manager on Linux: gVisor sandboxes run through
  `runsc` and their OCI bundles, private mounts and loop devices, network
  namespaces and the firewall table, cgroups, transient boot files, the
  machine-image store (paired generations, working recovery, pinned bases,
  publication and collection) and the socket server. The crate also holds the
  host install script, the Lima configuration and the pinned `runsc` build
  inputs; they implement [Cloud setup](../cloud/setup.md), not another
  lifecycle.
- **Process boundary:** `runsc`, `mke2fs`, `e2fsck`, `resize2fs`, `bsdtar` and
  `nft` are its only external programs, an intentional infrastructure
  exception; everything else is a syscall, ioctl, netlink message or file
  write. No user command becomes a host administration command. Docker and
  containerd are not dependencies.
- **Public boundary:** the `demi-machines` executable; a library target serves
  its integration tests.
- **Must not:** listen on TCP; know users, conversations or the control
  database; link the backend; execute image content on the host; offer
  alternative runtime modes. Workload credentials use `runner-protocol`'s
  `ManagedBoot`.

#### `runner` (`demi-runner`)

- **Owns:** the execution host: registration and the backend connection, Host
  operations (filesystem and process operations, file contents through pipes,
  the working tree, network and service streams), the Host log, shell jobs on
  the embedded brush shell with the standard utilities, local command
  forwarding, the artifact cache and resident service registry, and
  installation.
- **Conversation scope:** keeps each job's command context and writes it into
  every native invocation, keeps a service resident while it holds
  conversation state, and forwards the conversation release; it implements no
  browser operation.
- **Public boundary:** the executable. Behavior: [Runner](../execution/runner.md)
  and [Native command execution](../execution/native-runtime.md).
- **Must not:** own conversations or provider implementations; administer
  Cloud mounts, networking or volumes, which belong to the machine manager;
  boot as PID 1, since init belongs to the image; link Demi's command
  algorithms.

#### `demi-commands`

- **Owns:** the independently released `demi.builtin` resident program and
  every native Demi command: file read, create, edit and patch, and the
  conversation browser (the driver, the tab registry, observations, command
  operations, the live view with its capture extension and page observers, and
  resource cleanup). It routes invocations by the package's operation list.
- **Public boundary:** the executable. Behavior:
  [Commands](../execution/commands.md), [Conversation browser](../browser/browser.md),
  whose [catalog](../browser/browser.md#catalog) lists the browser commands, and
  [Live view](../browser/live-view.md).
- **Must not:** host a runner connection, define the agent's command tree, store
  conversations or be linked into the runner. Standard shell utilities belong
  to the runner.

#### `demi-claude`

- **Owns:** the independently released `demi.claude` package, which installs
  and verifies Demi's copy of the Claude Code CLI on the machine that runs it
  ([The package](../providers/claude-code.md#the-package)).
- **Public boundary:** the executable.
- **Must not:** read release pointers or choose a version, start the CLI, or be
  linked into the runner or `demi-commands`.

#### `xtask`

- **Owns:** the repository's development commands: `cargo xtask check`
  (formatting, clippy with the workspace lints, the tests and the crate
  boundary check), `cargo xtask contracts` (the TypeScript emitter,
  [Contracts](contracts.md#generated-typescript)), `cargo xtask test` (builds
  the binaries tests start and exports their paths), native build and release
  packaging for every executable, and Cloud image packaging.
- **Public boundary:** its commands; no crate links it.
- **Must not:** hold a second implementation of something a crate owns: it
  downloads and publishes through `artifact` and writes every record with its
  contract crate's type.

## TypeScript packages

The browser application and its libraries are TypeScript and Vue, as workspace
packages under `packages/`.

#### `@demicodes/protocol`

- **Published** to npm.
- **Owns:** the TypeScript form of the Rust contracts the browser reads: Zod
  schemas and `z.infer` types for agent frames, transcript blocks and patches,
  content, tool views, models, failure facts and the live view's messages, and
  the file-type table with its lookups. `cargo xtask contracts` generates all
  of it into `src/generated/`; nothing in it is written by hand, and nothing
  generated is committed.
- **Public boundary:** the generated schemas, types and lookups.
- **Must not:** declare a schema by hand or import another workspace package.

#### `@demicodes/agent-client`

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
- **Owns:** generic browser helper functions shared by `agent-client`,
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
  shared UI primitives, Markdown rendering and theme; and the summary text of
  queued messages, derived from their content. It consumes an injected
  `AgentClient` and ships no control-plane transport of its own.
- **Public boundary:** source-path exports (`./*`) consumed by `web` and
  `web-gallery`.
- **Must not:** import Node, `web` or `web-gallery`.

#### `@demicodes/web`

- **Private.**
- **Owns:** the Vue single-page application: the application frame, route
  navigation, product state and backend request handlers, with the REST types
  generated from `web-api` into `src/api/generated/`. Vue, TypeScript and Vite,
  with vue-router, Pinia and Tailwind.
- **Layout:** `main.ts` is the only composition root (app, router and
  account-scoped stores); `App.vue` is the application frame; `conversation/`
  owns chat state and containers; `targets/` environment selection;
  `settings/` settings containers; `auth/` cookie-session state and entry
  containers; `api/` validated HTTP and agent wire adapters and upload
  requests; `state/` server snapshots, preferences and per-user local state;
  `devices/` pairing and filesystem adapters. Reusable UI belongs to `web-ui`.
- **Integration boundary:** authentication calls the backend over same-origin
  HTTP; chat uses the agent client over WebSocket; resources and settings use
  REST. [Web application](../product/web-application.md) owns state ownership
  and operation contracts. The browser-contract suite in this package drives
  the backend binary ([Scenarios](../delivery/scenarios.md)).
- **Public boundary:** `bun run web:dev` and the production build; no library
  API.
- **Must not:** import `web-gallery` or Node.

#### `@demicodes/web-gallery`

- **Private.**
- **Owns:** the Vite component catalog for `web-ui`. It remaps `web-ui` tokens
  so paradigms (tone, accent, density, radius, shadow, light and dark) can be
  compared across the catalog. Pages are vue-router paths; Markdown is a
  full-pane message route. It is not a product surface and ships no themes into
  the product.
- **Public boundary:** `bun run web:gallery`.
- **Must not:** import Node or `web`, or be imported by another package.

#### `packages/guest-image` (not a workspace package)

- **Owns:** the chroot steps of the Linux Cloud image build (debootstrap, apt,
  the `demi` user and sudo configuration, init and the shell skeleton), written
  as shell. `xtask` packages the result: it embeds the verified runner and
  native releases, Chrome and uv, and writes `rootfs.tar.zst` with its
  manifest. [Cloud images](../cloud/images.md) owns the artifact and build
  contract.
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
core -> none
agent-protocol -> core
command-service -> none
command-tree -> none
builtin-protocol -> none
claude-protocol -> none
runner-protocol -> command-service, command-tree
machines-protocol -> command-service, runner-protocol
web-api -> agent-protocol, builtin-protocol, core, runner-protocol
gates -> none
artifact -> none
provider -> core
provider-anthropic-api -> core, provider
provider-openai-api -> provider
provider-google -> provider
provider-codex -> provider
provider-grok-build -> provider
provider-claude-code -> provider, shell
shell -> command-service, command-tree, core
agent -> agent-protocol, core, gates, provider, shell
coding-agent -> agent, builtin-protocol, command-tree, core, shell
host-remote -> command-service, command-tree, gates, runner-protocol, shell
backend -> agent, agent-protocol, artifact, builtin-protocol, claude-protocol, coding-agent, command-service, command-tree, core, gates, host-remote, machines-protocol, provider, provider-anthropic-api, provider-claude-code, provider-codex, provider-google, provider-grok-build, provider-openai-api, runner-protocol, shell, web-api
machines -> artifact, machines-protocol, runner-protocol
runner -> artifact, command-service, command-tree, gates, runner-protocol
demi-commands -> artifact, builtin-protocol, command-service, gates
demi-claude -> artifact, claude-protocol, command-service
xtask -> agent-protocol, artifact, builtin-protocol, claude-protocol, command-service, command-tree, core, machines-protocol, runner-protocol, web-api
```

### TypeScript packages

A line names a workspace package by its name without the `@demicodes/` scope.

```text
protocol -> none
utils -> none
agent-client -> protocol, utils
web-ui -> agent-client, protocol, utils
web -> protocol, utils, web-ui
web-gallery -> protocol, utils, web-ui
```

## Module layout

These rules decide where code goes. They are design rules, enforceable in
review.

- **Source organization.** Rust crates live under `crates/`, TypeScript
  packages under `packages/`, and vendored third-party crates under
  `vendor/<crate>/`. A separate crate or package requires independent use,
  distribution or dependency isolation; separate responsibilities within one
  consumer belong in modules. A type's correspondence across languages does
  not require matching packages: several contract crates generate into one
  `@demicodes/protocol`.
- **Crate shape.** A crate keeps its `Cargo.toml` at its root, its source under
  `src/` and Cargo's standard library and executable entry points. Each crate
  sets `[lints] workspace = true`.
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
- **Tests.** Unit tests sit beside the code, and each crate has one
  integration test binary. Test support is a `testing` cargo feature of the
  crate that owns the thing being faked (`agent::testing`,
  `provider::testing`, `host_remote::testing`, `command_service::testing`),
  never a crate that depends upward. Tests reach other programs as built
  binaries, whose paths `cargo xtask test` sets in environment variables.
- **Generated code.** The TypeScript generated from contract crates lives in
  `packages/protocol/src/generated/` and `packages/web/src/api/generated/` and
  is not committed ([Contracts](contracts.md#generated-typescript)).
- **Vendored crates.** A vendored crate keeps its upstream metadata and
  licenses. The root `Cargo.toml` declares its `[patch.crates-io]` path and
  excludes it from the workspace, so it stays outside the workspace lints.
  Demi's adapters of a vendored crate stay in the responsible Demi crate.
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
- no crate has a first-party dev-dependency on a crate that depends on it,
  directly or through other crates;
- the graph is acyclic.

`cargo xtask check` runs it.

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

The frontend test suite (`bun run test`) runs it.

Other rules are enforced where they apply: Rust visibility keeps internals
behind a crate's public items, the workspace lints enforce the rules in
[Concurrency](concurrency.md), and the remaining "must not" rules of the
entries above are enforced in review.
