# Package Boundaries

This document is the canonical package boundary contract and the highest architecture constraint for package work. When code and this document disagree, fix the code or update this document before continuing with feature work.

## Dependency Direction

Package direction is a core architecture invariant. Lower-level packages must not know higher-level products, adapters, UI shells, concrete providers, or local machine implementations.

The package registry below is the single source of truth for per-package responsibilities and boundaries. Do not scatter package-specific rules across separate sections. When a package is added, removed, renamed, or split, update its registry entry and the dependency graph together.

Test code may depend upward for integration coverage. Production code must not.

## Package Registry

### `@demicodes/core`

- Status: implemented.
- Production deps: none.
- Owns: shared data types only: transcript blocks, content blocks, model selection, thinking config, usage, and session phase.
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
- Owns: abstract provider contract, inference request items, provider events, public provider shell, hidden provider runtime factory helper, auth/runtime status, required `AgentProvider.clone()` for independent per-session runtimes, unified subscription/rate-limit quota types (`ProviderQuota` / `ProviderQuotaSnapshot`; see `docs/provider-quota.md`), optional multi-credential types (`ProviderCredentials` / `ProviderCredentialInfo` — global active switch, not multi-instance providers; see `docs/provider-global-credentials.md`), the shared node-only credential pool IO behind the `@demicodes/provider/credentials-pool` subpath (the main entry stays platform-neutral), model catalog shape, and the models.dev catalog client (`models-dev.ts`: the fetch with its cache and stale fallback, the zod schema of the parts read, the entry-to-catalog-model mapping) that concrete providers and the backend filter for their own lists.
- Public boundary: provider contract, direct `Provider[]` composition types, quota helpers (`createProviderQuota`, `ensureQuota`), credential public types, provider test helpers only from `@demicodes/provider/testing`, and pool IO only from `@demicodes/provider/credentials-pool`.
- Model catalog boundary: common catalog state exposes portable fields only: model ids, display metadata, capability metadata, service tiers, `sourceFetchedAt`, `stale`, and `warnings`.
- Model catalog must not: expose provider-specific `source` labels such as `codex-backend`, `models.dev`, or `cache` in public types.
- Must not: import concrete providers, the agent runtime, the shell packages, or Host implementations.

### `@demicodes/shell`

- Status: implemented.
- Production deps: `@demicodes/utils`.
- Owns three contracts and nothing that implements them. **The Host contract** (`host.ts`): `defaultCwd`, `identity`, `fs`, `process` (`openCwd`; `spawn` optional — absent on a Host that runs no processes), `store`; `fileHostStore` (a `HostStore` as JSON files on any `HostFileSystem`). **The command system** (`command.ts`, `command-abi.ts`, `storage.ts`, `reserved-names.ts`, `shell-quote.ts`): command specs and kinds, the command ABI (`CommandContext`, `CommandResult`, `DispatchIO`, `RootPaths`, path marks, `runtimeModule`, `importCommandModule`), `CommandRegistry` with the one reserved-name table, HostStore-scoped command storage. **The shell-environment contract** (`shell-environment.ts`, `command-records.ts`): `ShellEnvironment` behind the `shell_*` tools, the command record, the model's status view (each stream's delta since the last view, its tail, and the output path when the target keeps one). Engines live with their Hosts: `HostlessEnvironment` in `host-virtual`, `RemoteShellEnvironment` in `host-remote`.
- Entries: the root runs on every runtime (Bun, tinyjs; the runner bundles it); `storage` too. `testing` is the in-memory store and the Host conformance suite (`hostConformanceCases`, run by every Host implementation; the process cases apply only to a Host with `spawn`), runtime-neutral so the suite runs on tinyjs. No entry imports Node.
- `Host.defaultCwd` is a default working-directory helper only. It is not a sandbox, workspace boundary, permission boundary, or access-control source.
- Runtime file operations go through `Host.fs`; `Host.fs` is a system-level file access facet whose allowed paths are decided by the Host backend policy, not by `defaultCwd`.
- True external process execution goes through `Host.process.spawn`.
- An `rpc` command's `run` receives the invoking shell's Host in its context; command implementations use that Host instead of closing over an assembly-time Host.
- Runtime state such as command JSON state and agent session snapshots goes through `Host.store`; do not keep a separate top-level store adapter boundary.
- HostSpawnHandle must use platform-neutral types; `kill` must not expose `NodeJS.Signals`.
- Must not: import `@demicodes/agent`, `@demicodes/provider`, concrete providers, `@demicodes/coding-agent`, a Host implementation, tinybash, or Node.

### `@demicodes/agent`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: AgentSession, AgentServer, AgentClient, action-scoped caller metadata, transcript replay, compaction, `AgentSession.clone()` for isolated snapshot copies (see `docs/provider-session-clone.md`), transport frames, transcript patches, action-aware Host resolution, per-Host shell-environment reuse and shell-handle ownership checks, the model-facing standard tool surface (`shell_exec`, `shell_status`, `shell_write`, `shell_abort`, `yield`), AgentTool schemas/results, yield delayed-wakeup scheduling and steer-based wakeup delivery, repeated layered abort semantics, in-parent subagent supervision (ChildSupervisor, the injected `demi agent` command, subagent profiles, and the `subagent*` protocol frames; see `docs/subagent.md`), and assembly of one harness with the standard shell runtime.
- Public boundary: platform-neutral agent runtime and client/server protocol from root; explicit Node-only subpath `@demicodes/agent/stdio` for stdio transport only.
- The shell behind the `shell_*` tools is the `ShellEnvironment` contract; `AgentServer` takes a required `shellEnvironment` factory per Host, so a product supplies the engine and the agent never knows which one runs (the backend: `HostlessEnvironment` for a `VirtualHost`, `RemoteShellEnvironment` for a `RemoteHost`; tests: `HostlessEnvironment` over `LocalHost`).
- Must not: import concrete providers, Host implementations, or UI packages; must not own a shell interpreter.
- Runtime rule: AgentServer is the only runtime consumer that instantiates AgentSession.
- Assembly rule: AgentServer receives one AgentHarness, a public `Provider[]`, the `shellEnvironment` factory, and shell runtime options that do not replace the shell mechanism or the standard agent tool surface. `AgentHarness.host` receives action metadata for shell operations and returns a stable Host object for each execution target.
- Layout (directories mirror the package's modules; root keeps entrypoints, `types.ts`, and single-file modules like `tools.ts`):
  - `session/` — the AgentSession state machine and its collaborators (turn loop, steer queue, yield scheduler, recovery, retry policy, compaction).
  - `transcript/` — the TranscriptLog mutation journal and patch application.
  - `store/` — the session persistence realization over `HostStore` and the media blob contract (externalize/rehydrate).
  - `protocol/` — frame types, the inbound-frame zod schemas (`ClientFrame`'s single source of truth), and the transports (`stdio-transport.ts` backs the `./stdio` entry).
  - `server/` — the server facade, transport binding (frame dispatch, ingress validation), session-assembly pipeline, live-session runtime, ownership registry, and frame-view mappers.
  - `subagent/` — supervisor lifecycle and the declarative `demi agent` command tree behind the `SubagentCommandOps` seam.
  - `client/` — AgentClient.

### `@demicodes/coding-agent`

- Status: implemented.
- Production deps: `@demicodes/agent`, `@demicodes/core`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: coding harness, coding prompt, coding commands (the `demi` root: every subcommand is a noun domain group — `file` as `runtime` modules written against the ABI and `todo` as `rpc` built in, product groups like the backend's `host` composed in), and file reference resolution.
- Public boundary: harness and coding command construction based on Host and Command contracts.
- Must not: instantiate AgentSession, AgentServer, a shell environment, concrete providers, or a Host implementation.
- Runtime rule: defines Host, commands, prompt, preamble, lifecycle, and reference resolution through the harness; it must not replace the shell mechanism, the standard agent tool surface, or provide an alternate shell/tool runtime.

### `@demicodes/provider-claude-code`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Claude Code provider transport, JSONL/MCP mapping (including preservation of model-emitted parallel tool batches across the sequential SDK-MCP callback channel; see `docs/tool-call-concurrency.md`), model catalog mapping, provider event mapping, OAuth usage quota probe (`/api/oauth/usage`), active OAuth resolution injected into the CLI env at spawn, device-config isolation for injected-spawn runs (`CLAUDE_CONFIG_DIR` pinned inside the workspace artifacts dir — a managed device's CLI consumes zero device-local settings), provider glue over the shared credential pool (`@demicodes/provider/credentials-pool`; see `docs/provider-global-credentials.md`), and provider-specific tests.
- Public boundary: `createClaudeCodeProvider`, model catalog function, quota helpers, and public option types from root.
- Secret boundary: OAuth tokens and pool secret files stay inside the provider creator/auth resolver and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: CLI, JSONL, output, transport, parser, credential pool IO, and test cache helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-codex`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
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
- Internal boundary: Responses body builders, Chat Completions body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-anthropic-api`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Anthropic Messages API request mapping, event-stream mapping, official Anthropic API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Claude Code defaults unless caller-supplied models replace it, and provider-specific tests.
- Public boundary: `createAnthropicApiProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.anthropic.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `ANTHROPIC`. `baseUrl` must already include the API version prefix (typically `/v1`); the provider only appends `/messages` (or leaves the URL alone when it already ends with `/messages`). Claude Code / Kimi-style roots such as `https://api.kimi.com/coding/` are not drop-in values — pass `…/coding/v1` instead.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Messages body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-grok-build`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Grok Build CLI OAuth session reuse (`~/.grok/auth.json`), native RFC 8628 device login against auth.x.ai using the official frozen OAuth2 scopes, OIDC token refresh, cli-chat-proxy Chat Completions transport, model catalog mapping from `/v1/models`, billing/subscription quota probe (`/v1/billing?format=credits`, `/v1/user?include=subscription`), demi credential pool for global multi-credential switch, provider event mapping, and provider-specific tests.
- Public boundary: `createGrokBuildProvider`, auth status helper, model catalog function, quota helpers, and public option types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `https://cli-chat-proxy.grok.com/v1`. Auth is the Grok CLI OAuth session or native device login against `https://auth.x.ai` (no API-key product path).
- Secret boundary: session tokens, refresh tokens, raw auth file contents, and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Chat Completions builders, SSE readers, stream mappers, runtime classes, credential pool IO, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/provider-google`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Google Gemini `generateContent` API request mapping (native wire, not OpenAI-compatible), SSE event mapping including thought summaries / thought signatures / thinking token counts, tool-returned media as inline parts (including video), official Gemini API defaults, endpoint/env/api-key resolution, model metadata mapping, and provider-specific tests.
- Public boundary: `createGoogleProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://generativelanguage.googleapis.com/v1beta`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `GOOGLE`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: generateContent body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, or a Host implementation in production code.

### `@demicodes/backend`

- Status: implemented through M9 (Web API, conversation module, two-plane storage, runner management, LLM module/vault/accounting, the hostless shell, brokered transfers, media by reference; managed hosts M11, auth M12).
- Production deps: `@demicodes/agent`, `@demicodes/coding-agent`, `@demicodes/command-loader`, `@demicodes/core`, `@demicodes/host-remote`, `@demicodes/host-virtual`, `@demicodes/provider` and the concrete providers, `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`; external: `hono` (HTTP framework, Bun runtime).
- Owns: the hosted multi-user product's server — the storage module (SQLite layer, numbered control/conversation migrations, `ControlService` over `control.sqlite`, per-conversation block-row stores, blob store, home-image store, DB-backed `HostStore`), the Web API (Hono routes + the per-conversation frame-protocol WebSocket with server-side session/cwd scoping and media by reference on the way out), AgentServer assembly with the shell environment chosen per Host, runner management (pairing, device registry, one live socket per device, the rpc relay, the transfer broker, browse endpoints), the managed-hosts module (`ManagedHostProvisioner` driving Firecracker under jailer through the privileged helper, images and the home-image store, lifecycle/hibernate, the backend-contributed `demi host` subcommand group), the LLM module (per-provider provider assembly, live model catalog, metering wrap), the credential vault (instance secret, GCM-encrypted providers, subscription device-login flows over per-provider provider pools), and usage accounting (ledger + rate limit). The backend never touches credential bytes (it names where a provider's pool lives) and never proxies model traffic.
- Public boundary: `createBackend`, storage module types from root; the `demi-backend` bin.
- May assemble: concrete providers, AgentServer, `VirtualHost` over the conversation's files tree (`storage/files-tree.ts`: the `files` table and the blob store as a `VirtualFsBackend`), `RemoteHost`, and the coding harness.
- Composes the hostless shell: `createHostlessShell` builds the manifest with Bun's transpiler, the loader over the conversation's `VirtualHost` with `rpc` in process, and hands `HostlessEnvironment` (`@demicodes/host-virtual`) the root paths and the dispatcher; `HOSTLESS_HOME` (`/home/demi`) and `HOSTLESS_NAMESPACE` are the backend's constants.
- Must not: be imported by any other production package; put business logic in the HTTP layer beyond routing/validation; let providers or credentials cross to runners or browsers.
- Layout (directories mirror the design record's backend modules):
  - `backend.ts` — the composition root (wire and mount only).
  - `http/` — the external HTTP surface: app assembly, the session gate over `/api/*` with its exemptions, the cookie helpers, one route module per resource (setup, auth, transfers and blobs included), the WS upgrade adapter.
  - `auth/` — identity: the roles and the authenticated user shape, password hashing, the cookie sessions over the control plane, the login lockout.
  - `conversation/` — conversation-module domain logic (frame scoping/rewrite, attachment references, the virtual-host factory, the hostless shell composition, the execution-target resolution, the target switch and its announcement, the session upgrade as `UpgradingShell` over the two shell environments).
  - `storage/` — the SQLite layer (database seam, migrations, control service, conversation stores, blob store, host store).
  - `runner/` — runner management: pairing-code/device-token primitives, the registry (pending claims, one live socket per device, stable per-target `RemoteHost`s, liveness, the rpc relay), the transfer broker.
  - `llm/` — the provider runtime assembled per provider entry (the family registry with each family's credential kind, the vendor catalog over models.dev, the model catalog, the Test button) and the metering wrap at the inference entry.
  - `vault/` — instance secret, credential crypto, the typed provider vault over the control plane, and the provider scope (whose providers a caller works with under the instance mode).
  - `usage/` — enforcement (the provider-request rate limiter); the ledger rows live on the `ControlService`.
  - `managed/` — managed hosts: the provisioner seam, lifecycle (idle hibernate to home snapshots, wake, checkpoint, growth), the Cloud workspace creation (a host owned by the workspace over an empty home, the workspace at that home), and the `demi host` subcommand group over the current target and the grant set, injected into the coding command registry.
  - `managed/firecracker/` — the Firecracker implementation of the seam: the image tools over e2fsprogs (`mke2fs -d` from a directory, shrink after hibernate, grow the backing file), the VM process in its two launch modes (direct, or the jailer through the privileged helper), the Firecracker API over its socket, the tap slots and the per-VM kernel command line. Spawning `firecracker`, the jailer and e2fsprogs is this module's transport — the intentional external-process exception; nothing else in the backend spawns.
  - New modules get sibling directories — never new files at the root.

### `@demicodes/host-virtual`

- Status: implemented (M2; the hostless target's shell joined it in M9).
- Production deps: `@demicodes/shell`, `@demicodes/tinybash`, `@demicodes/utils`.
- Owns: the hostless execution target — its Host and its shell, the way `host-remote` owns a machine's. `VirtualHost`: a platform-neutral `Host` over a pluggable `VirtualFsBackend` (virtual-absolute normalized paths) with per-conversation namespace clamping, symlink containment, hardcoded per-file/per-conversation quotas and a logical cwd; no `spawn` — a hostless conversation runs no processes. `HostlessEnvironment`: the `ShellEnvironment` of a hostless conversation, tinybash over the Host with the loader's root paths and dispatcher injected — where Demi's Host contract and command ABI meet tinybash's own system interface; nothing beyond the model's view is kept. `HostlessEnvironment.outside` is the parse-first decision an embedder acts on before anything runs, and `handoverOf` what a machine's shell must be told to continue.
- Entries: `node` is `nodeFileSystem`, the Host filesystem over Node's `fs/promises` — the backing of the store-backed Host on the backend machine. `testing` is `hostlessShell` (the hostless shell composed over any Host with Bun's transpiler), `hostlessShellFactory`, the `probe` root (`hold`, `stdin`) that stands in for `sleep` and `read`, and `LocalHost`, the whole Host contract over this Node process's machine (`nodeFileSystem` plus child processes and a directory-fd cwd), which tests run against a real directory.
- Public boundary: `VirtualHost`, `HostlessEnvironment`, quota constants from root; `nodeFileSystem` from `node`; the fixtures from `testing`, `scopedFsBackend` among them (a `VirtualFsBackend` over a real directory, for tests; the product's hostless files are the backend's files tree).
- `ensureLayout` creates the working directory and the declared `directories` (the backend passes the hostless namespace).
- Must not: perform its own IO in the root entry (all bytes flow through the injected backend), spawn processes, or hold conversation state (`store` is injected by the composing product).

### `@demicodes/tinybash`

- Status: implemented (M8; `docs/demi-next/tinybash.md`).
- Production deps: `@demicodes/utils`.
- Owns: the hostless shell as standalone infrastructure — the lexer and parser for the fixed bash subset, the parse-first "inside / outside" decision (grammar, programs, flags, namespace paths under every shell state the script can reach), the executor (chains, concurrent pipelines over byte streams, redirections, session cwd and variables), the closed set of GNU-faithful builtins, and its own system interface (`src/host.ts`: `TinybashFs`, `TinybashIO`, `DispatchIO`, `RootPaths`) — what it asks of an embedder, declared by tinybash the way any shell declares its system calls. Demi's Host contract and loader are adapted to it by `HostlessEnvironment` in `@demicodes/host-virtual`, never the other way round.
- Public boundary: `runTinybash`, `parseTinybash`, the `OutsideReason`, the system-interface types from root; stub roots for embedders' tests from `@demicodes/tinybash/testing`.
- Acceptance implies bash-equivalence: any script it runs means what it means in GNU bash + coreutils; anything else is `outside`, never approximated. The equivalence corpus against real bash is the guarantee's test.
- Must not: import any Demi package but `@demicodes/utils` in production code (`@demicodes/shell` and `@demicodes/host-virtual` appear only as test dependencies for the corpus fixtures), know the backend, the manifest format or the loader, spawn processes, perform IO outside the injected `fs`, or run on real hosts.

### `@demicodes/command-loader`

- Status: implemented (M8; `docs/demi-next/commands.md`); the directory source and module import by path in M9 step 1; the socket source arrives with the runner port.
- Production deps: `@demicodes/shell`, `@demicodes/utils`.
- Owns: the manifest types, the manifest sources (`inMemorySource`; `directorySource` with the `writeManifestDirectory` layout), the loader (`createLoader` → `dispatch(root, argv, io)`: tree resolution, group help, argument parsing and validation, path-argument resolution, running a `runtime` module from its text or from the source's module file, forwarding an `rpc` invocation) and `rootPaths`, the `RootPaths` derivation tinybash consumes.
- Public boundary: `buildManifest`, `parseManifest` and the `Manifest` types, `createLoader` / `inMemorySource` / `directorySource` / `writeManifestDirectory`, `inProcessRpc` and the `RpcTransport` types, `treeFromManifest`, `rootPaths` from root; the `commandModulesAsText` build plugin (a `*.command.ts` file served as its text at build time) under `build`, Node-only.
- Pure JS with no runtime dependency: the same package runs in the backend, in tinyjs command mode and in tests. `buildManifest` takes the transpiler as a parameter (the backend passes Bun's); the package never transpiles on its own.
- Must not: know the backend, the runner, tinybash or any Host implementation (all injected), spawn processes, or hold a command definition of its own.

### `@demicodes/runner-protocol`

- Status: implemented (the final wire: MessagePack frames, per-op fs messages, jobs, the rpc relay, the manifest push, transfers).
- Production deps: `@demicodes/shell` (the Host types the fs messages carry), `@demicodes/utils`, `@msgpack/msgpack` (the Bun end's codec).
- Owns: the runner wire and nothing else — the message schemas (claim/auth handshake, liveness, the `fsOps` table from which the per-op fs requests and typed replies derive, streaming spawn, jobs, the rpc relay, the manifest push, transfers), `createRunnerWire(codec)` (encode, and decode-with-validation per direction over an injected MessagePack codec: `msgpackCodec` under `@demicodes/runner-protocol/msgpack` for Bun, `tinyjs:bytes` on tinyjs), the protocol constants (`RUNNER_PROTOCOL_VERSION`, `JOB_VIEW_BYTES`).
- Public boundary: message types and schemas, `createRunnerWire`, the constants from root; `msgpackCodec` under `msgpack`. Both ends of the wire depend on this package; it depends on neither end.
- Must not: contain network IO, a Host implementation, a shell environment, the job table, credentials, claim policy, device registry, or conversation state.

### `@demicodes/host-remote`

- Status: implemented (M9).
- Production deps: `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: the backend's end of a runner — `RemoteHost`, a `Host` over a connection with a jobs facet (stable object across reconnects, logical cwd fallback, injected store), and `RemoteShellEnvironment`, the `ShellEnvironment` of a real host over jobs (the model's view as the record, the working directory carried between execs). One of the two Hosts the backend injects into the agent, beside `@demicodes/host-virtual`.
- Public boundary: `RemoteHost`, `RemoteShellEnvironment` and their option types from root.
- Must not: contain network IO (the wire is an injected send/handle pair), credentials, the device registry, or conversation state. `Host.store` never crosses the wire.

### `packages/fc-helper` (Rust, not a workspace package)

- Owns: `demi-fc-helper`, the privileged helper of `jailer` mode (`docs/demi-next/managed-hosts.md` § Provisioning): `vm start` prepares the jail (kernel and rootfs linked in, the home image shared with the backend group, the socket directory group-accessible), runs the jailer and stays as the VM's parent; `vm kill` signals the recorded pid. Two verbs, whitelisted arguments, no shell. Invoked by the backend through `sudo -n`; the sudoers line for it is the backend user's only privilege.

### `packages/guest-image` (not a workspace package)

- Owns: the guest image pipeline (`docs/demi-next/managed-hosts.md` § Images): the kernel build (Linux 6.1 on Firecracker's microvm config plus `kernel/extra.config`), the rootfs build (Ubuntu by debootstrap, the toolchain list, the guest user with sudo, the runner as `/demi-runner` and `/usr/bin/demi`, `mke2fs -d`), and the runner packing for Linux musl. Shell scripts and a kernel config; runs on Linux with root at build time, never at backend runtime. Its outputs (`vmlinux`, `rootfs.ext4`) are release artifacts the backend is pointed at.

### `@demicodes/runner`

- Status: implemented on tinyjs (`src/tinyjs/entry.ts` is the bundle entry — command mode for any root name, runner mode for `demi-runner`; packaging in M14).
- Production deps: `@demicodes/command-loader`, `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/utils`; the `tinyjs:*` modules, declared once in `src/machine/tinyjs.d.ts` and imported nowhere outside `src/machine/`.
- Owns: the runner program (`runner.md`) — the single outbound backend WebSocket with reconnect/backoff and the hello/claim handshake; machine-local state under `DEMI_HOME` (`~/.demi`: `runner.json`, `runner-token` 0600, `runner.sock`, `commands/`, `bin/`, `output/`); the machine served over the protocol (spawns naming no `PATH`/`HOME` resolve against the device's own — binary resolution is a device fact); the job table over the tee; brokered transfers; the local relay and its command-mode client; the manifest cache with the `current` and root symlinks. Command mode: the loader over the machine layer, `rpc` leaves through the relay, the job's live stdin told from a redirection by `fdNode`.
- Public boundary: the packed `demi-runner` binary; `packedRunner`, `startTinyjsRunner`, `tinyjsBinary` and `bundleForTinyjs` under `@demicodes/runner/testing` for Bun tests that need a runner process or run JS on tinyjs; `HostRpcServer` and `JobTable` under `@demicodes/runner/serve` for tests that join the runner's end to a `RemoteHost` without a socket.
- Layout (directories mirror the runner's modules):
  - `machine/` — this machine as the runner sees it: the `Host` contract over tinyjs's primitives (`fs`, `process`, `cwd`, `stdio`), the teed spawn and tail reads for jobs, the WebSocket, Unix-socket and HTTP links, the codec re-export, the process itself (`argv`, `env`, `exit`, `onSignal`, `fdNode`). Accepted by the Host conformance suite on tinyjs. Internal to the runner: the agent never holds it — a machine is reached through `@demicodes/host-remote`.
  - `serve/` — the runner's end of the protocol: `HostRpcServer` (the `fs_*` and spawn messages over the machine layer) and `JobTable` (jobs over the teed spawn: the `EXIT` trap prelude, the stdin duplicate, the view budget, the job environment names).
  - `relay/` — the UDS relay: server, client, and its length-prefixed wire.
  - `init/` — PID 1 on a managed guest: the kernel command line as the guest's configuration, the boot as a plan of rootfs commands (kernel filesystems, the upper pivoted over `/`, the home, the network), the home image (the diskstats untouched report, the growth decision over `df`, `resize2fs` on `home_grown`). Pure over injected spawn and read, so Bun tests cover it without a kernel; `boot.ts` binds it to the machine layer.
  - `runner-mode.ts`, `command-mode.ts`, `entry.ts`, `manifest-cache.ts`, `state.ts`, `transfers.ts` — the two entry modes and the machine-local state they share.
- Must not: hold credentials other than the backend-issued device token, store any conversation or transcript state, or import `@demicodes/agent`, `@demicodes/coding-agent`, provider packages, or Node in production code.

### `@demicodes/web-ui`

- Status: implemented; published to npm as a source-form package (no build step — `.vue`/`.ts`
  source exports compiled by the consumer's bundler, which must handle Vue SFC + TypeScript).
- Production deps: `@demicodes/core`, `@demicodes/agent`, `@demicodes/utils`.
- Owns: the reusable browser component library (Vue) — the agent Tab, List (+ blocks), and
  Input surfaces, shared UI primitives, markdown/theme, the conversation/tab store, and a
  transport-agnostic control-client interface. Consumes an injected `AgentClient`.
- Public boundary: source-path exports (`./*`) consumed by web hosts; third parties embed it
  by supplying an `AgentClient` and a control client. External products consume the published
  package (registry semver), not `link:` paths into this repo.
- Must not: import Node, `@demicodes/shell`, `@demicodes/coding-agent`, concrete providers, or
  `@demicodes/web`. It may import the `@demicodes/agent` client surface only (`AgentClient`,
  WebSocket client transport, frame/event/block types).
- Enforcement: because the components are `.vue` (not scanned by the `.ts` boundary test),
  the web-ui boundary is enforced at the package-manifest level (no Node/adapter/provider
  dependencies declared), not by the production import-graph scan.

### `@demicodes/web`

- Status: the browser application; rebuilt in M13 on the backend's API (`docs/demi-next/roadmap.md`). Until then it carries the Vite scaffold only: no server of its own exists.
- Production deps: `@demicodes/web-ui`, `@demicodes/agent`, `@demicodes/core`, `@demicodes/utils`.
- Owns: the Demi web product's browser application — the Vite-built app over `@demicodes/web-ui`, talking to `@demicodes/backend` over its REST and WebSocket API. The backend serves the built assets in deployment (M14).
- Public boundary: the browser entry point.
- Must not: be imported by any other production package, or carry a server.

## Production Dependency Graph

The canonical production source graph contains every Demi package and must stay acyclic:

```text
core -> none
utils -> none
provider -> core
tinybash -> utils
shell -> utils
agent -> core, provider, shell, utils
coding-agent -> agent, core, shell, utils
provider-claude-code -> core, provider, utils
provider-codex -> core, provider, utils
provider-openai-api -> core, provider, utils
provider-anthropic-api -> core, provider, utils
provider-grok-build -> core, provider, utils
provider-google -> core, provider, utils
host-virtual -> shell, tinybash, utils
command-loader -> shell, utils
runner-protocol -> shell, utils
host-remote -> runner-protocol, shell, utils
runner -> command-loader, runner-protocol, shell, utils
backend -> agent, coding-agent, command-loader, core, host-remote, host-virtual, provider, provider-anthropic-api, provider-claude-code, provider-codex, provider-google, provider-grok-build, provider-openai-api, runner-protocol, shell, utils
web-ui -> agent, core, utils
web -> web-ui, agent, core, utils
```

`web-ui` and `web` are browser/product packages built with Vite/Vue; their internal source
is `.vue` + `.ts`. The `.ts`-only `platform-entrypoints` boundary test does not scan them as
production source. `web-ui`'s outward boundary (no Node/adapter/provider dependencies) is
enforced at the manifest level by that test; `web` is a product leaf like `backend`.

The graph is a compact view of the `Production deps` fields in the package registry. Every
package in the registry is implemented; keep the registry, this graph, and the maps in
`packages/core/src/__tests__/platform-entrypoints.test.ts` in lockstep.

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

Packages small enough to be a single module (e.g. `host-virtual`,
`host-remote`, `runner-protocol`) need no subdirectories; the registry
entry's Layout section appears only where a package has more than one
module.

## Global Boundary Rules

- Platform-neutral package roots must not statically pull Node-only adapters, concrete providers, UI code, or test helpers into their import closure.
- Public roots expose stable package contracts only; internal parser, transport, protocol, local adapter, auth-store, stream, and test helpers stay behind implementation files unless a package registry entry explicitly says otherwise.
- Any workspace package imported by production source must be declared in `dependencies`, not hidden in `devDependencies` or transitive packages.
- Runtime-specific code (Node, tinyjs) lives behind an entry or directory named for it (`@demicodes/host-virtual/node`, the runner's `machine/`), never in a platform-neutral root.
- Do not keep compatibility shims when a package split moves an implementation to its final package.

## Verification

Existing boundary coverage:

- `packages/core/src/__tests__/platform-entrypoints.test.ts` checks platform-neutral root entries for Node-only static closure leaks.
- The same test checks that only AgentServer imports AgentSession as a runtime value outside tests.
- The same test checks that `@demicodes/shell` does not depend on the agent runtime.
- The same test checks selected package manifest layering boundaries.
- The same test scans `@demicodes/core` and `@demicodes/provider` production source for concrete provider names, concrete catalog source labels, backend identifiers, and product-specific source identifiers.
- The same test builds the production source package graph and fails on cycles or edges outside the enforced graph.
- The same test checks that production workspace imports are declared in package `dependencies`.
- The same test checks public provider root exports so internal transport, parser, protocol, auth-store, stream, and testing helpers do not leak through by accident.
