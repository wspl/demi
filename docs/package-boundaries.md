# Package Boundaries

This document is the canonical package boundary contract and the highest architecture constraint for package work. When code and this document disagree, fix the code or update this document before continuing with feature work.

The Cloud/execution entries specify the target architecture in `demi-next/`.
Documentation acceptance and runtime acceptance are separate: the dependency graph
is the required final package set, not a claim that all workspace manifests have
already been brought into agreement. Implementation checkpoints must reconcile
source, manifests and boundary checks with this contract.

Data ingress, schema ownership, decoding and validation follow
[Data Contracts](data-contracts.md).

## Dependency Direction

Package direction is a core architecture invariant. Lower-level packages must not know higher-level products, adapters, UI shells, concrete providers, or local machine implementations.

The package registry below is the single source of truth for per-package responsibilities and boundaries. Do not scatter package-specific rules across separate sections. When a package is added, removed, renamed, or split, update its registry entry and the dependency graph together.

Test code may depend upward for integration coverage. Production code must not.

## Package Registry

### `@demicodes/core`

- Status: implemented.
- Production deps: none.
- Owns: shared data types only: transcript blocks, content blocks, model selection, thinking config, usage, session phase, queued messages, and accepted user steers awaiting transcript insertion.
- Public boundary: type/data contracts shared across packages.
- Must not: contain concrete provider names, catalog source names, shell runtime details, Host details, UI concepts, transport URLs, or backend identifiers.

### `@demicodes/utils`

- Status: implemented.
- Production deps: none.
- Owns: generic, platform-neutral helper functions only — type guards, error/abort helpers, async primitives, byte/UTF-8/base64 helpers, string helpers, the portable JSON codec (`Uint8Array`/`bigint` round-trip used by agent transports and HostStore implementations), and id generation.
- Public boundary: pure utility functions shared across packages; no domain types or runtime services.
- Must not: contain domain logic; transcript, provider, shell, or agent types; Node-only adapters; or any package-specific behavior.

### `@demicodes/provider`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/utils`.
- Owns: abstract provider contract, inference request items, provider events, public provider shell, hidden provider runtime factory helper, auth/runtime status, required `AgentProvider.clone()` for independent per-session runtimes, unified subscription/rate-limit quota types (`ProviderQuota` / `ProviderQuotaSnapshot`; see `docs/provider-quota.md`), optional multi-credential types (`ProviderCredentials` / `ProviderCredentialInfo` — global active switch, not multi-instance providers; see `docs/provider-global-credentials.md`), the shared node-only credential pool IO behind the `@demicodes/provider/credentials-pool` subpath (the main entry stays platform-neutral), model catalog shape, configured model schema/types and projection (`configured-models.ts`), and the models.dev catalog client (`models-dev.ts`: the fetch with its cache and stale fallback, the zod schema of the parts read, the entry-to-catalog-model mapping) that concrete providers and the backend filter for their own lists.
- Public boundary: provider contract, direct `Provider[]` composition types, quota helpers (`createProviderQuota`, `ensureQuota`) and shared quota amount/reset schemas, credential public types, schema parsing with payload-free diagnostics (`parseProviderData`, `parseProviderJson`, `parseProviderJwt`, `ProviderDataError`), shared SSE framing (`readServerSentEvents`, `ServerSentEvent`) and the named Responses/Chat wire contracts, provider test helpers only from `@demicodes/provider/testing`, and pool IO only from `@demicodes/provider/credentials-pool`. Concrete providers own their unique wire schemas. Shared Responses schemas/usage (`responses-wire.ts`) and content projection (`responses.ts`) serve Codex and OpenAI; shared Chat Completions schemas and stream mapping (`chat-completions-wire.ts`, `chat-completions.ts`) serve OpenAI-compatible and Grok endpoints. These modules have no endpoint, auth or concrete-provider dependency. The shared parser owns JSON/JWT payload decoding and validation error classification. JWT payload decoding extracts metadata without verifying signatures.
- Model catalog boundary: common catalog state exposes portable fields only: model ids, display metadata, capability metadata, service tiers, `sourceFetchedAt`, `stale`, and `warnings`.
- Model catalog must not: expose provider-specific `source` labels such as `codex-backend`, `models.dev`, or `cache` in public types.
- Must not: import concrete providers, the agent runtime, the shell packages, or Host implementations.

### `@demicodes/shell`

- Status: implemented.
- Production deps: `@demicodes/utils`.
- Owns three contracts and nothing that implements them. **The Host contract** (`host.ts`): `defaultCwd`, `identity`, `fs`, `process` (`openCwd`, `spawn`), `store`; `fileHostStore` (a `HostStore` as JSON files on any `HostFileSystem`). **The command system** (`command.ts`, `command-schema.ts`, `command-abi.ts`, `reserved-names.ts`, `shell-quote.ts`): command specs and kinds, the command ABI (`CommandContext`, `CommandResult`, `DispatchIO`, `runtimeModule`, `importCommandModule`), `CommandRegistry` with the one reserved-name table, the atomic `CommandStorage` interface. **The shell-environment contract** (`shell-environment.ts`, `command-records.ts`): `ShellEnvironment` behind the `shell_*` tools, the command record, the model's status view (each stream's delta since the last view, its tail, and the output path when the target keeps one). The production shell is `RemoteShellEnvironment` in `host-remote`, over real runner jobs.
- Entries: the root runs on every runtime (Bun, txiki.js; the runner bundles it). `testing` supplies in-memory Host and command stores and the Host conformance suite (`hostConformanceCases`, run by every Host implementation; including process cases), runtime-neutral so the suite runs on txiki.js. No entry imports Node.
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

- Status: implemented.
- Production deps: `@demicodes/agent`, `@demicodes/core`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: coding harness, coding prompt, coding commands (the `demi` root: every subcommand is a noun domain group — `file` as `runtime` modules written against the ABI and `todo` as `rpc` built in, product groups like the backend's `host` composed in). A `reference` block reaches the model as its path; the model reads the file with tools.
- Public boundary: harness and coding command construction based on Host and Command contracts.
- Must not: instantiate AgentSession, AgentServer, a shell environment, concrete providers, or a Host implementation.
- Runtime rule: defines Host, commands, prompt, preamble, lifecycle, and reference resolution through the harness; it must not replace the shell mechanism, the standard agent tool surface, or provide an alternate shell/tool runtime.

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
- Owns: Codex auth reuse, Responses transport, model catalog mapping, provider event mapping, rate-limit quota probe (`x-codex-*` headers), provider glue over the shared credential pool (`@demicodes/provider/credentials-pool`; see `docs/provider-global-credentials.md`), and provider-specific tests.
- Public boundary: `createCodexProvider`, auth status helper, model catalog function, quota helpers, transport mode type, and public option types from root.
- Secret boundary: auth.json material and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Responses builders, SSE/WebSocket transports, stream parsers, credential pool IO, and test cache helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-openai-api`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: official OpenAI Responses API request mapping, explicit Chat Completions wire option for OpenAI-compatible endpoints, SSE event mapping including observed compatible reasoning delta extensions such as `choices[].delta.reasoning_content`, opt-in Chat Completions replay of thinking as `reasoning_content` (`request.passBackReasoningContent`, required for DeepSeek-style thinking + tool loops), official OpenAI API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Codex-visible defaults unless caller-supplied models replace it, and provider-specific tests.
- Public boundary: `createOpenAIApiProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.openai.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `OPENAI`. `wireApi` defaults to `responses`; compatible endpoints can pass `wireApi: 'chat-completions'`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Responses body builders, Chat Completions body builders, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-anthropic-api`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Anthropic Messages API request mapping, Messages wire schemas (`response-schemas.ts`) and event-stream mapping, official Anthropic API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Claude Code defaults unless caller-supplied models replace it, and provider-specific tests.
- Public boundary: `createAnthropicApiProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.anthropic.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `ANTHROPIC`. `baseUrl` must already include the API version prefix (typically `/v1`); the provider only appends `/messages` (or leaves the URL alone when it already ends with `/messages`). Claude Code / Kimi-style roots such as `https://api.kimi.com/coding/` are not drop-in values — pass `…/coding/v1` instead.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Messages body builders, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-grok-build`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Grok Build CLI OAuth session reuse (`~/.grok/auth.json`), native RFC 8628 device login against auth.x.ai using the official frozen OAuth2 scopes, OIDC token refresh, cli-chat-proxy Chat Completions transport, model catalog mapping from `/v1/models`, billing/subscription quota probe (`/v1/billing?format=credits`, `/v1/user?include=subscription`), demi credential pool for global multi-credential switch, provider event mapping, and provider-specific tests.
- Public boundary: `createGrokBuildProvider`, `parseGrokBuildProviderConfig` for serializable configuration, auth status helper, model catalog function, quota helpers, and public option types from root. `config-schema.ts` owns the parsed configuration type and the matching creator option fields.
- Endpoint boundary: explicit `baseUrl` wins, then `https://cli-chat-proxy.grok.com/v1`. Auth is the Grok CLI OAuth session or native device login against `https://auth.x.ai` (no API-key product path).
- Secret boundary: session tokens, refresh tokens, raw auth file contents, and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Chat Completions builders, stream mappers, runtime classes, credential pool IO, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-google`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`, `zod`.
- Owns: Google Gemini `generateContent` API request mapping (native wire, not OpenAI-compatible), generateContent response schemas (`response-schemas.ts`) and SSE event mapping including thought summaries / thought signatures / thinking token counts, tool-returned media as inline parts (including video), official Gemini API defaults, endpoint/env/api-key resolution, model metadata mapping, and provider-specific tests.
- Public boundary: `createGoogleProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://generativelanguage.googleapis.com/v1beta`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `GOOGLE`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: generateContent body builders, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/backend`

- Status: target contract; acceptance tracked in `docs/demi-next/progress.md`.
- Production deps: `@demicodes/agent`, `@demicodes/coding-agent`, `@demicodes/command-loader`, `@demicodes/core`, `@demicodes/host-remote`, `@demicodes/machines`, `@demicodes/provider` and the concrete providers, `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`; external: `hono` (HTTP framework, Bun runtime).
- Owns: the hosted multi-user product's server — the storage module (SQLite layer, numbered control/conversation migrations, `ControlService` over `control.sqlite`, the per-conversation `AgentTreeStore` over node and block rows, blob store, DB-backed `HostStore`), the Web API (Hono routes + the per-conversation frame-protocol WebSocket with server-side session/cwd scoping and media by reference on the way out), AgentServer assembly with the shell environment chosen per Host, runner management (pairing, device registry, one live socket per device, the rpc relay, the transfer broker, browse endpoints), the managed-hosts module (one managed device per user over the `ManagedHostProvisioner` contract, reached through `@demicodes/machines`' `RemoteProvisioner` as the machine manager's client; lifecycle/hibernate/reset, the backend-contributed `demi host` subcommand group), the LLM module (per-provider provider assembly, live model catalog, metering wrap), the credential vault (instance secret, GCM-encrypted providers, subscription device-login flows over per-provider provider pools), and usage accounting (ledger + rate limit). The backend accepts explicitly submitted API keys and setup tokens at authenticated write boundaries, passes subscription material to provider-owned credential pools, and never returns secrets or proxies model traffic.
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
  - `conversation/` — frame scoping/rewrite, attachment and remote-file references, partial metadata updates, archive admission, read/activity summaries, persistent Fork creation and publication, Cloud/device/workspace target resolution, target/file admission and per-node execution context. The root and subagents resolve the conversation selection per action.
  - `storage/` — the SQLite layer (database seam, migrations, control service, the conversation stores with the `AgentTreeStore` realization, blob store, host store).
  - `runner/` — runner management: pairing-code/device-token primitives, the registry (pending claims, one live socket per device, stable per-target `RemoteHost`s, liveness, the rpc relay), the transfer broker, installation script generation and the shared `demi host` command group. `http/runner-install.ts` serves the generated script and release artifacts; `runner/installer.ts` defines the installation steps.
  - `llm/` — the provider runtime assembled per provider entry (the family registry with each family's credential kind, the vendor catalog over models.dev, the model catalog, the Test button) and the metering wrap at the inference entry.
  - `vault/` — instance secret, credential crypto, the typed provider vault over the control plane, and the provider scope (whose providers a caller works with under the instance mode).
  - `usage/` — enforcement (the provider-request rate limiter); the ledger rows live on the `ControlService`.
  - `managed/` — one managed device per user, lazy allocation/wake, device-wide admission, idle shutdown, paired system/home checkpointing, volume growth, external system reset, Cloud project directory creation. Every VM and disk operation goes to the `ManagedHostProvisioner` it is given; the backend never spawns a hypervisor or an image tool itself.
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

- Status: target API contract; JS manifest/loader architecture retained (`docs/demi-next/commands.md`).
- Production deps: `@demicodes/shell`, `@demicodes/utils`.
- Owns: the manifest types, the manifest sources (`inMemorySource`; `directorySource` with the `writeManifestDirectory` layout), the loader (`createLoader` → `dispatch(root, argv, io)`: tree resolution, group help, argument parsing and validation, running a `runtime` module from its text or from the source's module file, forwarding an `rpc` invocation).
- Public boundary: `buildManifest`, `parseManifest` and the `Manifest` types, `createLoader` / `inMemorySource` / `directorySource` / `writeManifestDirectory`, `inProcessRpc` and the `RpcTransport` types, `treeFromManifest` from root; the `commandModulesAsText` build plugin (a `*.command.ts` file served as its text at build time) under `build`, Node-only.
- Pure JS with no runtime dependency: the same package runs in the backend, in the txiki.js runner and in tests. `buildManifest` takes the transpiler as a parameter (the backend passes Bun's); the package never transpiles on its own.
- Must not: know the backend, the runner or any Host implementation (all injected), spawn processes, or hold a command definition of its own.

### `@demicodes/runner-protocol`

- Status: implemented (the final wire: MessagePack frames, per-op fs messages, jobs, the rpc relay, the manifest push, transfers).
- Production deps: `@demicodes/shell` (the Host types the fs messages carry), `@demicodes/utils`, `@msgpack/msgpack` (the Bun end's codec).
- Owns: the backend runner wire and native-client local IPC contract — the message schemas (claim/auth handshake, liveness, the `fsOps` table from which the per-op fs requests and typed replies derive, streaming spawn, jobs, the rpc relay, the manifest push, transfers), `createRunnerWire(codec)` (encode, and decode-with-validation per direction over an injected MessagePack codec: `msgpackCodec` under `@demicodes/runner-protocol/msgpack` shared by Bun and txiki.js), the protocol constants (`RUNNER_PROTOCOL_VERSION`, `JOB_VIEW_BYTES`).
- Public boundary: message types and schemas, `createRunnerWire`, the constants from root; `msgpackCodec` under `msgpack`; strict local metadata schemas, binary framing and generated-C-header facts under `local`; matched-release schemas under `release`. Both ends of the wire depend on this package; it depends on neither end.
- Must not: contain network IO, a Host implementation, a shell environment, the job table, credentials, claim policy, device registry, or conversation state.

### `@demicodes/host-remote`

- Status: implemented (M9).
- Production deps: `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: the backend's end of a runner — `RemoteHost`, a `Host` over a connection with a jobs facet (stable object across reconnects, logical cwd fallback, injected store), and `RemoteShellEnvironment`, the `ShellEnvironment` of a real host over jobs (the model's view as the record, the working directory carried between execs). The production Host and shell the backend injects into the agent.
- Public boundary: `RemoteHost`, `RemoteShellEnvironment` and their option types from root.
- Must not: contain network IO (the wire is an injected send/handle pair), credentials, the device registry, or conversation state. `Host.store` never crosses the wire.

### `vendor/txiki.js` (C/C++ submodule, not a workspace package)

- Owns: the QuickJS-ng runtime, Web APIs, native filesystem/process/socket APIs, Linux PID 1 orphan reaping, privilege drop, file leases and interruptible workers, and the CMake target that links an embedded bytecode entry.
- Dependency: pinned fork and recursive dependencies; the runner consumes its declarations rather than redeclaring the runtime API.
- Demi builds: `packages/runner/runtime` owns native build flags, cross toolchains and application bundling inputs. Native runtime code stays in the fork.

### `packages/guest-image` (not a workspace package)

- Owns: the guest image pipeline (`docs/demi-next/managed-hosts.md` § Images): the kernel build (Linux 6.1 on Firecracker's microvm config plus `kernel/extra.config`), the rootfs build (Ubuntu by debootstrap, the toolchain list, the guest user with sudo, the runner as `/demi-runner` and native client as `/usr/bin/demi`, `mke2fs -d`), and the runner packing for Linux musl. Shell scripts and a kernel config; runs on Linux with root at build time, never at backend runtime. Its outputs (`vmlinux`, `rootfs.ext4`) are release artifacts the backend is pointed at.

### `packages/command-client` (native executable)

- Status: implemented for macOS/Linux; see `docs/demi-next/command-client.md` for Windows acceptance gaps.
- Owns: a standalone `demi` C executable with statically linked libuv; endpoint discovery, raw invocation/context transport, byte streams, cancellation and exit status. Its executable is separate from `demi-runner`.
- Depends on: libuv and the local IPC contract owned by `@demicodes/runner-protocol`; no JS engine or TypeScript runtime packages. Cross-language protocol fixtures must verify both endpoints.
- Must not: parse business command arguments, cache manifests, execute command scripts, hold backend credentials or duplicate command schemas.
- Target deployment boundary: one runner process per local backend registration, with an installation-owned matched client/runner release, credentials, manifest cache and endpoint. Multiple backend registrations may run on the same device; upgrades do not affect other installations. Jobs inherit the owning client PATH plus exact endpoint and live context.
- Target runner responsibility: host `@demicodes/command-loader`, cache manifests, parse/validate/dispatch invocations, schedule local runtime modules and forward backend RPC. Definitions remain in their owning command packages. Platform endpoint names and access controls follow `docs/demi-next/command-client.md`.

### `@demicodes/runner`

- Status: implemented on txiki.js with a separate native client. `src/entry.ts` handles runner startup, PID 1 boot and installation management; `runtime/bundle.ts` embeds the worker source; `runtime/build.ts` links the runner; `runtime/release.ts` creates matched client/runner releases.
- Production deps: `@demicodes/command-loader`, `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`; txiki.js globals and Web APIs, declared by the pinned fork's `types/src` and used through `src/machine/`.
- Owns: one outbound backend WebSocket per installation, reconnect and pairing; private installation state (`runner.json`, device token, OS lock, `active.json`, manifest cache, output and Host store); the target Host, teed shell jobs and brokered transfers; native-client IPC validation and command-loader dispatch. Each job/spawn receives an opaque live context, the exact random endpoint and a manifest-specific client PATH. Runtime leaves run in isolated interruptible workers; RPC leaves use authenticated backend transport. Managed guests keep state under `/run/demi` and the token in memory.
- Public boundary: the packed `demi-runner` binary; `packedRunner`, `startTxikiRunner`, `txikiBinary` and `bundleForTxiki` under `@demicodes/runner/testing` for Bun tests that need a runner process or run JS on txiki.js; `LocalHost` and `nodeFileSystem` in the test-only `runner/testing/` directory provide real Node filesystem/process fixtures; `HostRpcServer` and `JobTable` under `@demicodes/runner/serve` for tests that join the runner's end to a `RemoteHost` without a socket.
- Layout (directories mirror the runner's modules):
  - `machine/` — this machine as the runner sees it: the `Host` contract over txiki.js's primitives (`fs`, `process`, `cwd`, `stdio`), the teed spawn and tail reads for jobs, the WebSocket, Unix-socket and HTTP links, the codec re-export, the process itself (`argv`, `env`, `exit`, `onSignal`, `fdNode`). Accepted by the Host conformance suite on txiki.js. Internal to the runner: the agent never holds it — a machine is reached through `@demicodes/host-remote`.
  - `serve/` — the runner's end of the protocol: `HostRpcServer` (the `fs_*` and spawn messages over the machine layer) and `JobTable` (jobs over the teed spawn: the `EXIT` trap prelude, the stdin duplicate, the view budget, the job environment names).
  - `relay/` — local IPC server and authenticated backend calls. The local frame contract belongs to `runner-protocol/local`; the C client is a separate native package.
  - `commands/` — live execution contexts, worker dispatch and worker entry; no command definitions.
  - `init/` — PID 1 on a managed guest: the kernel command line as the guest's configuration, the boot as a plan of rootfs commands (kernel filesystems, persistent system overlay over the pinned base, separate home, network and `/run/demi` temporary state), per-volume usage, sync and growth. Pure over injected spawn and read, so Bun tests cover it without a kernel; `boot.ts` binds it to the machine layer.
  - `testing/` — Node-only filesystem and process fixtures; imported only through the testing entry.
  - `runner-mode.ts`, `entry.ts`, `management.ts`, `manifest-cache.ts`, `state.ts`, `transfers.ts` — runner startup/administration, manifest storage and machine-local state.
- Must not: hold credentials other than the backend-issued device token, store any conversation or transcript state, or import `@demicodes/agent`, `@demicodes/coding-agent`, provider packages, or Node in production code.

### `@demicodes/web-ui`

- Status: implemented; published to npm as a source-form package (no build step — `.vue`/`.ts`
  source exports compiled by the consumer's bundler, which must handle Vue SFC + TypeScript).
- Production deps: `@demicodes/core`, `@demicodes/agent`, `@demicodes/utils`.
- Owns: the reusable browser component library (Vue) — the agent Tab, List (+ blocks), and
  Input surfaces, the assembled ChatSession page, the message editor and its
  draft/submission lifecycle (`agent/message-editing.ts`, `SessionComposer.vue`, and the inert `MessageEditRegion.vue`), sidebar layout, workspace and
  remote-file selection flows, shared UI primitives, markdown/theme, the conversation/tab store, shared sidebar presentation and list interaction, the sign-in page (`auth/EmailLoginPage`: email and password on the left, a wide empty intro on the right, over a host-reported phase), the settings surface (`settings/`: dialog shell and panels as presentation over host-mapped models), the reusable device pairing dialog and lifecycle (`devices/`, driven by a host-provided claim adapter), and a
  transport-agnostic control-client interface. Consumes an injected `AgentClient`.
- Public boundary: source-path exports (`./*`) consumed by web hosts; third parties embed it
  by supplying an `AgentClient` and a control client. External products consume the published
  package (registry semver), not `link:` paths into this repo.
- Must not: import Node, `@demicodes/shell`, `@demicodes/coding-agent`, concrete providers, or
  `@demicodes/web` or `@demicodes/web-gallery`. It may import the `@demicodes/agent` client surface only (`AgentClient`,
  WebSocket client transport, frame/event/block types).
- Enforcement: because the components are `.vue` (not scanned by the `.ts` boundary test),
  the web-ui boundary is enforced at the package-manifest level (no Node/adapter/provider
  dependencies declared), not by the production import-graph scan.

### `@demicodes/web`

- Status: backend-integrated web product.
- Production deps: `@demicodes/web-ui`, `@demicodes/core`, `@demicodes/utils`.
- Owns: the Vue SPA application frame, route navigation, product state and backend request handlers. Vue 3 + TypeScript + Vite, vue-router, Pinia and Tailwind 4.
- Public boundary: `bun run web:dev` and `bun run web:build`; no published library API.
- Layout: `main.ts` is the only composition root (app, router, account-scoped stores); `App.vue` is the application frame; `conversation/` owns chat state and
  containers; `targets/` owns environment selection; `settings/` owns settings
  containers; `auth/` owns cookie-session state and entry containers; `api/` owns
  validated browser HTTP/agent wire contracts and upload requests; `state/` owns
  server snapshots, preferences and per-user local state; `devices/` owns pairing
  and filesystem adapters. Reusable UI belongs to `web-ui`.
- Integration boundary: authentication calls the backend over same-origin HTTP.
  Chat uses the agent client over WebSocket; resource and settings operations use
  REST. The browser does not import backend code or concrete providers.
  See `docs/web-integration.md` for state ownership and operation contracts.
- Must not: import `web-gallery`, Node, Host implementations or concrete providers.

### `@demicodes/web-gallery`

- Status: implemented.
- Production deps: `@demicodes/web-ui`, `@demicodes/core`, `@demicodes/utils`.
- Owns: the Vite-only component catalog for `@demicodes/web-ui`. It remaps `web-ui` tokens so paradigms
  (tone, accent, density, radius, shadow, light/dark) can be compared against the catalog.
  Pages are vue-router paths; Markdown and Code are full-pane preview routes. Roadmap
  lists shipped and next library work. It is not a product surface and does not
  ship multiple themes into runtime.
- Public boundary: local `bun run web:gallery` entry (`packages/web-gallery`).
- Must not: import Node, `@demicodes/shell`, `@demicodes/coding-agent`,
  concrete providers, `@demicodes/web`. Must not be imported by any
  other production package.

## Production Dependency Graph

The canonical production source graph contains every Demi package and must stay acyclic:

```text
core -> none
utils -> none
provider -> core, utils
shell -> utils
agent -> core, provider, shell, utils
coding-agent -> agent, core, shell, utils
provider-claude-code -> core, provider, utils
provider-codex -> core, provider, utils
provider-openai-api -> core, provider, utils
provider-anthropic-api -> core, provider, utils
provider-grok-build -> core, provider, utils
provider-google -> core, provider, utils
command-loader -> shell, utils
runner-protocol -> shell, utils
machines -> utils
host-remote -> runner-protocol, shell, utils
runner -> command-loader, runner-protocol, shell, utils
backend -> agent, coding-agent, command-loader, core, host-remote, machines, provider, provider-anthropic-api, provider-claude-code, provider-codex, provider-google, provider-grok-build, provider-openai-api, runner-protocol, shell, utils
web-ui -> agent, core, utils
web-gallery -> web-ui, core, utils
web -> web-ui, core, utils
```

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
- Runtime-specific code (Node, txiki.js) lives behind an entry or directory named for it (the runner's `machine/`), never in a platform-neutral root.
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
