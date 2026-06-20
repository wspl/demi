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
- Owns: forked Bash parser, interpreter, builtins, expansion, filesystem interface, host-spawn hook, registered command hook, output hooks, audit hooks, and core bash compatibility tests.
- Public boundary: exposes the fork APIs consumed by `@demi/shell`; it is not a Demi agent runtime package.
- Must not: import Demi runtime packages or know about AgentSession, providers, TUI, or local host adapters.

### `@demi/core`

- Status: implemented.
- Production deps: none.
- Owns: shared data types only: transcript blocks, content blocks, model selection, thinking config, usage, and session phase.
- Public boundary: type/data contracts shared across packages.
- Must not: contain concrete provider names, catalog source names, shell runtime details, local host details, UI concepts, transport URLs, or backend identifiers.

### `@demi/provider`

- Status: implemented.
- Production deps: `@demi/core`.
- Owns: abstract provider contract, inference request items, provider events, provider definition, provider registry, auth/runtime status, and model catalog shape.
- Public boundary: provider contract and registry from root; provider test helpers only from `@demi/provider/testing`.
- Model catalog boundary: common catalog state exposes portable fields only: model ids, display metadata, capability metadata, service tiers, `sourceFetchedAt`, `stale`, and `warnings`.
- Model catalog must not: expose provider-specific `source` labels such as `codex-backend`, `models.dev`, or `cache` in public types.
- Must not: import concrete providers, agent runtime, shell runtime, local host adapters, or TUI.

### `@demi/shell`

- Status: implemented.
- Production deps: `just-bash`.
- Owns: Host contract, command specs, CommandRegistry, DemiStore contract, AgentSessionCommandStorage, HostBackedFileSystem, BashEnvironment, shell session tools, shell output, audit, and storage abstractions.
- Public boundary: platform-neutral shell contract and runtime from root; platform-neutral subpaths such as `storage` and `host-fs`.
- Runtime execution outside the interpreter goes through Host only.
- HostBackedFileSystem adapts just-bash filesystem operations to the Host contract and works for local, remote, or container hosts.
- HostSpawnHandle must use platform-neutral types; `kill` must not expose `NodeJS.Signals`.
- Must not: import `@demi/agent`, `@demi/provider`, concrete providers, `@demi/coding-agent`, `@demi/host-local`, `@demi/tui`, or own local Node adapters.

### `@demi/host-local`

- Status: implemented.
- Production deps: `@demi/shell`.
- Owns: local Node adapters, specifically LocalHost and LocalDemiStore.
- Public boundary: Node-only local host and local store implementations.
- May use: `node:child_process`, `node:fs`, `node:path`, `process.env`, Node streams, Buffer, and process-group signaling.
- Must not: depend on `@demi/agent`, `@demi/provider`, concrete providers, `@demi/coding-agent`, or `@demi/tui`.

### `@demi/agent`

- Status: implemented.
- Production deps: `@demi/core`, `@demi/provider`, `@demi/shell`.
- Owns: AgentSession, AgentServer, AgentClient, transcript replay, compaction, transport frames, transcript patches, and assembly of one harness with the standard shell tools.
- Public boundary: platform-neutral agent runtime and client/server protocol from root; explicit Node-only transports from explicit subpaths such as `@demi/agent/stdio`.
- Must not: import concrete providers, `@demi/host-local`, or UI packages.
- Runtime rule: AgentServer is the only runtime consumer that instantiates AgentSession.
- Assembly rule: AgentServer receives one AgentHarness, one ProviderRegistry, and shell runtime options that do not replace the shell mechanism.

### `@demi/coding-agent`

- Status: implemented.
- Production deps: `@demi/agent`, `@demi/core`, `@demi/shell`.
- Owns: coding harness, coding prompt, coding commands, todo command, and file reference resolution.
- Public boundary: harness and coding command construction based on Host and CommandSpec contracts.
- Must not: instantiate AgentSession, AgentServer, BashEnvironment, concrete providers, LocalHost, or LocalDemiStore.
- Runtime rule: defines Host, commands, prompt, preamble, lifecycle, and reference resolution through the harness; it must not replace the shell mechanism or provide an alternate BashEnvironment or tool runtime.

### `@demi/provider-claude-code`

- Status: implemented.
- Production deps: `@demi/core`, `@demi/provider`.
- Owns: Claude Code provider transport, JSONL/MCP mapping, model catalog mapping, provider event mapping, and provider-specific tests.
- Public boundary: provider definition, config parser, model catalog function, and public option types from root.
- Internal boundary: CLI, JSONL, output, transport, parser, and test cache helpers stay behind implementation files.
- Must not: import `@demi/agent`, `@demi/shell`, `@demi/coding-agent`, `@demi/host-local`, or `@demi/tui` in production code.

### `@demi/provider-codex`

- Status: implemented.
- Production deps: `@demi/core`, `@demi/provider`.
- Owns: Codex auth reuse, Responses transport, model catalog mapping, provider event mapping, and provider-specific tests.
- Public boundary: provider definition, config parser, auth status helper, model catalog function, transport mode type, and public option types from root.
- Internal boundary: auth stores, Responses builders, SSE/WebSocket transports, stream parsers, and test cache helpers stay behind implementation files.
- Must not: import `@demi/agent`, `@demi/shell`, `@demi/coding-agent`, `@demi/host-local`, or `@demi/tui` in production code.

### `@demi/tui`

- Status: implemented.
- Production deps: `@demi/agent`, `@demi/coding-agent`, `@demi/core`, `@demi/provider`, `@demi/provider-claude-code`, `@demi/provider-codex`, `@demi/shell`, `@demi/host-local`.
- Owns: local TUI process, command-line parsing, renderer, input loop, real-provider smoke entry points, and local composition.
- Public boundary: local application entry point and test/acceptance shell.
- May assemble: concrete providers, AgentServer, LocalHost, and the coding harness.
- Must not: be imported by any other production package.

## Production Dependency Graph

The canonical production source graph contains every Demi package and must stay acyclic:

```text
just-bash -> none
core -> none
provider -> core
shell -> just-bash
host-local -> shell
agent -> core, provider, shell
coding-agent -> agent, core, shell
provider-claude-code -> core, provider
provider-codex -> core, provider
tui -> agent, coding-agent, core, provider, provider-claude-code, provider-codex, shell, host-local
```

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
- The same test checks that `@demi/shell` does not depend on the agent runtime.
- The same test checks selected package manifest layering boundaries.
- The same test scans `@demi/core` and `@demi/provider` production source for concrete provider names, concrete catalog source labels, backend identifiers, and product-specific source identifiers.
- The same test builds the production source package graph and fails on cycles or edges outside the enforced graph.
- The same test checks that production workspace imports are declared in package `dependencies`.
- The same test checks public provider root exports so internal transport, parser, protocol, auth-store, stream, and testing helpers do not leak through by accident.
