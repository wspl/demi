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
- Owns: abstract provider contract, inference request items, provider events, public provider shell, hidden provider runtime factory helper, auth/runtime status, unified subscription/rate-limit quota types (`ProviderQuota` / `ProviderQuotaSnapshot`; see `docs/provider-quota.md`), optional multi-credential types (`ProviderCredentials` / `ProviderCredentialInfo` — global active switch, not multi-instance providers; see `docs/provider-global-credentials.md`), the shared node-only credential pool IO behind the `@demicodes/provider/credentials-pool` subpath (the main entry stays platform-neutral), and model catalog shape.
- Public boundary: provider contract, direct `Provider[]` composition types, quota helpers (`createProviderQuota`, `ensureQuota`), credential public types, provider test helpers only from `@demicodes/provider/testing`, and pool IO only from `@demicodes/provider/credentials-pool`.
- Model catalog boundary: common catalog state exposes portable fields only: model ids, display metadata, capability metadata, service tiers, `sourceFetchedAt`, `stale`, and `warnings`.
- Model catalog must not: expose provider-specific `source` labels such as `codex-backend`, `models.dev`, or `cache` in public types.
- Must not: import concrete providers, agent runtime, shell runtime, local host adapters, or REPL.

### `@demicodes/shell`

- Status: implemented.
- Production deps: `just-bash`, `@demicodes/utils`.
- Owns: Host contract (`defaultCwd`, `fs`, `process`, `store`), command specs, CommandRegistry, HostStore-scoped command storage, HostBackedFileSystem, BashEnvironment, shell sessions, command records, command artifacts, shell output, audit, storage abstractions, and shell runtime primitives used by agent-owned tools.
- Public boundary: platform-neutral shell contract and runtime from root; platform-neutral subpaths such as `storage` and `host-fs`. It does not expose model-facing AgentTool ownership.
- `Host.defaultCwd` is a default working-directory helper only. It is not a sandbox, workspace boundary, permission boundary, or access-control source.
- Runtime file operations go through `Host.fs`; `Host.fs` is a system-level file access facet whose allowed paths are decided by the Host backend policy, not by `defaultCwd`.
- True external process execution goes through `Host.process.spawn`.
- Registered `Command.run` receives the `BashEnvironment` Host in its execution context; command implementations use that Host instead of closing over an assembly-time Host.
- Runtime state such as command JSON state and agent session snapshots goes through `Host.store`; do not keep a separate top-level store adapter boundary.
- HostBackedFileSystem adapts just-bash `IFileSystem` operations to `Host.fs` and works for local, remote, container, virtual, or policy-restricted hosts.
- BashEnvironment must register fork portable commands before falling back to `Host.process.spawn`; `cat`/`ls`/`grep`/redirection should not require local coreutils.
- HostSpawnHandle must use platform-neutral types; `kill` must not expose `NodeJS.Signals`.
- Must not: import `@demicodes/agent`, `@demicodes/provider`, concrete providers, `@demicodes/coding-agent`, `@demicodes/host-local`, `@demicodes/repl`, or own local Node adapters.

### `@demicodes/host-local`

- Status: implemented.
- Production deps: `@demicodes/agent`, `@demicodes/provider`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: local Node Host adapter (`LocalHost`); open-box local agent assembly (`createLocalAgentServer`) with command bridge **on by default**; command-bridge UDS listener, PATH shim materialization, and `~/.demi` / `$DEMI_HOME` state layout (`bridges/`, `bridge-bin/`).
- Public boundary: Node-only local Host + local AgentServer factory + command-bridge primitives. Store is a Host facet, not a separate adapter family.
- May use: `node:child_process`, `node:fs`, `node:http`, `node:net`, `node:path`, `process.env`, Node streams, Buffer, process-group signaling, and `@demicodes/agent` for assembly.
- Must not: depend on concrete providers, `@demicodes/coding-agent`, or `@demicodes/repl`.
- Assembly rule: products that run on LocalHost should use `createLocalAgentServer` rather than hand-wiring command-bridge sockets. Bin dirs and UDS are LocalHost-internal, not user-facing product options.

### `@demicodes/agent`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: AgentSession, AgentServer, AgentClient, action-scoped caller metadata, transcript replay, compaction, transport frames, transcript patches, action-aware Host resolution, per-Host BashEnvironment reuse and shell-handle ownership checks, the model-facing standard tool surface (`shell_exec`, `shell_status`, `shell_write`, `shell_abort`, `yield`), AgentTool schemas/results, yield delayed-wakeup scheduling and steer-based wakeup delivery, repeated layered abort semantics, host-agnostic `prepareShell` and shell-origin `runCommandLine` on AgentServer, and assembly of one harness with the standard shell runtime.
- Public boundary: platform-neutral agent runtime and client/server protocol from root; explicit Node-only subpath `@demicodes/agent/stdio` for stdio transport only.
- Must not: import concrete providers, `@demicodes/host-local`, or UI packages; must not own UDS sockets, PATH shim materialization, or `bridge-bin` layout.
- Runtime rule: AgentServer is the only runtime consumer that instantiates AgentSession.
- Assembly rule: AgentServer receives one AgentHarness, a public `Provider[]`, optional `prepareShell`, and shell runtime options that do not replace the shell mechanism or the standard agent tool surface. `AgentHarness.host` receives action metadata for shell operations and returns a stable Host object for each execution target. Local open-box assembly (bridge default on, UDS + shims) lives entirely in `@demicodes/host-local` via `createLocalAgentServer`.

### `@demicodes/coding-agent`

- Status: implemented.
- Production deps: `@demicodes/agent`, `@demicodes/core`, `@demicodes/shell`, `@demicodes/utils`.
- Owns: coding harness, coding prompt, coding commands, todo command, and file reference resolution.
- Public boundary: harness and coding command construction based on Host and Command contracts.
- Must not: instantiate AgentSession, AgentServer, BashEnvironment, concrete providers, or LocalHost.
- Runtime rule: defines Host, commands, prompt, preamble, lifecycle, and reference resolution through the harness; it must not replace the shell mechanism, the standard agent tool surface, or provide an alternate BashEnvironment/tool runtime.

### `@demicodes/provider-claude-code`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Claude Code provider transport, JSONL/MCP mapping, model catalog mapping, provider event mapping, OAuth usage quota probe (`/api/oauth/usage`), active OAuth resolution for CLI env overlay, provider glue over the shared credential pool (`@demicodes/provider/credentials-pool`; see `docs/provider-global-credentials.md`), and provider-specific tests.
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
- Owns: official OpenAI Responses API request mapping, explicit Chat Completions wire option for OpenAI-compatible endpoints, SSE event mapping including observed compatible reasoning delta extensions such as `choices[].delta.reasoning_content`, official OpenAI API defaults, endpoint/env/api-key resolution, compatible endpoint options, model metadata mapping mirrored from Codex-visible defaults unless caller-supplied models replace it, and provider-specific tests.
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
- Endpoint boundary: explicit `baseUrl` wins, then `${envPrefix}_BASE_URL`, then `https://api.anthropic.com/v1`; explicit `apiKey` wins, then `${envPrefix}_API_KEY`. `envPrefix` defaults to `ANTHROPIC`.
- Secret boundary: API keys, custom headers, raw endpoint values, env prefixes, and raw provider options stay inside the provider creator closure and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: Messages body builders, SSE readers, stream mappers, runtime classes, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

### `@demicodes/provider-grok-build`

- Status: implemented.
- Production deps: `@demicodes/core`, `@demicodes/provider`, `@demicodes/utils`.
- Owns: Grok Build CLI OAuth session reuse (`~/.grok/auth.json`), OIDC token refresh, cli-chat-proxy Chat Completions transport, model catalog mapping from `/v1/models`, billing/subscription quota probe (`/v1/billing`, `/v1/user?include=subscription`), demi credential pool for global multi-credential switch (see `docs/provider-global-credentials.md`), provider event mapping, and provider-specific tests.
- Public boundary: `createGrokBuildProvider`, auth status helper, model catalog function, quota helpers, and public option types from root.
- Endpoint boundary: explicit `baseUrl` wins, then `https://cli-chat-proxy.grok.com/v1`. Auth is the Grok CLI OAuth session only (no independent login flow, no API-key product path).
- Secret boundary: session tokens, refresh tokens, raw auth file contents, and pool secret files stay inside the provider creator/auth store and must not cross AgentClient/Web browser-visible frames.
- Internal boundary: auth stores, Chat Completions builders, SSE readers, stream mappers, runtime classes, credential pool IO, and test helpers stay behind implementation files.
- Must not: import `@demicodes/agent`, `@demicodes/shell`, `@demicodes/coding-agent`, `@demicodes/host-local`, or `@demicodes/repl` in production code.

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

- Status: planned.
- Production deps: `@demicodes/web-ui`, `@demicodes/agent`, `@demicodes/host-local`, `@demicodes/coding-agent`,
  `@demicodes/core`, `@demicodes/provider`, `@demicodes/provider-claude-code`, `@demicodes/provider-codex`,
  `@demicodes/provider-openai-api`, `@demicodes/provider-anthropic-api`, `@demicodes/provider-grok-build`,
  `@demicodes/shell`.
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
repl -> agent, coding-agent, core, provider, provider-claude-code, provider-codex, provider-openai-api, provider-anthropic-api, provider-grok-build, shell, host-local, utils
web-ui -> agent, core, utils
web -> web-ui, agent, coding-agent, core, host-local, provider, provider-claude-code, provider-codex, provider-openai-api, provider-anthropic-api, provider-grok-build, shell
agent-eval -> agent, coding-agent, core, host-local, provider, provider-claude-code, provider-codex, provider-openai-api, provider-anthropic-api, provider-grok-build, shell, utils
```

`web-ui` and `web` are browser/product packages built with Vite/Vue; their internal source
is `.vue` + `.ts`. The `.ts`-only `platform-entrypoints` boundary test does not scan them as
production source. `web-ui`'s outward boundary (no Node/adapter/provider dependencies) is
enforced at the manifest level by that test; `web` is a product leaf like `repl`.

The graph is a compact view of the `Production deps` fields in the package registry. A package accepted by the graph but not yet implemented is still part of the design contract.

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
