---
status: accepted
---

# The registry names no provider

## Problem

`provider-registry` builds every family's provider, and three of its parts
are Claude Code's:

- It reads Claude Code's newest release (`claude_releases.rs`), with the
  vendor's download address, where Codex and Grok Build read theirs in their
  own crates (`provider-codex/src/releases.rs`,
  `provider-grok-build/src/releases.rs`).
- A provider says only that it runs a process (`Capabilities::process_host`),
  and the registry answers that its CLI is Claude Code's package
  (`cli_package`).
- `FamilyArgs::machine`, `ProviderRegistry::set_machine` and
  `ProviderFamily::process_runtime` carry Claude Code's `AccountMachine` and
  `Placement`, so the registry and every family's trait depend on the Claude
  Code crate.

A second provider that runs a process would find the registry answering
for it with Claude Code's package, and every family already compiles
against Claude Code's machine.

The user shard's Claude Code machine (`conversation/claude_cli.rs`) stays
where it is: placing a process on the master's Cloud, relaying it across
users and keeping its CLI's versions need the shard's runners, and it is
the Claude Code crate's `Placement` and `AccountMachine` as the shard
provides them, as `device_requests.rs` is the registry's device route.

## Decision

Nothing the user sees changes.

- `ClaudeReleases` moves to `provider-claude-code/src/releases.rs`, with its
  default address, beside Codex's and Grok Build's. The backend's
  `--claude-releases-url` keeps its name and default.
- A provider names the package of the CLI its requests run:

  ```rust
  pub struct Capabilities {
      /// The command package whose CLI its requests run on a machine, such
      /// as `demi.claude-code`; none for a provider whose requests are HTTP.
      pub cli_package: Option<&'static str>,
  }
  ```

  It replaces `process_host`, and the registry's catalog and details read
  it.
- The registry and `ProviderFamily` carry nothing of Claude Code's:
  - The Claude Code family gets its machine from the user shard's
    `MachineSlot`, the one place the machine is, which the backend gives to
    the family and to the shard's services, and which the master's shard
    fills as it starts. `FamilyArgs::machine` and
    `ProviderRegistry::set_machine` go.
  - `ProviderFamily::process_runtime` and `ProviderRegistry::process_runtime`
    go. The shard asks the registry for a provider of the entry's own,
    built without the cache (`ProviderRegistry::own_provider`), as a session's
    runtime has one, and asks it, as Claude Code's provider, for the
    runtime with its placement. `Provider` gains `Any` as a supertrait, so
    the shard tells Claude Code's provider from another.
- `provider-registry` drops its dependencies on `provider-claude-code` and
  `command-package-claude-code-protocol`; `provider-claude-code` takes the
  protocol, whose release and package it reads.

## Spike

None: every reader of the moved and removed items was read. The only
provider that runs a process is Claude Code's; the only family that reads
`FamilyArgs::machine` is its own; the shard calls `process_runtime` from a
conversation's runtime and from Test Connection.

## Prior art

None needed: Codex and Grok Build already read their releases in their
crates, and the device route already reaches the registry from the shard.

## Alternatives

- **A process contract in `provider-common`.** `Placement`,
  `AccountMachine` and `ConfigDir` would become every provider's, but they
  carry Claude Code's idle processes, MCP version and flag refreshes; a
  contract shaped by one provider is that provider's.
- **Leave the machine in the registry until a second provider runs a
  process.** The registry and every family keep compiling against one
  provider's types meanwhile, which is the problem.

## Open questions

None.
