# Dependency Boundaries

This document is the canonical package boundary contract and the highest architecture constraint for package work. When code and this document disagree, fix the code or update this document before continuing with feature work.

## Core Rule

Package dependency direction is a core architecture invariant. Lower-level packages must not know higher-level products, adapters, UI shells, or concrete provider implementations.

## Production Dependency Graph

The production source graph must stay acyclic and limited to these edges:

```text
core -> none
provider -> core
shell -> none
agent -> core, provider, shell
coding-agent -> agent, core, shell
provider-claude-code -> core, provider
provider-codex -> core, provider
tui -> agent, coding-agent, core, provider, provider-claude-code, provider-codex, shell
```

Test code may depend upward for integration coverage. Production code must not.

## Package Responsibilities

- `@demi/core` owns shared data types only: transcript blocks, content blocks, model selection, thinking config, usage, and session phase.
- `@demi/provider` owns the abstract provider contract: inference request items, provider events, provider definition, provider registry, auth/runtime status, and model catalog shape.
- `@demi/shell` owns the shell runtime boundary: Host contract, command specs, BashEnvironment, shell session tools, shell output, audit, and storage abstractions.
- `@demi/agent` owns AgentSession, AgentServer, AgentClient, transcript replay, compaction, transport frames, and assembly of a harness with the standard shell tools.
- `@demi/coding-agent` owns the coding harness, coding prompt, coding commands, todo command, and reference resolution. It defines Host and command usage; it does not replace the shell mechanism.
- `@demi/provider-claude-code` owns Claude Code provider transport and event mapping. It may depend on `core` and `provider`, not `agent`, `shell`, `coding-agent`, or `tui` in production code.
- `@demi/provider-codex` owns Codex auth reuse, Responses transport, model catalog mapping, and event mapping. It may depend on `core` and `provider`, not `agent`, `shell`, `coding-agent`, or `tui` in production code.
- `@demi/tui` is a composition root. It may assemble concrete providers, AgentServer, LocalHost, and the coding harness. Other packages must not depend on it.

## Forbidden Boundary Leaks

- `@demi/core` and `@demi/provider` must not contain concrete provider names, concrete catalog source names, product backend names, transport URLs, UI concepts, or shell implementation details.
- Provider implementations must not import `@demi/agent`, `@demi/shell`, `@demi/coding-agent`, or `@demi/tui` in production code.
- `@demi/shell` must not import `@demi/agent`, `@demi/provider`, concrete providers, or UI packages.
- `@demi/agent` must not import concrete providers or UI packages.
- `@demi/coding-agent` must not instantiate AgentSession, AgentServer, BashEnvironment, concrete providers, or LocalHost.
- Root package entries for platform-neutral packages must not statically pull Node-only adapters, concrete providers, or UI code into their import closure.
- Public root exports must not expose internal parser, transport, protocol, or test helpers unless they are intentionally part of the package contract.

## Composition Boundary

`AgentServer` is the runtime assembly boundary. It receives:

- one `AgentHarness`;
- one `ProviderRegistry`;
- shell runtime options that do not replace the shell mechanism.

The harness may define Host, commands, prompt, preamble, lifecycle, and reference resolution. It must not provide an alternate BashEnvironment or tool runtime. Shell/Bash is part of the agent runtime core, not a replaceable coding-agent detail.

`@demi/tui` is currently the local application composition root: it chooses concrete provider definitions, creates the LocalHost, creates the coding harness, constructs AgentServer, and renders AgentClient events.

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

## Enforced Decisions

- `ProviderModel` has no provider-specific `source`; common catalog state is limited to portable fields such as `stale`, `sourceFetchedAt`, and `warnings`.
- `StubProvider` and `events` are testing helpers and are exported only through `@demi/provider/testing`, not the `@demi/provider` root.
- Concrete provider root exports are white-listed public API. Internal auth stores, parsers, protocol mappers, transports, stream helpers, and test cache reset helpers stay behind implementation files.
- Any workspace package imported by production source must be declared in `dependencies`, not hidden in `devDependencies` or transitive packages.

## Verification

Existing boundary coverage:

- `packages/core/src/__tests__/platform-entrypoints.test.ts` checks platform-neutral root entries for Node-only static closure leaks.
- The same test checks that only AgentServer imports AgentSession as a runtime value outside tests.
- The same test checks that `@demi/shell` does not depend on the agent runtime.
- The same test checks selected package manifest layering boundaries.
- The same test scans `@demi/core` and `@demi/provider` production source for concrete provider names, concrete catalog source labels, backend identifiers, and product-specific source identifiers.
- The same test builds the production source package graph and fails on cycles or edges outside this document.
- The same test checks that production workspace imports are declared in package `dependencies`.
- The same test checks public provider root exports so internal transport, parser, protocol, auth-store, stream, and testing helpers do not leak through by accident.
