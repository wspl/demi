---
status: proposed
---

# What one agent needs is an Apache library

## Problem

Alex wants a small agent for one person, like a coding CLI: it talks to a
model Alex configured, works on Alex's laptop or on a build machine that runs
a Demi runner, follows the project's `AGENTS.md`, and uses Demi's plugins.

Much of it is already Apache-licensed: the agent runtime (`agent-*`), every
model provider (`provider-*`), the contracts (`host-interface`,
`plugin-interface`) and the wires (`runner-protocol`, `command-*`). But four
pieces Alex cannot do without live in `crates/backend-*`, under the Demi
License, though none of them needs a server:

*   **Driving a runner.** The connection engine, `RemoteHost` and
    `RemoteShellEnvironment` are `backend-remote-host`, which depends only on
    Apache contracts.
*   **Making a provider from a configuration.** The provider families, the
    vendors from models.dev, where a catalog comes from, account rotation, the
    Claude Code release lookup and the choice of the small model are in
    `backend-providers`.
*   **Running plugins.** The plugin registry with its manifest checks, the
    composition of the command tree, a plugin's outbound requests, its OAuth
    sign-in and a user's plugin lifecycle are in `backend-plugins`.
*   **Reading the project's instructions.** Which files are searched, in
    which order, and how they become one context block are in
    `backend-user-shard`.

Alex's program, which serves no one but Alex, would fall under the server's
conditions, and the names tell every reader these pieces need the server.
The code map says the same: its backend box reads
`backend (crates/backend, backend-*, agent-*, provider-*, plugin-*)`.

## Decision

The line between the two licenses is drawn by what a part is for:

*   **Apache-2.0:** everything one agent program needs to hold a
    conversation: the agent runtime, providers and their registry, the
    plugins and their host, the runner and the end that drives it, the
    contracts and the wires.
*   **Demi License (`backend-*`):** what serving many users over the web
    needs: accounts and sessions, the HTTP edge, page synchronization, the
    Cloud and the machine manager, pairing, installers and package
    publication, the server's stores (control database, blobs, search), the
    sealed credential vault, the permission flow and the user's shard.

A library that needs storage or tells pages of a change defines the trait it
needs, and the backend realizes it over its databases and page
synchronization, as `agent-store` and `agent-server` already do.

### The moves

| Today | Becomes | Stays in the backend |
| --- | --- | --- |
| `backend-remote-host` (whole crate) | `runner-client` | nothing |
| `backend-providers`: all of `llm` (`assembly`, `families`, `vendors`, `catalog`, `catalog_cache`, `rotation`, `claude_releases`, `small_model`, `details`), `vault/quotas` and the entry types | `provider-registry` | the vault (entry records, sealing, credential pools, logins, whose entries each user infers with) and usage metering |
| `backend-plugins` (whole crate) and the plugin records of `backend-database` | `plugin-host` | its two traits' realizations: the control store's plugin tables (`backend-database`) and the page marks (`backend-page-sync`); the sealing of plugin secrets (the user's shard, through `ProductPort`) |
| `backend-user-shard`: the search and rendering of `conversation/instructions.rs` | `agent-server`, module `instructions` | how the files are read: kept reads that a runner watch refreshes, and the user's personal instructions |

Within the moves:

*   `provider-registry` holds the type today's `ProviderAssembly` becomes,
    `ProviderRegistry`, beside `FamilyRegistry`; "provider assembly" leaves
    every name. It reads an entry's accounts and credential pool, keeps
    quota snapshots and catalogs, and says when an entry's accounts changed,
    through one trait it defines, `RegistryStore`, which the vault
    implements. The catalog record joins the catalog types in
    `shared-types`.
*   `plugin-host` keeps `ProductPort` as it is, and adds one trait for what
    it stores, `PluginStore` (each user's switches and Host directories,
    the plugins' values, secrets, reports and OAuth clients, each
    conversation's work panel and the device it works on), and one for the
    changes pages are told of, `PluginChanges`. The records they speak in
    move with them out of `backend-database`, which implements
    `PluginStore`.
*   `agent-server`'s `instructions` module searches the project's
    directories through a file reader the product passes in, and renders
    the block. The design choices in today's entry comment move with it.
    The batched reads it searches with (`HostRead`, `HostFile`) and the
    walk to the repository's root (`project`) move from `plugin-interface`
    to `host-interface`, beside the Host's paths: they read a Host's files,
    which plugins, the product and the agent runtime all do, and the agent
    runtime knows no plugin.

### Names

Each name follows a precedent, and a name that had none was a sign of a
wrong layer:

| Name | Precedent |
| --- | --- |
| `runner-client` | Codex's `exec-server`, the counterpart of the runner, is driven by `ExecServerClient`; Docker's Go library is `client`, Kubernetes' `client-go`. |
| `provider-registry` | Goose's `providers/provider_registry.rs`, with a `ProviderEntry` as Demi's; the Vercel AI SDK's `createProviderRegistry`. |
| `plugin-host` | Zed's `extension_host`, VS Code's Extension Host, Codex's `plugin-host`; the code already calls this part the plugin host. |
| `agent-server::instructions` | Codex keeps `agents_md.rs` in its core, Gemini CLI `memoryDiscovery.ts` in its core, Goose `hints/` in its core. No project gives it a package, so it joins the agent runtime rather than becoming `agent-context`. |

`ARCHITECTURE.md` gains a suffix: `-client` holds the end of a wire that
drives a program; `-sdk` stays the end a program is written with, as in
`command-sdk` and `plugin-sdk`. § Names says that `runner-` also names the
runner's wire, whose `runner-protocol` and `runner-client` are not part of
the runner executable. The code map's backend box names only
`crates/backend` and `backend-*`, and lists the Apache libraries it links
under it.

### Licensing

Moving code out of `crates/backend-` relicenses it under Apache-2.0 alone,
which cannot be taken back for what is released: about 10,800 lines of
`runner-client`, 2,500 of `provider-registry`, 3,100 of `plugin-host` and
300 of the instructions. `LICENSE-DEMI` keeps its text: it names the
`crates/backend-` prefix, which the moved code leaves. The READMEs' lists of
Apache parts, in every language, add the library that drives a runner, the
provider registry and the plugin host.

No Apache library depends on a Demi License crate, before or after. The one
Apache crate that links one, `xtask`, the repository's developer tool, links
the backend only to run it in development, and no release ships it.

### What does not change

No behavior, wire message or stored value changes, and an older runner or
page meets the same backend. Log events keep their names, except
`assembly.set_twice`, which the rename makes `provider_registry.set_twice`.
`runner-protocol` and `runner-jobs` keep calling the runner's peer "the
backend", which it is in the product.
`backend-idle-watch` and `backend-blobs` stay.

### Work packages

One commit per move, in the order of the table, on one branch merged at
once. This record lands with the first.

## Spike

The traits of `provider-registry` and `plugin-host` are drawn from the
calls the moving code makes today, and both are built: `provider-registry`'s
`RegistryStore` has eight methods over the vault and the control store, and
`plugin-host`'s `PluginStore` twenty over the control store, each one of
its existing operations, with `PluginChanges` four over the page marks. Building each move settles its trait; a seam
that turns out wider than these names comes back to this record.

## Prior art

*   **Codex** (Apache-2.0): every part is a library and `app-server` is a
    transport over them. Persistence is a storage-neutral `thread-store`
    with `rollout` (JSONL files) and `state` (SQLite) realizing it.
*   **Goose** (Apache-2.0): the `goose` library holds providers and their
    registry, extensions, sessions in SQLite, permissions and the
    scheduler; `goose-cli` and its server are shells over it.
*   **matrix-rust-sdk**: `matrix-sdk-base` defines the store traits and
    `matrix-sdk-sqlite` realizes them.

Demi departs in that its line also decides a license: Codex and Goose are
Apache throughout. Like Grafana, whose `LICENSING.md` lists the Apache
directories of an AGPL repository, Demi draws it by directory.

## Alternatives

*   **List exceptions in LICENSE-DEMI.** The license would need a list to
    read, and the names would still say the parts need the server.
*   **One `agent-sdk` crate holding all four, as Goose's library does.** It
    would mix the runner's wire, providers and plugins, which Demi's crates
    keep apart by owner, and rebuild them all for a change to one.
*   **`runner-sdk`.** In Demi `-sdk` is what a program is written with
    (`command-sdk`, `plugin-sdk`), so it would read as the kit for writing a
    runner.

## Open questions

None. Deferred to a later record: a SQLite realization of the tree store
(`agent-store-sqlite`, after `matrix-sdk-sqlite`), and a credential store
for one person's agent.
