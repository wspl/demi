# Package Boundaries

This document is the canonical package boundary contract and the highest architecture constraint for package work. When code and this document disagree, fix the code or update this document before continuing with feature work.

## Core Rule

Package direction is a core architecture invariant. Lower-level packages must not know higher-level products, adapters, UI shells, concrete providers, or local machine implementations.

Package roots should expose stable package contracts. Node-only adapters, concrete providers, transport adapters, and test helpers must be explicit packages or explicit subpaths with narrow names.

## Current Production Graph

The implemented production source graph must stay acyclic and limited to these edges:

```text
just-bash -> none
core -> none
provider -> core
shell -> just-bash
agent -> core, provider, shell
coding-agent -> agent, core, shell
provider-claude-code -> core, provider
provider-codex -> core, provider
tui -> agent, coding-agent, core, provider, provider-claude-code, provider-codex, shell
```

Current implementation note:

- `@demi/shell/local-host` and `@demi/shell/store` still contain local Node adapters. They are not final package boundaries.
- `@demi/tui` currently imports `LocalHost` from `@demi/shell/local-host` because `@demi/host-local` has not been extracted yet.
- The next package boundary checkpoint must remove those shell subpaths instead of keeping compatibility shims.

Test code may depend upward for integration coverage. Production code must not.

## Accepted Target Graph

The accepted final split for local machine adapters is:

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

`@demi/host-local` owns local Node implementations. `@demi/shell` owns only shell contracts and shell runtime. After the split, production imports of `@demi/shell/local-host` and `@demi/shell/store` must not exist.

## Package Responsibilities

- `just-bash` owns the forked Bash parser, interpreter, builtins, expansion, filesystem interface, host-spawn hook, registered command hook, output hooks, audit hooks, and core bash compatibility tests.
- `@demi/core` owns shared data types only: transcript blocks, content blocks, model selection, thinking config, usage, and session phase.
- `@demi/provider` owns the abstract provider contract: inference request items, provider events, provider definition, provider registry, auth/runtime status, and model catalog shape.
- `@demi/shell` owns the shell runtime boundary: Host contract, command specs, CommandRegistry, DemiStore contract, AgentSessionCommandStorage, HostBackedFileSystem, BashEnvironment, shell session tools, shell output, audit, and storage abstractions.
- `@demi/host-local` owns local Node adapters: LocalHost and LocalDemiStore. It may depend on `@demi/shell` for Host and DemiStore contracts. It must not depend on `agent`, `provider`, `coding-agent`, concrete providers, or `tui`.
- `@demi/agent` owns AgentSession, AgentServer, AgentClient, transcript replay, compaction, transport frames, and assembly of one harness with the standard shell tools.
- `@demi/coding-agent` owns the coding harness, coding prompt, coding commands, todo command, and reference resolution. It defines Host and command usage; it does not replace the shell mechanism.
- `@demi/provider-claude-code` owns Claude Code provider transport and event mapping. It may depend on `core` and `provider`, not `agent`, `shell`, `coding-agent`, `host-local`, or `tui` in production code.
- `@demi/provider-codex` owns Codex auth reuse, Responses transport, model catalog mapping, and event mapping. It may depend on `core` and `provider`, not `agent`, `shell`, `coding-agent`, `host-local`, or `tui` in production code.
- `@demi/tui` is a local composition root. It may assemble concrete providers, AgentServer, LocalHost, and the coding harness. Other packages must not depend on it.

## Public Entry Rules

- Platform-neutral package roots must not statically pull Node-only adapters, concrete providers, UI code, or test helpers into their import closure.
- `@demi/shell` root exports the platform-neutral shell contract and runtime only.
- `@demi/shell/host-fs` is allowed because HostBackedFileSystem is Host-driven shell runtime, not a local Node adapter.
- `@demi/shell/storage` is allowed because it exports storage contracts and agent-session scoping, not a local filesystem implementation.
- `@demi/agent/stdio` is an explicit Node-only transport subpath. It must not be re-exported from the `@demi/agent` root.
- Concrete provider package roots expose only provider definitions, config parsers, public auth/status helpers, model catalog functions, and public option types.
- Internal auth stores, parsers, protocol mappers, transports, stream helpers, and test cache reset helpers stay behind implementation files.
- `StubProvider` and `events` are testing helpers and are exported only through `@demi/provider/testing`, not the `@demi/provider` root.

## Host Boundary

`Host` is the only shell runtime interface for executing outside the interpreter. `BashEnvironment`, coding commands, and reference resolution must depend on `Host`, not `LocalHost`.

Final Host contract requirements:

- `Host` stays in `@demi/shell`.
- `LocalHost` moves to `@demi/host-local`.
- `HostSpawnHandle.kill` uses a platform-neutral signal type such as `string` or a Demi-defined union, not `NodeJS.Signals`.
- Local process groups, `process.env`, Node streams, `Buffer`, and `node:child_process` belong only in `@demi/host-local`.

`HostBackedFileSystem` remains in `@demi/shell`. It adapts just-bash filesystem operations to the Host contract and therefore works for local, remote, or container hosts.

## Local Adapter Boundary

`@demi/host-local` should export:

```ts
export { LocalHost } from './local-host'
export { LocalDemiStore } from './local-store'
```

It should own tests for local process IO, stdin, process termination, local file store reads/writes, and path traversal rejection.

The migration must delete:

- `packages/shell/src/local-host.ts`;
- local filesystem implementation from `packages/shell/src/store.ts`;
- `@demi/shell/local-host`;
- `@demi/shell/store`.

No compatibility subpaths should remain after the extraction.

## Composition Boundary

`AgentServer` is the runtime assembly boundary. It receives:

- one `AgentHarness`;
- one `ProviderRegistry`;
- shell runtime options that do not replace the shell mechanism.

The harness may define Host, commands, prompt, preamble, lifecycle, and reference resolution. It must not provide an alternate BashEnvironment or tool runtime. Shell/Bash is part of the agent runtime core, not a replaceable coding-agent detail.

`@demi/tui` is the local application composition root: it chooses concrete provider definitions, creates the local Host, creates the coding harness, constructs AgentServer, and renders AgentClient events.

## Catalog Source Boundary

Provider model catalogs are provider capabilities. The common provider contract should expose portable catalog state, not provider-specific source labels.

Allowed common catalog fields:

- model ids and display metadata;
- capability metadata;
- service tier metadata;
- `sourceFetchedAt`;
- `stale`;
- `warnings`.

Provider-specific source labels such as `codex-backend` or `models.dev` belong inside provider implementation tests and provider-specific docs, not in `@demi/provider` public types.

## Forbidden Boundary Leaks

- `@demi/core` and `@demi/provider` must not contain concrete provider names, concrete catalog source names, product backend names, transport URLs, UI concepts, shell implementation details, or local machine implementation details.
- Provider implementations must not import `@demi/agent`, `@demi/shell`, `@demi/coding-agent`, `@demi/host-local`, or `@demi/tui` in production code.
- `@demi/shell` must not import `@demi/agent`, `@demi/provider`, concrete providers, `@demi/host-local`, or UI packages.
- `@demi/agent` must not import concrete providers, `@demi/host-local`, or UI packages.
- `@demi/coding-agent` must not instantiate AgentSession, AgentServer, BashEnvironment, concrete providers, LocalHost, or LocalDemiStore.
- `@demi/host-local` must not import `@demi/agent`, `@demi/provider`, concrete providers, `@demi/coding-agent`, or `@demi/tui`.
- Public root exports must not expose internal parser, transport, protocol, local adapter, or test helpers unless they are intentionally part of the package contract.

## Enforced Decisions

- `ProviderModel` has no provider-specific `source`; common catalog state is limited to portable fields such as `stale`, `sourceFetchedAt`, and `warnings`.
- Provider testing helpers are explicit test helpers and are not part of the provider root contract.
- Concrete provider root exports are white-listed public API.
- Any workspace package imported by production source must be declared in `dependencies`, not hidden in `devDependencies` or transitive packages.
- Local Node adapters must live in adapter packages, not in platform-neutral runtime packages.

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

Required verification when `@demi/host-local` is extracted:

- Add `@demi/host-local -> @demi/shell` to the enforced production graph.
- Add `@demi/host-local` to package manifest boundary checks.
- Assert platform-neutral production packages do not depend on `@demi/host-local`.
- Assert production source has no imports from `@demi/shell/local-host` or `@demi/shell/store`.
- Keep LocalHost and LocalDemiStore behavior tests under `packages/host-local`.
