# Package Boundaries

This document is the canonical package boundary contract and the highest architecture constraint for package work. When code and this document disagree, fix the code or update this document before continuing with feature work.

The Cloud/execution entries specify the target architecture in `demi-next/`.
Documentation acceptance and runtime acceptance are separate: the dependency graph
is the required final package set, not a claim that all workspace manifests have
already been brought into agreement. Implementation checkpoints must reconcile
source, manifests and boundary checks with this contract.

## Extension principle

Host capabilities are plugins behind one narrow port, and the port is the whole
of what the framework knows about them. The runner and the native service SDK
supply mechanism: the trusted conversation and caller identity on every
invocation, and the generic conversation release with its status query, defined
in [Native runtime](demi-next/native-runtime.md#conversation-scoped-state). The
backend owns one policy, when a conversation is idle. Each tool owns its own
state and cleanup. Consequently a new conversation-scoped capability is added by
implementing commands in `demi-commands` and declaring them in `coding-agent`;
`runner`, `runner-protocol`, `host-remote`, `backend`, and `agent` do not change.
A proposal that adds a tool-specific message, adapter, or lifecycle hook to any
of those packages violates this contract and is redesigned at the port instead.
A generic Host mechanism is different: a message the runner answers without
knowing who asked or why, such as a filesystem operation, a working-tree
listing, or a network stream, belongs to the runner wire like the others.

## Source organization

TypeScript packages live under `packages/`; Rust crates live under `crates/`.
An independent package requires independent use, distribution or dependency
isolation. Separate responsibilities within one consumer belong in modules.
Cross-language type correspondence does not require matching packages.

The following tree shows all first-party Rust crates and the TypeScript packages
that own their contracts. Other TypeScript packages retain their registry entries.

```text
packages/
├── runner-protocol/
│   └── src/schemas.ts          # Authoritative Zod runner messages
├── command-loader/
│   └── src/manifest/schema.ts  # Authoritative Zod manifest structure
└── command-protocol/
    └── src/index.ts            # Authoritative Zod command wire/package contracts

crates/
├── runner/                    # Execution host; distributed as demi-runner
│   ├── Cargo.toml
│   ├── build.rs               # Generates runner messages and manifest bindings
│   └── src/
│       ├── main.rs            # Executable entry point
│       ├── connection/        # Backend connection and runner-message codec
│       ├── commands/          # Manifest, CLI parsing/help, dispatch, callbacks,
│       │                      # artifact cache and resident-process ownership
│       └── shell/             # In-process brush, standard utility adaptation
├── command-service/           # Communication SDK shared by execution hosts
│   ├── Cargo.toml             # and independently distributed command programs
│   ├── build.rs               # Generates command wire and package bindings
│   └── src/
│       ├── lib.rs             # Library entry point
│       ├── protocol.rs        # Generated types and incremental framing
│       ├── client.rs          # Caller side
│       └── server.rs          # Handler side
└── demi-commands/              # Independently distributed Demi command program
    ├── Cargo.toml
    └── src/
        ├── main.rs            # Resident service entry point
        ├── browser/           # Native browser driver and resource ownership
        ├── files.rs           # Demi file operations
        └── patch.rs           # Demi patch implementation

scripts/
├── rust-zod.ts                # Shared Zod-to-Rust generation tooling
└── native/                    # Cross-target builds and release packaging
```

Each package keeps its own source, tests and build manifest. The tree names the
modules relevant to these boundaries, not every implementation file. Vendored
third-party crates remain under `vendor/` outside the first-party workspace.

Zod schemas remain authoritative in their owning TypeScript packages. The runner
consumes runner-message and manifest definitions; command-service consumes command
wire and package definitions. The latter supplies its Rust types to both runner
and command programs. There is no separate contracts directory, mirrored Rust
protocol package or independently maintained Rust schema source.

Cargo generation writes types and validators to each consuming crate's `OUT_DIR`.
Generated Rust and intermediate schema documents do not belong in source control.
Test fixtures belong with tests; release descriptors belong with release artifacts.
Shared generation tools live under `scripts/`; protocol packages do not import
one another's private generation scripts. General-purpose operations use standard
or established libraries. Shell-specific path conventions belong to shell
adaptation, not a standalone path utility package.

## Dependency Direction

Package direction is a core architecture invariant. Lower-level packages must not know higher-level products, adapters, UI shells, concrete providers, or local machine implementations.

The package registry below is the single source of truth for per-package responsibilities and boundaries. Do not scatter package-specific rules across separate sections. When a package is added, removed, renamed, or split, update its registry entry and the dependency graph together.

Test code may depend upward for integration coverage. Production code must not.

## Package Registry

### `@demicodes/core`

- Status: implemented.
- Production deps: none.
- Owns: shared data types only: transcript blocks, content blocks, model selection, thinking config, usage, session phase, queued messages, accepted user steers awaiting transcript insertion, and the file types the product previews, by extension, with which of them the page shows in place (`file-types.ts`, `docs/demi-next/file-previews.md`), which the backend serves by and `web-ui` picks viewers from.
- Public boundary: type/data contracts shared across packages.
- Must not: contain concrete provider names, catalog source names, shell runtime details, Host details, UI concepts, transport URLs, or backend identifiers.

### `@demicodes/utils`

- Status: implemented.
- Production deps: none.
- Owns: generic, platform-neutral helper functions only — type guards, error/abort helpers, async primitives, byte/UTF-8/base64 helpers, string helpers, the portable JSON codec (`Uint8Array`/`bigint` round-trip used by agent transports and HostStore implementations), JWT payload decoding (`decodeJwtPayload`: base64url claims as `unknown`, no signature verification — the caller validates), and id generation.
- Public boundary: pure utility functions shared across packages; no domain types or runtime services.
- Must not: contain domain logic; transcript, provider, shell, or agent types; Node-only adapters; or any package-specific behavior.

### `@demicodes/provider`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/utils`, `zod`.
- Owns: abstract provider contract, inference request items, provider events, public provider shell, hidden provider runtime factory helper, auth/runtime status, required `AgentProvider.clone()` for independent per-session runtimes, unified subscription/rate-limit quota types (`ProviderQuota` / `ProviderQuotaSnapshot`; see `docs/provider-quota.md`), optional multi-credential types (`ProviderCredentials` / `ProviderCredentialInfo` — global active switch, not multi-instance providers; see `docs/provider-global-credentials.md`), the shared node-only credential pool IO behind the `@demicodes/provider/credentials-pool` subpath (the main entry stays platform-neutral; the same subpath owns `writeJsonFileAtomic`, which a provider uses to rewrite its vendor `auth.json` after a refresh), model catalog shape, the models.dev catalog client (`models-dev.ts`: the fetch with its cache and stale fallback, the zod schema of the parts read, the entry-to-catalog-model mapping) that concrete providers and the backend filter for their own lists, the token limits every catalog entry must state (`model-limits.ts`), and the vendor-wire building blocks every HTTP provider decodes with: server-sent-event framing (`sse.ts`), the two-step decode of payloads tagged by `type` (`vendor-schema.ts`: an unregistered tag is ignored, a registered tag with a malformed payload is an error), the schemas of the OpenAI-shaped formats Demi reads (`responses.ts`, `chat-completions.ts`) with the stream mappers that turn them into provider events (`responses-stream.ts`, `chat-completions-stream.ts`, the vendor's name only a label in the error text) and the two request-body fields those formats share (`openai-request.ts`), the OAuth device-login pieces (`oauth.ts`: one response decoder and the RFC 8628 seconds schemas), their token accounting (`usage.ts`), and the failure-reading contract (`Provider.readFailure`) with the standard pieces a concrete provider composes, such as the HTTP failure record and its `Retry-After` header (see `docs/provider-errors-and-retries.md`).
- Public boundary: provider contract, direct `Provider[]` composition types, quota helpers (`createProviderQuota`, `ensureQuota`), credential public types, provider test helpers only from `@demicodes/provider/testing`, and pool IO only from `@demicodes/provider/credentials-pool`.
- Model catalog boundary: common catalog state exposes portable fields only: model ids, display metadata, capability metadata, service tiers, `sourceFetchedAt`, `stale`, and `warnings`.
- Model catalog must not: expose provider-specific `source` labels such as `codex-backend`, `models.dev`, or `cache` in public types.
- Must not: import concrete providers, the agent runtime, the shell packages, or Host implementations.

### `@demicodes/shell`

- Status: implemented.
- Production deps: `@demicodes/command-protocol`, `@demicodes/utils`.
- Owns three contracts and nothing that implements them. **The Host contract** (`host.ts`): `defaultCwd`, `identity`, `fs` (a file's contents whole or as a stream of a byte range), `process` (`openCwd`, `spawn`), `store`; `fileHostStore` (a `HostStore` as JSON files on any `HostFileSystem`). **The command system** (`command.ts`, `command-abi.ts`, `reserved-names.ts`, `shell-quote.ts`): command specs and kinds, the command invocation ABI (`CommandResult`, `DispatchIO`, `NativeInvocation`, `NativeExecutor`), `CommandRegistry` with the one reserved-name table, the atomic `CommandStorage` interface. **The shell-environment contract** (`shell-environment.ts`, `command-records.ts`): `ShellEnvironment` behind the `shell_*` tools, the command record, the model's status view (each stream's delta since the last view, its tail, and the output path when the target keeps one). The production shell is `RemoteShellEnvironment` in `host-remote`, over real runner jobs.
- Entries: the root is platform-neutral TypeScript for application embedders. `testing` supplies in-memory Host and command stores and the Host conformance suite. No entry imports Node. Native leaves bind a package id and operation; an injected native executor owns execution. RPC leaves call application handlers.
- `Host.defaultCwd` is a default working-directory helper only. It is not a sandbox, workspace boundary, permission boundary, or access-control source.
- Runtime file operations go through `Host.fs`; `Host.fs` is a system-level file access facet whose allowed paths are decided by the Host backend policy, not by `defaultCwd`.
- True external process execution goes through `Host.process.spawn`.
- An `rpc` command's `run` receives the invoking shell's Host in its context; command implementations use that Host instead of closing over an assembly-time Host.
- Command JSON state (`CommandStorage`) belongs to the agent node. `agent/session` serializes its immutable versions and transcript cutoffs through `AgentTreeStore`; `backend/storage` commits them in the conversation database. The shell receives a job-bound handle from the node. `Host.store` remains an independent Host storage facet.
- HostSpawnHandle must use platform-neutral types; `kill` must not expose `NodeJS.Signals`.
- Must not: import `@demicodes/agent`, `@demicodes/provider`, concrete providers, `@demicodes/coding-agent`, a Host implementation, or Node.

### `@demicodes/agent`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: AgentSession, AgentServer, AgentClient, action-scoped caller metadata, transcript replay, compaction, `AgentSession.clone()` for isolated snapshot copies (see `docs/provider-session-clone.md`), transport frames, transcript patches, the session-tree node — one assembly for the root and every subagent (`node/`), with action-aware Host resolution, per-Host shell-environment reuse and shell-handle ownership checks — the `AgentTreeStore` persistence contract (node rows, per-node journals, the three atomic commits, completion delivery; see `docs/subagent.md` § Persistence), the model-facing standard tool surface (`shell_exec`, `shell_status`, `shell_write`, `shell_abort`, `yield`) with the durable dispatch of every tool call before it runs, AgentTool schemas/results, yield delayed-wakeup scheduling and steer-based wakeup delivery, repeated layered abort semantics, in-parent subagent supervision (ChildSupervisor as the relationship module, the injected `demi agent` command, subagent profiles, and the `subagent*` protocol frames; see `docs/subagent.md`), and assembly of one harness with the standard shell runtime.
- Public boundary: platform-neutral agent runtime and client/server protocol from root; explicit Node-only subpath `@demicodes/agent/stdio` for stdio transport only; `MemoryAgentStore`, the in-memory `AgentTreeStore` for tests and fixtures, from `@demicodes/agent/testing`.
- The shell behind the `shell_*` tools is the `ShellEnvironment` contract; `AgentServer` takes a required `shellEnvironment` factory per Host, so a product supplies the engine and the agent never knows which one runs (the backend supplies `RemoteShellEnvironment` for a `RemoteHost`; isolated tests inject test doubles, integration tests use a real runner).
- Must not: import concrete providers, Host implementations, or UI packages; must not own a shell interpreter.
- Execution coordination: authenticated live-job RPC routing, conversation target/file admission and user-device lifecycle admission. Managed operations persist allocation/reset intent and recover it before new work; every conversation using Cloud shares its device-wide operation gate.
- Runtime rule: the node assembly (`node/assemble.ts`) is the only runtime consumer that instantiates AgentSession; the supervisor asks it for a child and never builds one.
- Assembly rule: AgentServer receives one AgentHarness, a public `Provider[]`, the `shellEnvironment` factory, the `store` factory (an `AgentTreeStore` per root session id — a product's own database, never a Host's store), and shell runtime options that do not replace the shell mechanism or the standard agent tool surface. `AgentHarness.host` receives action metadata for shell operations and returns a stable Host object for each execution target; it is never called to reach storage.
- Media persistence rule: the agent defines the put/get `BlobStore` contract and the externalize/rehydrate mapping; where media bytes go is the tree store realization's decision — the backend's store externalizes every node's media into the conversation owner's blob namespace. AgentServer itself never sees a blob store.
- Session-tree options: `subagents.maxLiveSubagents` limits each session's live direct children (default 8); `notifyParentOnIdle` controls only the root's automatic wakeups. Every descendant shares the root's agent directory and inherits its tool options.
- Shell previews: `tools.shellPreviewBudgetTokens(contextWindow)` selects the token budget against each request's current model, across the entire session tree. The default is 10,000 tokens below an 800,000-token context window and 100,000 at or above it.
- Layout (directories mirror the package's modules; root keeps entrypoints, `types.ts`, and single-file modules like `tools.ts`):
  - `session/` — the AgentSession state machine and its collaborators (turn loop, steer queue, yield scheduler, recovery, retry policy, compaction, and transactional message editing with durable operation receipts; see `docs/message-editing.md`).
  - `transcript/` — the TranscriptLog mutation journal, snapshot versions, editable-user selection, retained-prefix preparation, suffix replacement, and patch application.
  - `store/` — the persistence contract's helpers (the completion-message id and which completions a checkpoint carries) and the media blob contract (externalize/rehydrate); realizations live with the products (`@demicodes/backend`) and in `testing.ts`.
  - `node/` — the session-tree node: the one assembly that builds a root or a subagent (session from the store or fresh, supervisor, command tree, tools), the node's per-Host shell environments with handle ownership checks, and the lifecycle policy applied on restore.
  - `protocol/` — frame types, the inbound-frame zod schemas (`ClientFrame`'s single source of truth), and the transports (`stdio-transport.ts` backs the `./stdio` entry).
  - `server/` — the server facade, transport binding (frame dispatch, ingress validation), the root node's transport view (the frame sink), ownership registry, and frame-view mappers.
  - `subagent/` — the relationship module: the supervisor (spawn, resume, abort, messages, the close policy and completion delivery over the store), the root-session agent directory, tree formatting, and the declarative `demi agent` command tree behind the `SubagentCommandOps` seam.
  - `client/` — AgentClient.

### `@demicodes/coding-agent`

- Browser scope: declare `demi browser` commands and help using schemas
  from the `@demicodes/browser-protocol` dependency, and the `browser`
  [user stream](demi-next/native-runtime.md#user-streams) of the live view;
  algorithms remain native. See [Browser commands](demi-next/browser.md#command-contract).
- Status: implemented.
- Production deps: `@demicodes/agent`, `@demicodes/browser-protocol`, `@demicodes/core`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: coding harness, coding prompt, coding commands (the `demi` root: every subcommand is a noun domain group — `file` as `runtime` modules written against the ABI and `todo` as `rpc` built in, product groups like the backend's `host` composed in). A `reference` block reaches the model as its path; the model reads the file with tools.
- Public boundary: harness and coding command construction based on Host and Command contracts.
- Must not: instantiate AgentSession, AgentServer, a shell environment, concrete providers, or a Host implementation.
- Runtime rule: defines Host, commands, prompt, preamble, lifecycle, and reference resolution through the harness; it must not replace the shell mechanism, the standard agent tool surface, or provide an alternate shell/tool runtime.

- Native binding: the file command tree declares package and operation ids.
  The backend supplies exact release descriptors at runtime; command declarations
  do not import a compiled-in release catalog. The Rust command implementation
  remains in `crates/demi-commands`.

### `@demicodes/provider-claude-code`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Claude Code provider transport, JSONL/MCP mapping (including preservation of model-emitted parallel tool batches across the sequential SDK-MCP callback channel; see `docs/tool-call-concurrency.md`), model catalog mapping, provider event mapping, OAuth usage quota probe (`/api/oauth/usage`), active OAuth resolution injected into the CLI env at spawn, device-config isolation for injected-spawn runs (`CLAUDE_CONFIG_DIR` pinned inside the workspace artifacts dir — a managed device's CLI consumes zero device-local settings), provider glue over the shared credential pool (`@demicodes/provider/credentials-pool`; see `docs/provider-global-credentials.md`), and provider-specific tests.
- Public boundary: `createClaudeCodeProvider`, model catalog function, quota helpers, and public option types from root.
- Secret boundary: OAuth tokens and pool secret files stay inside the provider creator/auth resolver and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: CLI, JSONL, output, transport, parser, credential pool IO, and test cache helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-codex`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Codex auth reuse, Responses transport, model catalog mapping, provider event mapping, reading its failure records (the usage limit's `resets_at` and `resets_in_seconds`), rate-limit quota probe (`x-codex-*` headers), provider glue over the shared credential pool (`@demicodes/provider/credentials-pool`; see `docs/provider-global-credentials.md`), and provider-specific tests.
- Public boundary: `createCodexProvider`, auth status helper, model catalog function, quota helpers, transport mode type, and public option types from root.
- Secret boundary: auth.json material and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Responses builders, SSE/WebSocket transports, stream parsers, credential pool IO, and test cache helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-openai-api`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: official OpenAI Responses API request mapping, explicit Chat Completions wire option for OpenAI-compatible endpoints, SSE event mapping including observed compatible reasoning delta extensions such as `choices[].delta.reasoning_content`, opt-in Chat Completions replay of thinking as `reasoning_content` (`request.passBackReasoningContent`, required for DeepSeek-style thinking + tool loops), official OpenAI API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Codex-visible defaults unless caller-supplied models replace it, and provider-specific tests.
- Public boundary: `createOpenAIApiProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.openai.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `OPENAI`. `wireApi` defaults to `responses`; compatible endpoints can pass `wireApi: 'chat-completions'`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Responses body builders, Chat Completions body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-anthropic-api`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Anthropic Messages API request mapping, event-stream mapping, official Anthropic API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Claude Code defaults unless caller-supplied models replace it, and provider-specific tests.
- Public boundary: `createAnthropicApiProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.anthropic.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `ANTHROPIC`. `baseUrl` must already include the API version prefix (typically `/v1`); the provider only appends `/messages` (or leaves the URL alone when it already ends with `/messages`). Claude Code / Kimi-style roots such as `https://api.kimi.com/coding/` are not drop-in values — pass `…/coding/v1` instead.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Messages body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-grok-build`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Grok Build CLI OAuth session reuse (`~/.grok/auth.json`), native RFC 8628 device login against auth.x.ai using the official frozen OAuth2 scopes, OIDC token refresh, cli-chat-proxy Chat Completions transport, model catalog mapping from `/v1/models`, billing/subscription quota probe (`/v1/billing?format=credits`, `/v1/user?include=subscription`), demi credential pool for global multi-credential switch, provider event mapping, and provider-specific tests.
- Public boundary: `createGrokBuildProvider`, auth status helper, model catalog function, quota helpers, and public option types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `https://cli-chat-proxy.grok.com/v1`. Auth is the Grok CLI OAuth session or native device login against `https://auth.x.ai` (no API-key product path).
- Secret boundary: session tokens, refresh tokens, raw auth file contents, and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Chat Completions builders, SSE readers, stream mappers, runtime classes, credential pool IO, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-google`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Google Gemini `generateContent` API request mapping (native wire, not OpenAI-compatible), SSE event mapping including thought summaries / thought signatures / thinking token counts, tool-returned media as inline parts (including video), official Gemini API defaults, endpoint/env/api-key resolution, model metadata mapping, and provider-specific tests.
- Public boundary: `createGoogleProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://generativelanguage.googleapis.com/v1beta`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `GOOGLE`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: generateContent body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/backend`

- Lifecycle scope: `lifecycle/` owns the conversation idle clock, Cloud idle
  scheduling and the conversation release, reusing `ActivityGate`; it does not
  import the domain modules that supply activity. See
  [Conversation idle and Host resource release](demi-next/resource-lifecycle.md).
- Browser scope: the backend has no browser module. It names the conversation
  and invoking node in the command context of every job it starts and sends
  the generic conversation release; browser state lives in the native package. The live browser view
  reaches the Host through the generic user stream route and activity reports
  ([Web API](demi-next/web-api.md#user-streams)).
- Status: target contract.
- Production deps: `@demicodes/agent`, `@demicodes/browser-protocol`, `@demicodes/coding-agent`, `@demicodes/command-loader`, `@demicodes/command-protocol`, `@demicodes/core`, `@demicodes/host-remote`, `@demicodes/machines`, `@demicodes/provider` and the concrete providers, `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`; external: `hono` (HTTP framework, Bun runtime).
- Owns: the hosted multi-user product's server — the storage module (SQLite layer, numbered control/conversation migrations, `ControlService` over `control.sqlite`, the per-conversation `AgentTreeStore` over node and block rows, blob store, DB-backed `HostStore`), the Web API (Hono routes + the per-conversation frame-protocol WebSocket with server-side session/cwd scoping and media by reference on the way out), AgentServer assembly with the shell environment chosen per Host, runner management (pairing, device registry, one live socket per device, the rpc relay, the pipe routes over `host-remote`'s broker, the user stream route bridging a page's WebSocket to a service stream's pipes with backpressure, browse endpoints), the managed-hosts module (one managed device per user over the `ManagedHostProvisioner` contract, reached through `@demicodes/machines`' `RemoteProvisioner` as the machine manager's client; lifecycle/hibernate/reset, the backend-contributed `demi host` subcommand group), the expose module (`expose/`: expose records and their one-hour lifetime, the public relay answering expose hostnames over device access and the network stream, the `demi host expose` leaves; see `docs/demi-next/expose.md`), the LLM module (per-provider provider assembly, model metadata caching in memory and control.sqlite with TTL and shared refresh, metering wrap), the credential vault (instance secret, GCM-encrypted providers, subscription device-login flows over per-provider provider pools), and usage accounting (ledger + rate limit). The backend accepts explicitly submitted API keys and setup tokens at authenticated write boundaries, passes subscription material to provider-owned credential pools, and never returns secrets or proxies model traffic.
- Public boundary: `createBackend`, storage module types from root; the `demi-backend` bin.
- May assemble: concrete providers, AgentServer, `RemoteHost`, `RemoteShellEnvironment` and the coding harness.
- Builds the JS command manifest with Bun's transpiler, serves it to runners and executes authenticated RPC handlers. All shell scripts execute on the selected machine.
- Must not: be imported by any other production package; put business logic in the HTTP layer beyond routing/validation; let providers or credentials cross to runners or browsers.
- Layout (directories mirror the design record's backend modules):
  - `backend.ts` — the composition root (wire and mount only).
  - `testing.ts` — test-only real-runner shell fixtures for agent and command tests; exported through `@demicodes/backend/testing`, never imported by production code.
  - `http/` — the external HTTP surface: app assembly, the session gate over `/api/*` with its exemptions, the cookie helpers, one route module per resource (setup, auth, transfers and blobs included), the WS upgrade adapter.
  - `auth/` — identity: the roles and the authenticated user shape, password hashing, the cookie sessions over the control plane, the login lockout, and email-change verification through an injected mail sender.
  - `settings/` — validated per-user appearance and shortcut overrides, merged field by field in control storage.
  - `sync/` — reconstructible user-scoped product snapshots for conditional HTTP polling; chat stays on the agent protocol.
  - `conversation/` — frame scoping/rewrite, attachment and remote-file references, file transfers for previews and downloads (ranges, headers, their end on idle, archive or target change), partial metadata updates, archive admission, read/activity summaries, persistent Fork creation and publication, Cloud/device/workspace target resolution, target/file admission and per-node execution context. The root and subagents resolve the conversation selection per action.
  - `storage/` — the SQLite layer (database seam, migrations, control service, the conversation stores with the `AgentTreeStore` realization, blob store, host store).
  - `runner/` — runner management: pairing-code/device-token primitives, the registry (pending claims, one live socket per device, stable per-target `RemoteHost`s, liveness, the rpc relay), the relayed call's pipes on its `io`, installation script generation and the shared `demi host` command group. `http/runner-install.ts` serves the generated script and release artifacts; `runner/installer.ts` defines the installation steps.
  - `llm/` — the provider runtime assembled per provider entry (the family registry with each family's credential kind, the vendor catalog over models.dev, the model catalog, the Test button) and the metering wrap at the inference entry.
  - `vault/` — instance secret, credential crypto, the typed provider vault over the control plane, and the provider scope (whose providers a caller works with under the instance mode).
  - `usage/` — enforcement (the provider-request rate limiter); the ledger rows live on the `ControlService`.
  - `lifecycle/` — the conversation idle clock (one window for Cloud stop and conversation release), retirement admission and shutdown. Domain modules supply activity facts; there is no domain-specific idle sweep.
  - `managed/` — one managed device per user, Cloud policy and its lifecycle adapter, allocation/wake, paired system/home checkpointing, volume growth, external system reset, Cloud project directory creation. Every VM and disk operation goes to the `ManagedHostProvisioner` it is given; the backend never spawns a hypervisor or an image tool itself.
  - New modules get sibling directories — never new files at the root.

- Execution coordination: authenticated live-job RPC routing, conversation target/file admission and user-device lifecycle admission. Managed operations persist allocation/reset intent and recover it before new work; every conversation using Cloud shares its device-wide operation gate.

### `@demicodes/machines`

- Status: implemented (`docs/demi-next/managed-hosts.md` § Provisioning).
- Production deps: `@demicodes/utils`; external: `zod`.
- Owns: the machine manager — the `ManagedHostProvisioner` contract (reconcile, base version, image state, wake, hibernate, checkpoint, volume growth, reset, close, death events) and the `MachineImageState` schema; the machine wire (`protocol.ts`: the `machineOps` table from which the request union and the typed replies derive, newline-delimited JSON over a Unix socket); `serveMachines` (the service end: one socket, any number of backend connections, every request dispatched to one provisioner, deaths broadcast); `RemoteProvisioner` (the backend's end: a provisioner whose calls travel over the socket, reconnecting on the next call after a drop); the Firecracker implementation (`firecracker/`: the image tools over e2fsprogs, the VM process in its two launch modes, the Firecracker API over its socket, the tap slots, the per-VM kernel command line) and the machine-image store (paired system/home generations, pinned bases); the `demi-machines` bin (`main.ts`: `DEMI_MACHINES_SOCKET`, `DEMI_MACHINES_DATA`, `DEMI_MANAGED_*`).
- Public boundary: the provisioner types and `imageStateSchema`, the protocol schemas, `RemoteProvisioner`, `serveMachines`, `FirecrackerProvisioner`, `firecrackerConfigFromEnv` from root; the `demi-machines` bin.
- Deployment files: `scripts/install-managed-hosts.sh` (taps, forwarding, egress rules), `scripts/firecracker-jailer.sh` (the privileged jailer helper the manager invokes through `sudo -n`), `scripts/lima-machines.sh` and `lima/demi-machines.yaml` (the manager inside a Lima instance with nested virtualization, for macOS).
- Spawning `firecracker`, the jailer and e2fsprogs is this package's transport — the intentional external-process exception.
- Must not: listen on TCP (the socket file's permissions are the boundary; there is no authentication); know users, conversations or the control database (it receives device ids and boot arguments); import the backend.

### `@demicodes/command-loader`

- Status: accepted native manifest contract; acceptance is tracked separately.
- Production deps: `@demicodes/command-protocol`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: strict manifest schemas, canonical manifest identities, package catalog validation, declaration serialization, manifest sources, and TypeScript dispatch for embedders through injected RPC/native executors. Manifests pin complete package descriptors and exact operation bindings; they contain no implementation source or artifact location.
- Public boundary: `buildManifest`, `parseManifest`, manifest types, `createLoader`, manifest sources, `inProcessRpc`, RPC transport types and `treeFromManifest`.
- Must not: know the backend, spawn processes, resolve object-store credentials, hold command definitions, transpile source or load downloaded code. The native runner consumes generated Rust manifest values and implements its CLI dispatch in Rust.

### `@demicodes/runner-protocol`

- Conversation scope: jobs and service streams carry the
  [command context](demi-next/native-runtime.md#command-context);
  `conversation_release` is the one generic release message; no browser policy.
- Status: implemented (the final wire: MessagePack frames, per-op fs messages, jobs, the rpc relay, the manifest push, transfers).
- Production deps: `@demicodes/command-protocol`, `@demicodes/shell` (the Host types the fs messages carry), `@demicodes/utils`, `@msgpack/msgpack` (the Bun end's codec).
- Owns: the authoritative Zod backend runner wire contract — the message schemas (claim/auth handshake, liveness, the `fsOps` table from which the per-op fs requests and typed replies derive (file contents name a pipe and never ride a message, `docs/demi-next/runner.md` § File contents), streaming spawn, jobs, the rpc relay, the manifest push, transfers, the network stream `net_open` and its replies), `createRunnerWire(codec)` (encode, and decode-with-validation per direction over an injected MessagePack codec: `msgpackCodec` under `@demicodes/runner-protocol/msgpack` used by Bun; Rust uses the generated contract and rmp-serde), the protocol constants (`RUNNER_PROTOCOL_VERSION`, `JOB_VIEW_BYTES`, the message size limit both ends enforce).
- Public boundary: message types and schemas, `createRunnerWire`, the constants from root; `msgpackCodec` under `msgpack`; six-target runner release schemas under `release`. The backend consumes this package directly; the Rust runner generates bindings from its Zod schemas. It depends on neither endpoint.
- Must not: contain network IO, a Host implementation, a shell environment, the job table, credentials, claim policy, device registry, or conversation state.

### `@demicodes/host-remote`

- Conversation scope: pass the command context with each job and forward the
  conversation release; no browser knowledge.
- Status: implemented (M9).
- Production deps: `@demicodes/command-loader`, `@demicodes/command-protocol`, `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: the backend's end of a runner — the pipe broker (`pipes.ts`: the rendezvous of a pipe's two ends, which the backend's `/api/pipes` routes feed, holding only the bytes in flight; `devicePipes` binds it to one device) and `RemoteHost`, a `Host` over a connection whose file contents travel through that device's pipes, with a jobs facet (stable object across reconnects, logical cwd fallback, injected store), a working-tree facet and a network facet (`net.open`: one TCP stream on the device as two pipes, `runner.md` § Network streams), and `RemoteShellEnvironment`, the `ShellEnvironment` of a real host over jobs (the model's view as the record, the working directory carried between execs). The production Host and shell the backend injects into the agent.
- Public boundary: `RemoteHost`, `RemoteShellEnvironment`, `createRemoteShellEnvironmentFactory` and their option types from root. The factory validates a package catalog and serializes each context's declarations. Each job pins its manifest; artifact location requests are admitted only for a matching active job, manifest and target artifact. The injected resolver supplies a location, and job completion cancels outstanding resolutions.
- Must not: contain network IO (the wire is an injected send/handle pair, and the backend's HTTP routes hand the broker its device ends), credentials, the device registry, or conversation state. `Host.store` never crosses the wire.

- Test boundary: `@demicodes/host-remote/testing` owns native process fixtures,
  the test WebSocket bridge, host conformance adapters and host-only native
  package catalogs. These are excluded from its platform-neutral root.

### `packages/guest-image` (not a workspace package)

- Owns: the guest image pipeline (`docs/demi-next/managed-hosts.md` § Images): the kernel build (Linux 6.1 on Firecracker's microvm config plus `kernel/extra.config`), the rootfs build (Ubuntu by debootstrap, the toolchain list, the guest user with sudo, the runner as `/demi-runner` with a command alias at `/usr/bin/demi`, `mke2fs -d`), and the runner packing for Linux musl. Shell scripts and a kernel config; runs on Linux with root at build time, never at backend runtime. Its outputs (`vmlinux`, `rootfs.ext4`) are release artifacts the backend is pointed at.

### `@demicodes/browser-protocol`

- Status: implemented schemas and generated native bindings.
- Production deps: no first-party packages; external: Zod.
- Owns: browser operation arguments/results, tab state, observations,
  resource event payloads, and the live view protocol: its messages and frame
  header ([Live browser view](demi-next/browser-live-view.md)).
  Generic resource envelopes remain in command-protocol/runner-protocol.
- Public boundary: platform-neutral schemas and derived types. `coding-agent`
  uses them to declare CLI commands; backend validates browser data;
  demi-commands generates native bindings at build time.
- Independence rationale: native commands, the coding harness, and backend need one
  browser contract without importing each other's implementations.
- Must not: implement browser operations, transport, components, Host access,
  process management, or conversation persistence.

### `@demicodes/command-protocol`

- Retained-resource scope: authoritative native resource lifecycle and
  scoped invocation/event wire schemas, following
  [Native runtime](demi-next/native-runtime.md#retained-resources).
- Owns: authoritative Zod command-service wire and native package descriptor
  schemas, including the [command context](demi-next/native-runtime.md#command-context), derived TypeScript types, protocol constants and package identities.
  Package identities use canonical JSON and SHA-256.
- Production deps: none.
- Public boundary: command wire and package schemas, types and constants.
- Must not: implement command algorithms, perform transport IO, spawn services
  or hold backend state. Rust bindings belong to `crates/command-service`.

### `crates/command-service` (Rust library)

- Retained-resource scope: generated lifecycle types, scoped dispatch,
  cancellation and bounded event transport; no tab, cookie or input policy.
- Owns: generated command wire/package types and validation, incremental framing,
  HTTP/2 client and server, bounded invocation IO and handler cancellation, and
  the shared invocation edit recorder (`edits`): bounded file snapshots and a
  schema-validated journal coordinated across processes by an OS file lock.
- Independent boundary: the runner and independently distributed command programs
  use the same communication SDK. Third-party command authors depend on this
  library without depending on runner or Demi command implementations.
- `build.rs` consumes the Zod definitions in `packages/command-protocol`.
  `src/integrity.rs` owns streaming artifact verification reused by installers;
  `src/protocol.rs` includes generated types and owns framing and the
  `Metadata` an invocation stream opens with: a native `Invocation`, or the
  local command client's `LocalInvocation`; `client.rs` and `server.rs` own the
  two transport roles. `src/descriptors.rs` owns waiting
  out a lack of open files, for the runner, the edit recorder and command
  programs alike (`docs/demi-next/runner.md` § Load).
- Depends on: Tokio, tokio-util, h2, http, futures-util, bytes, serde, serde_json,
  thiserror, package-identity hashing/canonicalization libraries, and libc or
  windows-sys for the system's error codes.
- Public boundary: protocol types, client, service entry point, handler and IO,
  and waiting out a lack of open files.
- Must not: implement commands, download artifacts, start command processes,
  hold credentials or change process-global cwd/environment for an invocation.

### `crates/demi-commands` (Rust executable)

- Browser scope: `browser/` owns browser/driver integration, the canonical
  tab registry, observations, command operations, the live view module with
  its capture extension and page observers
  ([Live browser view](demi-next/browser-live-view.md)), and resource
  cleanup. Browser business schemas come from `browser-protocol` and are generated
  by this crate's build for native consumers; no second Rust schema authority.
  The command catalog and its ownership follow the
  [browser implementation status](demi-next/browser.md#implementation-status).
- Owns: the independently released `demi-commands` resident program and all native
  Demi command implementations, including file read/create/edit/patch.
  `coding-agent` owns the TypeScript declarations; backend supplies the runtime
  release catalog. `scripts/native/release-package.ts` packages six target binaries.
- Uses command-service for transport and invocation context. File operations use
  invocation-local cwd and cancellation. Mutations serialize planning and
  application; each replacement publishes atomically, and multi-file patches
  roll back earlier changes if a later write fails.
- First-party production dependency: command-service only.
- Must not: host a runner connection, define the agent's command tree, store
  conversations or be linked into runner. Standard shell utilities belong to
  runner's shell module.

### `crates/runner` (Rust executable)

- Conversation scope: `commands/` keeps each job's command context and writes
  it into every native invocation, keeps a service resident while it holds
  conversation state, and forwards the conversation release; it implements no
  browser operation.
- Owns: the `demi-runner` execution host, backend registration and connection,
  filesystem/process RPC with file contents through pipes, network streams (`net_open`: a TCP socket on the
  device carried as two pipes, no protocol parsing), service streams
  (`service_open`: a user stream's invocation carried as two pipes, no protocol parsing), shell jobs, local command
  forwarding and installation.
- `build.rs` consumes the Zod runner-message and manifest definitions from
  `packages/runner-protocol` and `packages/command-loader`. Generated bindings
  remain internal to the consuming runner modules.
- `src/connection/` owns the backend connection and runner-message codec.
- `src/commands/` owns manifest validation, CLI parsing/help, dispatch, application
  callbacks, artifact acquisition/cache and resident command-process lifetimes.
  It verifies exact artifacts and reuses services by digest. Backend supplies
  artifact locations; storage-vendor selection remains outside runner.
- `src/shell/` owns brush execution and standard utility registration/adaptation.
  Brush, subshells and utility builtins execute inside the resident runner.
  Each job owns cwd, environment and IO. Declared builtins call the dispatcher
  directly; external programs call it through the forwarding executable.
- Command definitions and package catalogs are fixed during the host program's
  lifetime. Context disposal releases bindings and service references.
  Cancellation preserves the runner and unrelated jobs; see
  [runner.md](demi-next/runner.md) for lifecycle behavior.
- Local forwarding uses an owner-restricted Unix socket or Windows named pipe.
  Guest initialization owns Linux PID 1 boot and volume operations.
- First-party production dependency: command-service only. Vendored brush and
  utility libraries are external dependencies, not additional Demi workspace crates.
- Public boundary: the executable. Native build/release scripts produce six
  target artifacts. TypeScript integration fixtures belong to host-remote/testing.
- Must not: own conversations or model/provider implementations, execute
  downloaded JavaScript, or link Demi command algorithms into the runner.

### `@demicodes/web-ui`

- Status: implemented; published to npm as a source-form package (no build step — `.vue`/`.ts`
  source exports compiled by the consumer's bundler, which must handle Vue SFC + TypeScript).
- Production deps: `@demicodes/core`, `@demicodes/agent`, `@demicodes/browser-protocol`, `@demicodes/utils`, `zod`.
- Owns: the reusable browser component library (Vue) — the agent Tab, List (+ blocks), and
  Input surfaces, the assembled ChatSession page, the message editor and its
  draft/submission lifecycle (`agent/message-editor/`: the tiptap editor a user message is written and shown in, whose document is the message — each file a node carrying what its capsule shows and what the message sends, while the bytes in flight stay with the host under that node's id; `markdown/user-markdown.ts`: the user dialect, read and written; `agent/message-editing.ts`, `SessionComposer.vue`, and the inert `MessageEditRegion.vue`), sidebar layout, workspace and
  remote-file selection flows, the live browser view (video, input, native control overlays and the viewport menu over an injected stream source; `docs/demi-next/browser-live-view.md`), file previews (`files/`: the viewer for each kind, the side-by-side change comparison, releasing a transfer when its preview hides; `markdown/document.ts`: a Markdown file rendered and sanitized as a document; `docs/demi-next/file-previews.md`), shared UI primitives, markdown/theme, shared sidebar presentation and list interaction, the sign-in page (`auth/EmailLoginPage`: email and password on the left, a wide empty intro on the right, over a host-reported phase), the settings surface (`settings/`: dialog shell and panels as presentation over host-mapped models), the reusable device pairing dialog and lifecycle (`devices/`, driven by a host-provided claim adapter), and the
  model-catalog DTOs the composer reads (`transport/protocol.ts`: `ProviderInfo`, `ModelInfo`
  and friends, which the host fills from its own catalog, plus the agent's frame, block and
  tool-view schemas re-exported for the host).
  Consumes an injected `AgentClient`; the library ships no control-plane transport of its own.
- Public boundary: source-path exports (`./*`) consumed by web hosts; third parties embed it
  by supplying an `AgentClient` and the model-catalog DTOs. External products consume the
  published package (registry semver), not `link:` paths into this repo.
- Must not: import Node, `@demicodes/shell`, `@demicodes/coding-agent`, concrete providers, or
  `@demicodes/web` or `@demicodes/web-gallery`. It may import the `@demicodes/agent` client surface only (`AgentClient`,
  WebSocket client transport, frame/event/block types and their schemas).
- Enforcement: because the components are `.vue` (not scanned by the `.ts` boundary test),
  the web-ui boundary is enforced at the package-manifest level (no Node/adapter/provider
  dependencies declared), not by the production import-graph scan.

### `@demicodes/web`

- Status: backend-integrated web product.
- Production deps: `@demicodes/web-ui`, `@demicodes/core`, `@demicodes/utils`, `pinia`, `zod`.
- Owns: the Vue SPA application frame, route navigation, product state and backend request handlers. Vue 3 + TypeScript + Vite, vue-router, Pinia and Tailwind 4.
- Public boundary: `bun run web:dev` and `bun run web:build`; no published library API.
- Build order: the root `bun run build` finishes library builds before invoking
  `web:build`. Vite consumes `web-ui` source and the libraries' published entry
  points, including `@demicodes/agent/client`; those library artifacts must exist
  before the product bundle starts.
- Layout: `main.ts` is the only composition root (app, router, account-scoped stores); `App.vue` is the application frame; `conversation/` owns chat state and
  containers; `targets/` owns environment selection; `settings/` owns settings
  containers; `auth/` owns cookie-session state and entry containers; `api/` owns
  validated browser HTTP/agent wire contracts and upload requests; `state/` owns
  server snapshots, preferences and per-user local state; `devices/` owns pairing
  and filesystem adapters. Reusable UI belongs to `web-ui`.
- Integration boundary: authentication calls the backend over same-origin HTTP.
  Chat uses the agent client over WebSocket; resource and settings operations use
  REST. The browser does not import backend code or concrete providers.
  See `docs/demi-next/web-application.md` for state ownership and operation contracts.
- Must not: import `web-gallery`, Node, Host implementations or concrete providers.

### `@demicodes/web-gallery`

- Status: implemented.
- Production deps: `@demicodes/web-ui`, `@demicodes/browser-protocol`, `@demicodes/core`, `@demicodes/utils`, `zod`.
- Owns: the Vite-only component catalog for `@demicodes/web-ui`. It remaps `web-ui` tokens so paradigms
  (tone, accent, density, radius, shadow, light/dark) can be compared against the catalog.
  Pages are vue-router paths; Markdown is a full-pane message route. Roadmap
  lists shipped and next library work. It is not a product surface and does not
  ship multiple themes into runtime.
- Public boundary: local `bun run web:gallery` entry (`packages/web-gallery`).
- Must not: import Node, `@demicodes/shell`, `@demicodes/coding-agent`,
  concrete providers, `@demicodes/web`. Must not be imported by any
  other production package.

## Production Dependency Graph

The production dependency graph contains every TypeScript workspace package and
must stay acyclic:

```text
core -> none
utils -> none
provider -> core, utils
shell -> command-protocol, utils
agent -> core, provider, shell, utils
coding-agent -> agent, browser-protocol, core, shell, utils
provider-claude-code -> core, provider, utils
provider-codex -> core, provider, utils
provider-openai-api -> core, provider, utils
provider-anthropic-api -> core, provider, utils
provider-grok-build -> core, provider, utils
provider-google -> core, provider, utils
browser-protocol -> none
command-protocol -> none
command-loader -> command-protocol, shell, utils
runner-protocol -> command-protocol, shell, utils
machines -> utils
host-remote -> command-loader, command-protocol, runner-protocol, shell, utils
backend -> agent, browser-protocol, coding-agent, command-loader, command-protocol, core, host-remote, machines, provider, provider-anthropic-api, provider-claude-code, provider-codex, provider-google, provider-grok-build, provider-openai-api, runner-protocol, shell, utils
web-ui -> agent, browser-protocol, core, utils
web-gallery -> web-ui, browser-protocol, core, utils
web -> web-ui, core, utils
```

The browser contract supplies shared schemas to coding-agent and backend. Native
binding generation consumes browser-protocol as a build input and adds no
first-party Rust runtime dependency.

First-party Rust production dependencies are:

```text
runner -> command-service
demi-commands -> command-service
command-service -> none
```

Zod source consumption during Cargo builds is a build dependency on the owning
TypeScript definitions, not a Rust runtime dependency or a mirrored crate.
Cargo manifests must match this graph; external libraries and vendored dependencies
are outside it.

`web-ui`, `web-gallery` and `web` are browser/product packages built with Vite/Vue; their internal source
is `.vue` + `.ts`. The `.ts`-only `platform-entrypoints` boundary test does not scan them as
production source. `web-ui`'s outward boundary (no Node/adapter/provider dependencies) is
enforced at the manifest level by that test..

The graph is a compact view of the `Production deps` fields in the package registry; keep the
two in lockstep. `packages/core/src/__tests__/platform-entrypoints.test.ts` reads this block
rather than a copy of it: every workspace package must have a line, each package manifest's
`@demicodes/*` dependencies must equal its line, production source imports must stay within it,
and it must be acyclic. The same test reads the workspace: each package's `exports` names the
source file of every entry under a `development` condition, the root `tsconfig.json` `paths`
mirror every subpath entry (roots resolve through the `@demicodes/*` wildcard, so a root entry
is always `src/index.ts`), and the root `test` script names every package that has tests.

## Module Layout Conventions

How files and directories are organized inside a package. These are design
rules, enforceable in review — not taste:

Rust crates keep `Cargo.toml` at the crate root, Rust source under `src/` and
Cargo's standard library/executable entrypoints. TypeScript source stays in its
own package or repository build tooling. Each consuming `build.rs` generates
Rust types and validation from authoritative Zod schemas into `OUT_DIR`, included
with `include!`. A normal Cargo build runs generation without a manual preparation
step. Cross-target build/release orchestration belongs in `scripts/native/`.
See [native-runtime.md](demi-next/native-runtime.md#contract-generation-and-validation)
for validation requirements. Cross-language integration fixtures belong to the
TypeScript adapter exercising the native executable.

Third-party source trees belong in the repository root's `vendor/<crate>/`,
with their upstream metadata and licenses retained. The root `Cargo.toml`
declares their `[patch.crates-io]` paths and excludes them from workspace
membership. Demi adapters stay in their responsible `crates/` module. Dependency
versions and source identifiers stay in manifests, lockfiles and vendor metadata.
`docs/` describes current architecture and usage, not dependency inventories,
artifact hashes, CI run logs or one-off acceptance records.

1. **One composition root per product package.** Exactly one file assembles
   the package (`backend.ts`, `main.ts`): it may construct, inject, mount,
   and return — never branch on business state. Any domain logic appearing
   in a composition root is a violation.
2. **Directories mirror design modules; files carry one responsibility.**
   A subdirectory must be nameable as a module of the owning package's
   design record entry (e.g. the backend's conversation/LLM/runner/vault/
   usage/auth modules). No domain-less catch-all directories (`misc/`,
   `helpers/`) — generic code goes to `@demicodes/utils`, domain helpers
   sit next to their module.
3. **Split by responsibility, not by line count.** A file that carries two
   of {route handling, domain logic, storage access, wire adaptation} gets
   split, regardless of size; a long file with one responsibility may stay.

Packages small enough to be a single module (e.g.
`host-remote`, `runner-protocol`) need no subdirectories; the registry
entry's Layout section appears only where a package has more than one
module.

## Global Boundary Rules

- Platform-neutral package roots must not statically pull Node-only adapters, concrete providers, UI code, or test helpers into their import closure.
- Public roots expose stable package contracts only; internal parser, transport, protocol, local adapter, auth-store, stream, and test helpers stay behind implementation files unless a package registry entry explicitly says otherwise.
- Any workspace package imported by production source must be declared in `dependencies`, not hidden in `devDependencies` or transitive packages.
- Runtime-specific code (Node or native Rust) lives behind an explicit platform entry or crate, never in a platform-neutral root.
- Do not keep compatibility shims when a package split moves an implementation to its final package.

## Verification

Existing boundary coverage:

- `packages/core/src/__tests__/platform-entrypoints.test.ts` checks platform-neutral root entries for Node-only static closure leaks.
- The same test checks that only AgentServer imports AgentSession as a runtime value outside tests.
- The same test checks that `@demicodes/shell` does not depend on the agent runtime.
- The same test checks selected package manifest layering boundaries.
- The same test scans `@demicodes/core` and `@demicodes/provider` production source for concrete provider names, concrete catalog source labels, backend identifiers, and product-specific source identifiers.
- Repository-wide production scans enumerate the existing production package directory table. The models.dev client's endpoint and diagnostics belong only in `provider/src/models-dev.ts`; shared model/catalog metadata still cannot expose concrete source labels. Quota cache metadata follows its separate domain contract.
- The same test builds the production source package graph and fails on cycles or edges outside the enforced graph.
- The same test checks that production workspace imports are declared in package `dependencies`.
- The same test checks public provider root exports so internal transport, parser, protocol, auth-store, stream, and testing helpers do not leak through by accident.
