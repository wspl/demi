# Package Boundaries

This document is the canonical package boundary contract and the highest architecture constraint for package work. When code and this document disagree, fix the code or update this document before continuing with feature work.

## Dependency Direction

Package direction is a core architecture invariant. Lower-level packages must not know higher-level products, adapters, UI shells, concrete providers, or local machine implementations.

The package registry below is the single source of truth for per-package responsibilities and boundaries. Do not scatter package-specific rules across separate sections. When a package is added, removed, renamed, or split, update its registry entry and the dependency graph together.

Test code may depend upward for integration coverage. Production code must not.

## Package Registry

### `just-bash`

- Status: implemented.
- Production deps: none.
- Owns: forked Bash parser, interpreter, builtins, expansion, portable command registry, filesystem interface, host-spawn hook, registered command hook, output hooks, audit hooks, and core bash compatibility tests.
- Portable command output that disagrees with GNU coreutils for the same flags (for example portable `ls -l` vs `stat`/`find` on `st_mode`) is a just-bash bug. Semantic fixes go upstream, not into Demi-fork-only commits.
- The in-memory VM (no `hostSpawn`) may use virtual identity and a path-string cwd. That is not a license for `@demicodes/shell` to present that VM as the Host.
- Public boundary: exposes the fork APIs consumed by `@demicodes/shell`; it is not a Demi agent runtime package.
- Must not: import Demi runtime packages or know about AgentSession, providers, REPL, or local host adapters.

### `@demicodes/core`

- Status: implemented.
- Production deps: none.
- Owns: shared data types only: transcript blocks, content blocks, model selection, thinking config, usage, and session phase.
- Public boundary: type/data contracts shared across packages.
- Must not: contain concrete provider names, catalog source names, shell runtime details, local host details, UI concepts, transport URLs, or backend identifiers.

### `@demicodes/utils`

- Status: implemented.
- Production deps: none.
- Owns: generic, platform-neutral helper functions only — type guards, error/abort helpers, async primitives, byte/UTF-8/base64 helpers, string helpers, the portable JSON codec (`Uint8Array`/`bigint` round-trip used by agent transports and HostStore implementations), and id generation.
- Public boundary: pure utility functions shared across packages; no domain types or runtime services.
- Must not: contain domain logic; transcript, provider, shell, or agent types; Node-only adapters; or any package-specific behavior.

### `@demicodes/provider`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/utils`.
- Owns: abstract provider contract, inference request items, provider events, public provider shell, hidden provider runtime factory helper, auth/runtime status, required `AgentProvider.clone()` for independent per-session runtimes, unified subscription/rate-limit quota types (`ProviderQuota` / `ProviderQuotaSnapshot`; see `docs/provider-quota.md`), optional multi-credential types (`ProviderCredentials` / `ProviderCredentialInfo` — global active switch, not multi-instance providers; see `docs/provider-global-credentials.md`), the shared node-only credential pool IO behind the `@demicodes/provider/credentials-pool` subpath (the main entry stays platform-neutral), and model catalog shape.
- Public boundary: provider contract, direct `Provider[]` composition types, quota helpers (`createProviderQuota`, `ensureQuota`), credential public types, provider test helpers only from `@demicodes/provider/testing`, and pool IO only from `@demicodes/provider/credentials-pool`.
- Model catalog boundary: common catalog state exposes portable fields only: model ids, display metadata, capability metadata, service tiers, `sourceFetchedAt`, `stale`, and `warnings`.
- Model catalog must not: expose provider-specific `source` labels such as `codex-backend`, `models.dev`, or `cache` in public types.
- Must not: import concrete providers, agent runtime, shell runtime, local host adapters, or REPL.

### `@demicodes/shell`

- Status: implemented.
- Production deps: `just-bash`, `@demicodes/utils`.
- Owns: Host contract (`defaultCwd`, `identity`, `fs`, `process` including `openCwd` / spawn-error kinds, `store`), `fileHostStore` (a `HostStore` as JSON files on any `HostFileSystem`), the Host conformance suite (`hostConformanceCases` under `@demicodes/shell/testing`, run by every Host implementation), command specs and kinds, the command ABI (`CommandContext`, `CommandResult`, `DispatchIO`, `RootPaths`, path marks, `runtimeModule`, `importCommandModule`) with the `commandModulesAsText` build plugin under `@demicodes/shell/build`, the `ShellEnvironment` contract behind the `shell_*` tools with its command records, status views and artifact store (shared by every engine), CommandRegistry (the reserved-name set injected by the engine), HostStore-scoped command storage, HostBackedFileSystem, shell sessions, command records, command artifacts, shell output, audit, storage abstractions, and shell runtime primitives used by agent-owned tools. Under `@demicodes/shell/bash`: `BashEnvironment` and the portable command set — the just-bash engine, deleted in M9.
- Public boundary: the command system and the Host contract from root, which runs on every runtime (Bun, tinyjs); platform-neutral subpaths `storage`, `host-fs`, `testing`; the just-bash engine only under `bash`. It does not expose model-facing AgentTool ownership.
- `Host.defaultCwd` is a default working-directory helper only. It is not a sandbox, workspace boundary, permission boundary, or access-control source.
- Runtime file operations go through `Host.fs`; `Host.fs` is a system-level file access facet whose allowed paths are decided by the Host backend policy, not by `defaultCwd`.
- True external process execution goes through `Host.process.spawn`.
- Registered `Command.run` receives the `BashEnvironment` Host in its execution context; command implementations use that Host instead of closing over an assembly-time Host.
- Registered commands run as virtual foreground jobs with the same control surface as host-spawned processes: an abort signal, live stdout/stderr capture, and a post-start stdin chunk stream (`CommandRunContext.signal` / `stdinStream`), so `shell_status` / `shell_write` / `shell_abort` treat them uniformly.
- Runtime state such as command JSON state and agent session snapshots goes through `Host.store`; do not keep a separate top-level store adapter boundary.
- HostBackedFileSystem adapts just-bash `IFileSystem` operations to `Host.fs` and works for local, remote, container, virtual, or policy-restricted hosts.
- BashEnvironment registers fork portable commands so a Host without coreutils still has `cat`/`ls`/`grep`; they run only when `hostSpawn` reports executable-not-found. Registering a portable Unix name when the Host has the binary is a shell defect, not a just-bash license.
- Host-backed shell behavior versus GNU bash is `docs/bash-behavior.md`. Attribute each divergence: just-bash (portable/builtin/hook), `@demicodes/shell` (registry/`hostSpawn` mapping/`HostFileStat`), or the Host backend (`posix_spawn` cwd, env). Invented exception paths are defects in the layer that owns them.
- HostSpawnHandle must use platform-neutral types; `kill` must not expose `NodeJS.Signals`.
- Must not: import `@demicodes/agent`, `@demicodes/provider`, concrete providers, `@demicodes/coding-agent`, `@demicodes/host-local`, `@demicodes/repl`, or own local Node adapters.

### `@demicodes/host-local`

- Status: implemented; deleted in M9 (`docs/demi-next/roadmap.md`). Not in the final design: user hosts and managed hosts are served by `@demicodes/host-runner` inside the runner, hostless conversations by `@demicodes/host-virtual`; the backend's use of `LocalHost` as a Node filesystem for its data directory becomes `node:fs`.
- Production deps: `@demicodes/agent`, `@demicodes/provider`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: local Node Host adapter (`LocalHost`); open-box local agent assembly (`createLocalAgentServer`) with command bridge **on by default**; command-bridge UDS listener, PATH shim materialization, and `~/.demi` / `$DEMI_HOME` state layout (`bridges/`, `bridge-bin/`).
- Public boundary: Node-only local Host + local AgentServer factory + command-bridge primitives. Store is a Host facet, not a separate adapter family.
- Spawn `cwd` and child env are this backend, not just-bash: `openCwd` holds the shell working directory as a directory fd; children receive the shell exported set, not `{ ...process.env, ...params.env }`. `HostSpawnExit.spawnError` classifies failed exec.
- May use: `node:child_process`, `node:fs`, `node:http`, `node:net`, `node:path`, `process.env`, Node streams, Buffer, process-group signaling, and `@demicodes/agent` for assembly.
- Must not: depend on concrete providers, `@demicodes/coding-agent`, or `@demicodes/repl`.
- Assembly rule: products that run on LocalHost should use `createLocalAgentServer` rather than hand-wiring command-bridge sockets. Bin dirs and UDS are LocalHost-internal, not user-facing product options.

### `@demicodes/agent`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: AgentSession, AgentServer, AgentClient, action-scoped caller metadata, transcript replay, compaction, `AgentSession.clone()` for isolated snapshot copies (see `docs/provider-session-clone.md`), transport frames, transcript patches, action-aware Host resolution, per-Host BashEnvironment reuse and shell-handle ownership checks, the model-facing standard tool surface (`shell_exec`, `shell_status`, `shell_write`, `shell_abort`, `yield`), AgentTool schemas/results, yield delayed-wakeup scheduling and steer-based wakeup delivery, repeated layered abort semantics, host-agnostic `prepareShell` and shell-origin `runCommandLine` on AgentServer, in-parent subagent supervision (ChildSupervisor, the injected `demi agent` command, subagent profiles, and the `subagent*` protocol frames; see `docs/subagent.md`), and assembly of one harness with the standard shell runtime.
- Public boundary: platform-neutral agent runtime and client/server protocol from root; explicit Node-only subpath `@demicodes/agent/stdio` for stdio transport only.
- The shell behind the `shell_*` tools is the `ShellEnvironment` contract; `AgentServer` takes a `shellEnvironment` factory per Host (default: `BashEnvironment`), so a product substitutes an engine without the agent knowing which.
- Must not: import concrete providers, `@demicodes/host-local`, or UI packages; must not own UDS sockets, PATH shim materialization, or `bridge-bin` layout.
- Runtime rule: AgentServer is the only runtime consumer that instantiates AgentSession.
- Assembly rule: AgentServer receives one AgentHarness, a public `Provider[]`, optional `prepareShell`, and shell runtime options that do not replace the shell mechanism or the standard agent tool surface. `AgentHarness.host` receives action metadata for shell operations and returns a stable Host object for each execution target. Local open-box assembly (bridge default on, UDS + shims) lives entirely in `@demicodes/host-local` via `createLocalAgentServer`.
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
- Must not: instantiate AgentSession, AgentServer, BashEnvironment, concrete providers, or LocalHost.
- Runtime rule: defines Host, commands, prompt, preamble, lifecycle, and reference resolution through the harness; it must not replace the shell mechanism, the standard agent tool surface, or provide an alternate BashEnvironment/tool runtime.

### `@demicodes/provider-claude-code`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Claude Code provider transport, JSONL/MCP mapping (including preservation of model-emitted parallel tool batches across the sequential SDK-MCP callback channel; see `docs/tool-call-concurrency.md`), model catalog mapping, provider event mapping, OAuth usage quota probe (`/api/oauth/usage`), active OAuth resolution injected into the CLI env at spawn, device-config isolation for injected-spawn runs (`CLAUDE_CONFIG_DIR` pinned inside the workspace artifacts dir — a managed device's CLI consumes zero device-local settings), provider glue over the shared credential pool (`@demicodes/provider/credentials-pool`; see `docs/provider-global-credentials.md`), and provider-specific tests.
- Public boundary: `createClaudeCodeProvider`, model catalog function, quota helpers, and public option types from root.
- Secret boundary: OAuth tokens and pool secret files stay inside the provider creator/auth resolver and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: CLI, JSONL, output, transport, parser, credential pool IO, and test cache helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

### `@demicodes/provider-codex`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Codex auth reuse, Responses transport, model catalog mapping, provider event mapping, rate-limit quota probe (`x-codex-*` headers), provider glue over the shared credential pool (`@demicodes/provider/credentials-pool`; see `docs/provider-global-credentials.md`), and provider-specific tests.
- Public boundary: `createCodexProvider`, auth status helper, model catalog function, quota helpers, transport mode type, and public option types from root.
- Secret boundary: auth.json material and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Responses builders, SSE/WebSocket transports, stream parsers, credential pool IO, and test cache helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

### `@demicodes/provider-openai-api`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: official OpenAI Responses API request mapping, explicit Chat Completions wire option for OpenAI-compatible endpoints, SSE event mapping including observed compatible reasoning delta extensions such as `choices[].delta.reasoning_content`, opt-in Chat Completions replay of thinking as `reasoning_content` (`request.passBackReasoningContent`, required for DeepSeek-style thinking + tool loops), official OpenAI API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Codex-visible defaults unless caller-supplied models replace it, and provider-specific tests.
- Public boundary: `createOpenAIApiProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.openai.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `OPENAI`. `wireApi` defaults to `responses`; compatible endpoints can pass `wireApi: 'chat-completions'`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Responses body builders, Chat Completions body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

### `@demicodes/provider-anthropic-api`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Anthropic Messages API request mapping, event-stream mapping, official Anthropic API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Claude Code defaults unless caller-supplied models replace it, and provider-specific tests.
- Public boundary: `createAnthropicApiProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.anthropic.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `ANTHROPIC`. `baseUrl` must already include the API version prefix (typically `/v1`); the provider only appends `/messages` (or leaves the URL alone when it already ends with `/messages`). Claude Code / Kimi-style roots such as `https://api.kimi.com/coding/` are not drop-in values — pass `…/coding/v1` instead.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Messages body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

### `@demicodes/provider-grok-build`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Grok Build CLI OAuth session reuse (`~/.grok/auth.json`), native RFC 8628 device login against auth.x.ai using the official frozen OAuth2 scopes, OIDC token refresh, cli-chat-proxy Chat Completions transport, model catalog mapping from `/v1/models`, billing/subscription quota probe (`/v1/billing?format=credits`, `/v1/user?include=subscription`), demi credential pool for global multi-credential switch, provider event mapping, and provider-specific tests.
- Public boundary: `createGrokBuildProvider`, auth status helper, model catalog function, quota helpers, and public option types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `https://cli-chat-proxy.grok.com/v1`. Auth is the Grok CLI OAuth session or native device login against `https://auth.x.ai` (no API-key product path).
- Secret boundary: session tokens, refresh tokens, raw auth file contents, and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Chat Completions builders, SSE readers, stream mappers, runtime classes, credential pool IO, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

### `@demicodes/provider-google`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Google Gemini `generateContent` API request mapping (native wire, not OpenAI-compatible), SSE event mapping including thought summaries / thought signatures / thinking token counts, tool-returned media as inline parts (including video), official Gemini API defaults, endpoint/env/api-key resolution, model metadata mapping, and provider-specific tests.
- Public boundary: `createGoogleProvider`, default model catalog function, and public option/model types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://generativelanguage.googleapis.com/v1beta`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `GOOGLE`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: generateContent body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

### `@demicodes/backend`

- Status: implemented through M3 (Web API skeleton + conversation module + two-plane storage + virtual default; runner management M4, LLM module/vault/accounting M5+).
- Production deps: `@demicodes/agent`, `@demicodes/coding-agent`, `@demicodes/command-loader`, `@demicodes/core`, `@demicodes/host-virtual`, `@demicodes/provider` and the concrete providers, `@demicodes/runner-protocol`, `@demicodes/shell`, `@demicodes/tinybash`, `@demicodes/utils` (`@demicodes/host-local` until M9, for a Node filesystem over the data directory only); external: `hono` (HTTP framework, Bun runtime).
- Owns: the hosted multi-user product's server — the storage module (SQLite layer, numbered control/conversation migrations, `ControlService` over `control.sqlite`, per-conversation block-row stores, blob store, DB-backed `HostStore`), the Web API (Hono routes + the per-conversation frame-protocol WebSocket with server-side session/cwd scoping), AgentServer assembly over per-conversation virtual Hosts, runner management (pairing, device registry, remote-Host resolution, browse endpoints), the managed-hosts module (`ManagedHostProvisioner` driving Firecracker under jailer through the privileged helper, images and the home-image store, lifecycle/hibernate, the backend-contributed `demi host` subcommand group), the LLM module (per-connection provider assembly, live model catalog, metering wrap), the credential vault (instance secret, GCM-encrypted connections, subscription device-login flows over per-connection provider pools), and usage accounting (ledger + rate limit). The backend never touches credential bytes (it names where a provider's pool lives) and never proxies model traffic.
- Public boundary: `createBackend`, storage module types from root; the `demi-backend` bin.
- May assemble: concrete providers, AgentServer, LocalHost (as the virtual-fs real backing), VirtualHost, and the coding harness.
- Owns the hostless shell: `HostlessEnvironment` (tinybash over the conversation's `VirtualHost`, root commands through `@demicodes/command-loader`, `rpc` in process) behind the agent server's `shellEnvironment` factory; `HOSTLESS_HOME` (`/home/demi`) and `HOSTLESS_NAMESPACE`; the manifest build with Bun's transpiler.
- Must not: be imported by any other production package; put business logic in the HTTP layer beyond routing/validation; let providers or credentials cross to runners or browsers.
- Layout (directories mirror the design record's backend modules):
  - `backend.ts` — the composition root (wire and mount only).
  - `http/` — the external HTTP surface: app assembly, one route module per resource, the WS upgrade adapter.
  - `conversation/` — conversation-module domain logic (frame scoping/rewrite, virtual-host factory).
  - `storage/` — the SQLite layer (database seam, migrations, control service, conversation stores, blob store, host store).
  - `runner/` — runner management: pairing-code/device-token primitives and the registry (pending claims, one live socket per device, stable per-target `RemoteHost`s, liveness).
  - `llm/` — provider assembly per connection (type factories, catalog, connection test) and the metering wrap at the inference entry.
  - `vault/` — instance secret, credential crypto, and the typed connection vault over the control plane.
  - `usage/` — enforcement (the provider-request rate limiter); the ledger rows live on the `ControlService`.
  - `managed/` — managed hosts: the provisioner seam + Firecracker implementation (jailer via the privileged helper, kernel and rootfs images, per-host home image, cgroup caps), lifecycle (idle hibernate to home snapshots, wake, prev release), and the `demi host` subcommand group injected into the coding command registry.
  - New modules get sibling directories — never new files at the root.

### `@demicodes/host-virtual`

- Status: implemented (M2: local-dir topology via `scopedFsBackend`; the S3 backend arrives with the scaled milestone in `@demicodes/backend`); reduced in M9 to the store-backed Host of hostless conversations, its spawn refusal deleted.
- Production deps: `@demicodes/shell`, `@demicodes/utils`.
- Owns: the virtual execution target — a platform-neutral `Host` over a pluggable `VirtualFsBackend` (virtual-absolute normalized paths): per-conversation namespace with chroot-style clamping, symlink containment, hardcoded per-file/per-conversation quotas (artifact writes exempt), spawn refusal with `executable_not_found` + upgrade guidance, logical cwd; plus `scopedFsBackend`, the real-directory backend adapter (root-prefix translation, symlink/realpath untranslation).
- Public boundary: `VirtualHost`, `scopedFsBackend`, quota constants, guidance constant from root.
- `ensureLayout` creates the working directory, the artifact directory and the declared `directories` (the backend passes the hostless namespace).
- Must not: perform its own IO (all bytes flow through the injected backend), spawn processes, or hold conversation state (`store` is injected by the composing product).

### `@demicodes/tinybash`

- Status: implemented (M8; `docs/demi-next/tinybash.md`).
- Production deps: `@demicodes/shell`, `@demicodes/utils`.
- Owns: the hostless shell — the lexer and parser for the fixed bash subset, the parse-first "inside / outside" decision (grammar, programs, flags, namespace paths under every shell state the script can reach), the executor (chains, concurrent pipelines over byte streams, redirections, session cwd and variables), and the closed set of GNU-faithful builtins over an injected `HostFileSystem`; root commands go to an injected `dispatch`.
- Public boundary: `runTinybash`, `parseTinybash`, the `OutsideReason` and IO types from root; stub roots for embedders' tests from `@demicodes/tinybash/testing`.
- Acceptance implies bash-equivalence: any script it runs means what it means in GNU bash + coreutils; anything else is `outside`, never approximated. The equivalence corpus against real bash is the guarantee's test.
- Must not: know the backend, the manifest format or the loader (it receives a `RootPaths` function and a `dispatch`), spawn processes, perform IO outside the injected `fs`, or run on real hosts.

### `@demicodes/command-loader`

- Status: implemented (M8; `docs/demi-next/commands.md`); the directory source and module import by path in M9 step 1; the socket source arrives with the runner port.
- Production deps: `@demicodes/shell`, `@demicodes/utils`.
- Owns: the manifest types, the manifest sources (`inMemorySource`; `directorySource` with the `writeManifestDirectory` layout), the loader (`createLoader` → `dispatch(root, argv, io)`: tree resolution, group help, argument parsing and validation, path-argument resolution, running a `runtime` module from its text or from the source's module file, forwarding an `rpc` invocation) and `rootPaths`, the `RootPaths` derivation tinybash consumes.
- Public boundary: `buildManifest`, `parseManifest` and the `Manifest` types, `createLoader` / `inMemorySource` / `directorySource` / `writeManifestDirectory`, `inProcessRpc` and the `RpcTransport` types, `treeFromManifest`, `rootPaths` from root.
- Pure JS with no runtime dependency: the same package runs in the backend, in tinyjs command mode and in tests. `buildManifest` takes the transpiler as a parameter (the backend passes Bun's); the package never transpiles on its own.
- Must not: know the backend, the runner, tinybash or any Host implementation (all injected), spawn processes, or hold a command definition of its own.

### `@demicodes/runner-protocol`

- Status: implemented (M1; claim flow productized in M4).
- Production deps: `@demicodes/shell`, `@demicodes/utils`.
- Owns: the runner wire protocol — message types (claim/auth handshake, liveness, Host fs RPC, streaming spawn), the portable-JSON frame codec, the backend-side `RemoteHost` proxy (a `Host` over a connection: stable object across reconnects, logical cwd fallback, injected store), and the runner-side `HostRpcServer` serving a Host's `fs`/`process` facets.
- Public boundary: message types, codec functions, `RemoteHost`, `HostRpcServer` from root.
- Must not: contain network IO (the wire is an injected send/handle pair), credentials, claim policy, device registry, or conversation state. `Host.store` never crosses this protocol.

### `@demicodes/host-runner`

- Status: implemented (M9 step 1, `docs/demi-next/runner.md`, `docs/demi-next/tinyjs.md`): accepted by the Host conformance suite run on tinyjs.
- Production deps: `@demicodes/shell`, `@demicodes/utils`; the `tinyjs:*` modules, declared once in `src/tinyjs.d.ts`.
- Owns: the `Host` contract over tinyjs's primitives — the Host every user host and managed host serves through its runner. Its counterpart on the backend is `RemoteHost` in `@demicodes/runner-protocol`: the two ends of one Host. Also the typed access to the process itself (`argv`, `env`, `cwd`, `exit`, `onSignal`, the standard streams as byte streams and writers) for the runner and the command-mode entry, which never import `tinyjs:*` themselves.
- Public boundary: `createRunnerHost` and the process access from root; `tinyjsBinary` and `bundleForTinyjs` under `@demicodes/host-runner/testing` for Bun tests that run JS on tinyjs.
- Must not: import Node, `@demicodes/host-local`, `@demicodes/agent`, `@demicodes/coding-agent`, or run anywhere but tinyjs.

### `@demicodes/runner`

- Status: implemented (M1: connection + Host RPC; M4: pairing against the product backend; the tinyjs bundle entry `src/tinyjs/entry.ts` with command mode in M9 step 1; runner mode ported to tinyjs and `@demicodes/host-runner` in M9 step 3; packaging in M10).
- Production deps: `@demicodes/host-runner`, `@demicodes/command-loader`, `@demicodes/runner-protocol`, `@demicodes/utils`; `@demicodes/host-local` and `@demicodes/shell` until the runner port.
- Owns: the runner program — the single outbound backend WebSocket with reconnect/backoff, the hello/claim handshake client, machine-local state (`~/.demi/runner.json`, `runner-token` 0600), serving the machine's Host over the runner protocol (spawn requests naming no `PATH`/`HOME` resolve against the device's own — binary resolution is a device fact), and the `demi-runner` CLI entry.
- Public boundary: `RunnerClient`, `RunnerState` from root; the `demi-runner` bin.
- Must not: hold credentials other than the backend-issued device token, store any conversation or transcript state, or import `@demicodes/agent`, `@demicodes/coding-agent`, or provider packages in production code.

### `@demicodes/repl`

- Status: implemented.
- Production deps: `@demicodes/agent`, `@demicodes/coding-agent`, `@demicodes/core`, `@demicodes/provider`, `@demicodes/provider-claude-code`, `@demicodes/provider-codex`, `@demicodes/provider-openai-api`, `@demicodes/provider-anthropic-api`, `@demicodes/provider-grok-build`, `@demicodes/shell`, `@demicodes/host-local`, `@demicodes/utils`.
- Owns: local REPL process, command-line parsing, renderer, input loop, real-provider smoke entry points, and local composition.
- Public boundary: local application entry point and test/acceptance shell.
- May assemble: concrete providers, AgentServer, LocalHost, and the coding harness.
- Must not: be imported by any other production package.

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
- Must not: import Node, `@demicodes/host-local`, `@demicodes/shell`, `@demicodes/coding-agent`, concrete
  providers, `@demicodes/web`, or `@demicodes/repl`. It may import the `@demicodes/agent` client surface
  only (`AgentClient`, WebSocket client transport, frame/event/block types).
- Enforcement: because the components are `.vue` (not scanned by the `.ts` boundary test),
  the web-ui boundary is enforced at the package-manifest level (no Node/adapter/provider
  dependencies declared), not by the production import-graph scan.

### `@demicodes/web`

- Status: implemented.
- Production deps: `@demicodes/web-ui`, `@demicodes/agent`, `@demicodes/host-local`, `@demicodes/coding-agent`,
  `@demicodes/core`, `@demicodes/provider`, `@demicodes/provider-claude-code`, `@demicodes/provider-codex`,
  `@demicodes/provider-openai-api`, `@demicodes/provider-anthropic-api`, `@demicodes/provider-grok-build`,
  `@demicodes/shell`, `@demicodes/utils`.
- Owns: the Demi web product — the Vite-dev-only browser app plus its embedded Node/Bun
  backend. The server serves only the WebSocket/API endpoints (per-session `/agent` + a
  `/control` RPC), assembling shared public providers and a per-cwd `AgentServer` over
  `LocalHost` and the coding harness. It must not serve built browser assets, preview pages,
  or production fallback HTML. The server is not split into its own package.
- Public boundary: top-level product application entry points (browser `main.ts`, server
  `index.ts`).
- Must not: be imported by any other production package.

### `@demicodes/agent-eval`

- Status: implemented.
- Production deps: `@demicodes/agent`, `@demicodes/coding-agent`, `@demicodes/core`, `@demicodes/host-local`,
  `@demicodes/provider`, `@demicodes/provider-claude-code`, `@demicodes/provider-codex`,
  `@demicodes/provider-openai-api`, `@demicodes/provider-anthropic-api`, `@demicodes/provider-grok-build`,
  `@demicodes/shell`, `@demicodes/utils`.
- Owns: agent benchmark case loading, fixture setup, Evaluator supervision/judging loop,
  oracle execution, metrics aggregation, run artifacts, reports, and gated real-provider
  evaluation entry points.
- Public boundary: local evaluation CLI and artifact schema. This package is a product leaf
  like `@demicodes/repl` and `@demicodes/web`.
- Must not: be imported by any other production package, place benchmark-specific behavior in
  runtime/provider packages, bypass provider config parsers, directly instantiate
  `AgentSession`, or mutate Worker workspaces outside declared oracle side effects.

## Production Dependency Graph

The canonical production source graph contains every Demi package and must stay acyclic:

```text
just-bash -> none
core -> none
utils -> none
provider -> core
shell -> just-bash, utils
host-local -> agent, provider, shell, utils
agent -> core, provider, shell, utils
coding-agent -> agent, core, shell, utils
provider-claude-code -> core, provider, utils
provider-codex -> core, provider, utils
provider-openai-api -> core, provider, utils
provider-anthropic-api -> core, provider, utils
provider-grok-build -> core, provider, utils
provider-google -> core, provider, utils
host-virtual -> shell, utils
tinybash -> shell, utils
command-loader -> shell, utils
host-runner -> shell, utils
backend -> agent, coding-agent, command-loader, core, host-local, host-virtual, provider, provider-anthropic-api, provider-claude-code, provider-codex, provider-google, provider-grok-build, provider-openai-api, runner-protocol, shell, tinybash, utils
runner-protocol -> shell, utils
runner -> command-loader, host-local, host-runner, runner-protocol, shell, utils
repl -> agent, coding-agent, core, provider, provider-claude-code, provider-codex, provider-openai-api, provider-anthropic-api, provider-grok-build, shell, host-local, utils
web-ui -> agent, core, utils
web -> web-ui, agent, coding-agent, core, host-local, provider, provider-claude-code, provider-codex, provider-openai-api, provider-anthropic-api, provider-grok-build, shell, utils
agent-eval -> agent, coding-agent, core, host-local, provider, provider-claude-code, provider-codex, provider-openai-api, provider-anthropic-api, provider-grok-build, shell, utils
```

`web-ui` and `web` are browser/product packages built with Vite/Vue; their internal source
is `.vue` + `.ts`. The `.ts`-only `platform-entrypoints` boundary test does not scan them as
production source. `web-ui`'s outward boundary (no Node/adapter/provider dependencies) is
enforced at the manifest level by that test; `web` is a product leaf like `repl`.

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
`runner-protocol`) need no subdirectories; the registry entry's Layout
section appears only where a package has more than one module.

## Global Boundary Rules

- Platform-neutral package roots must not statically pull Node-only adapters, concrete providers, UI code, or test helpers into their import closure.
- Public roots expose stable package contracts only; internal parser, transport, protocol, local adapter, auth-store, stream, and test helpers stay behind implementation files unless a package registry entry explicitly says otherwise.
- Any workspace package imported by production source must be declared in `dependencies`, not hidden in `devDependencies` or transitive packages.
- Local Node adapters must live in adapter packages, not in platform-neutral runtime packages.
- Do not keep compatibility shims when a package split moves an implementation to its final package.

## Verification

Existing boundary coverage:

- `packages/core/src/__tests__/platform-entrypoints.test.ts` checks platform-neutral root entries for Node-only static closure leaks.
- The same test checks that only AgentServer imports AgentSession as a runtime value outside tests.
- The same test checks that runtime source uses the forked `just-bash` package without embedded upstream snapshots or vendor imports.
- The same test checks that `@demicodes/shell` does not depend on the agent runtime.
- The same test checks selected package manifest layering boundaries.
- The same test scans `@demicodes/core` and `@demicodes/provider` production source for concrete provider names, concrete catalog source labels, backend identifiers, and product-specific source identifiers.
- The same test builds the production source package graph and fails on cycles or edges outside the enforced graph.
- The same test checks that production workspace imports are declared in package `dependencies`.
- The same test checks public provider root exports so internal transport, parser, protocol, auth-store, stream, and testing helpers do not leak through by accident.
