# Architecture

This document is the map of the code and the rules that hold across it. The
[overview](docs/overview.md) describes what Demi is and follows one
conversation end to end. What one module decides lives in its entry comment
(`lib.rs`, `main.rs`, `index.ts`), next to the code. When this document and
the code disagree, the code is right and this document is fixed.

## Code map

The following diagram shows the programs and how they talk to each other:

```text
web app (packages/web)  --- HTTP, sync and conversation sockets --->
backend (crates/backend, backend-*, agent-*, provider-*, plugin-*)
   |                                    |
   | runner wire                        | JSON lines on a Unix socket
   v                                    v
runner (crates/runner, runner-*)    machine manager (crates/machine-manager)
   |                                    | starts each Cloud with its runner
   | command invocations                v
   v                                 Cloud sandbox
command programs (command-package-*): demi-file, demi-browser,
demi-fetch, demi-mobile, demi-claude-code
```

Each program's inside is drawn in its entry comment. The following table lists
the entry comments:

| Part | Entry comment |
| --- | --- |
| The backend's layers | `crates/backend/src/lib.rs` |
| A user's shard | `crates/backend-user-shard/src/lib.rs` |
| The runner | `crates/runner/src/main.rs` |
| The machine manager | `crates/machine-manager/src/lib.rs` |
| The plugin contract | `crates/plugin-interface/src/lib.rs` |
| The page API | `packages/plugin-sdk/src/index.ts` |

## Capabilities as plugins

Every capability is a plugin. The agent runtime, the backend and the web app
know a capability only through the plugin contract (`plugin-interface`).

A plugin contributes the following, all as messages:

-   Commands
-   Text for the model's context
-   Host directories
-   A part of the page

Three parties supply what the agent can do:

-   The agent runtime supplies its own tools, `demi agent` and `demi shell`.
-   The product supplies the conversation's Host, `demi host` and
    `demi attachment`.
-   Plugins supply everything else the agent can do.

### State on a Host

A capability that keeps state on a Host is also a command package behind the
runner's narrow port. The browser shows how this works:

-   `command-package-browser-protocol` holds the browser's operation types.
-   The `demi.browser` package (`command-package-browser` with `-chrome`)
    carries the operations out.
-   `plugin-browser` declares the operations as `demi browser` commands.
-   The package keys the browser's tabs by the conversation the runner names
    on every invocation. The generic conversation release ends the tabs.
-   No browser type or message exists in the runner crates,
    `runner-protocol`, `backend-remote-host`, the agent or the lifecycle.

### Add a capability

To add a capability:

1.  Add a `plugin-<name>` crate.
2.  When the capability keeps Host state, add a command package, with its
    operation types in a contract crate.

Adding a capability changes none of the following:

-   The runner
-   `runner-protocol`
-   `backend-remote-host`
-   The agent
-   `backend-plugins`
-   The lifecycle

A message, adapter or lifecycle hook for one tool in any of them belongs at
the plugin contract or the runner's port instead.

A shared mechanism goes where its use decides:

-   A mechanism answered without knowing who asked or why goes in the runner
    wire. A file operation, a working-tree listing and a service stream are
    examples.
-   A mechanism usable by any plugin goes in the plugin contract. A Host
    directory is an example.

## Dependency direction

Dependencies point down. The following rules apply:

-   A crate or package never knows the products, adapters, user interfaces,
    providers or machines above it, in production code or in tests.
-   A test reaches a fake through the `testing` feature of the crate that owns
    what it fakes.
-   No crate has a dev-dependency on a crate that depends on it, except on
    itself to turn on its own `testing`.
-   A crate's public items are the ones its entry comment names. The rest is
    `pub(crate)` or private.

### Backend layers

Inside the backend, each layer knows only the layers beneath it. A domain that
needs another domain's state follows these rules:

-   The domain defines the narrow trait of what it needs (`CloudShard`,
    `HostShard`, `PermissionShard`, `PluginShard`).
-   `backend-user-shard` implements the trait.
-   No crate below `backend-user-shard` sees `Shard` or `Services`.

### Runner crates

The runner is one executable of several crates, so a change recompiles only
its part. Only `runner-shell` links the shell and its utilities, and nothing
below the executable depends on it.

### Graphs and manifests

The [dependency graphs](#dependency-graphs) list the required crates and
packages and the edges between them. Manifests follow the graphs. When a
`Cargo.toml` or `package.json` disagrees with its line, either the manifest
changes, or the graph changes first with the work package that needs it.

Both graphs stay acyclic. They leave out external and vendored crates.

## Names and layout

The following sections cover where code lives, how to name it, when to split
it, how to organize a crate and how many test binaries a crate has.

### Directories

Code lives in these directories:

-   Rust crates live in `crates/`.
-   TypeScript packages live in `packages/`.
-   Vendored crates live in `vendor/<upstream>/<crate>/`.

### Names

A crate directory is `<owner>-<part>`. The owner is `host-`, `plugin-` or one
of the following:

-   `backend-`, `runner-` and `machine-manager-` name one program each.
-   `agent-` names the conversation runtime.
-   `provider-` names model providers.
-   `command-` names the command wire.
-   `command-package-<name>-` names one package, released as `demi.<name>`.
-   `shared-` names code with no single owner.

A program is named by its owner alone. A library's package is
`demi-<directory>`, and a program's package is its executable's name.

A suffix tells what a crate holds:

-   `-protocol` holds the types of one wire or data family, and the decode
    function of each boundary that receives them. It has no IO and no async
    runtime.
-   `-sdk` holds one end of a wire.
-   `-interface` holds traits, and nothing that implements them.

Words whose meaning differs outside stay out of names: `service`, `extension`,
`edge` and `core`. The word `plugin` names only a plugin, `browser` appears
only in the browser's own crates (`command-package-browser*` and
`plugin-browser`), and `shell` appears only for the shell.

### When to split

A separate crate or package needs one of these reasons:

-   Independent use
-   Distribution
-   Dependency isolation
-   Build isolation

Code that changes together is modules of one crate. Never split it because it
is long.

Cargo rebuilds every crate above a changed one, so a split saves build time
only when the part split off is not above the code that changes. Place code
accordingly:

-   Code that changes often belongs above.
-   Types many crates read belong below.

### Inside a crate

Inside a crate, these rules apply:

-   Each executable has one composition root (`main.rs`, `Backend::start`, the
    web app's `main.ts`). The root constructs, injects, mounts and returns,
    and never branches on business state.
-   Modules follow responsibilities. There is no `misc`, `helpers` or `utils`
    module.
-   Generic Rust comes from std or an established crate, generic web code
    comes from `@demicodes/utils`, and a domain helper sits beside its module.
-   Generated TypeScript (`src/generated/` directories) stays uncommitted.
-   A vendored crate keeps its upstream metadata and licenses. Its
    `[package.metadata.demi]` names each change from upstream with its reason,
    and `bun xtask vendor diff` compares the crate with crates.io.

### Test binaries

These rules decide how many test binaries a crate has:

-   Each crate has one test binary, its integration tests, which test the
    public boundary with a module per area.
-   The library and every executable set `test = false`. Every library sets
    `doctest = false` and `bench = false`. Otherwise, `--all-targets` compiles
    each library twice.
-   A crate without programs whose behavior is observable only inside keeps
    unit tests and no `tests/`.
-   A test gets a binary of its own only when it changes or exhausts
    process-wide state (such as the open-file limit), or needs what others
    must not have (such as root). Its file says so at the top.
-   A test reaches another program as the binary Cargo built.

## Contracts

Define every message and stored document that crosses a process boundary once,
as a Rust type. Its JSON Schema, TypeScript type and validator derive from it
(`bun run contracts`). A second declaration of the same shape is a defect, even
while the two agree.

The encoding conventions are in `crates/shared-types/src/lib.rs`. The
generator is in `crates/xtask/src/contracts/`.

The following table shows where each wire or data family is defined:

| Wire or data | Defined in |
| --- | --- |
| The web app's HTTP routes | `web-api-protocol` |
| Conversation socket frames, blocks, patches | `conversation-socket-protocol`, `shared-types` |
| Runner wire, its manifests and the managed boot record | `runner-protocol` |
| Command invocations | `command-protocol` |
| One command package's operations | its `command-package-<name>-protocol` |
| Live view frames and input | `live-view-protocol` |
| Plugin manifests, requests and replies | `plugin-interface` |
| A plugin's page state and calls | the plugin's crate |
| Machine manager socket, Cloud image manifest | `machine-manager-protocol` |
| JSON stored in the databases | the crate that owns the data |

Contracts follow these further rules:

-   The runner wire and the machine manager socket have both ends in Rust and
    fixed encodings. A golden corpus of every message kind pins them.
-   A vendor's API is not a Demi contract. Each provider crate declares only
    what it reads.
-   A boundary decodes a value from outside a process at its entry, against
    its type. After that, nothing checks the value again. Before that, nothing
    looks inside it.
-   A boundary refuses corrupt data and never repairs or defaults it.

## Concurrency

The following sections cover where work runs, how it shares state, how
blocking work leaves the async threads, how cancellation works and how tests
handle time.

### Where work runs

All programs use Tokio.

State that one user or one connection owns lives on one single-threaded
runtime, so it needs no lock and a test's paused clock reaches it. The
following run on such a runtime:

-   A backend shard thread
-   The runner's control loop
-   The machine manager's loop

Bytes, and work across users, run in parallel at the edge.

Each runtime builds its own `reqwest::Client`, because a pooled connection's
task is tied to the runtime that made it.

### Shared state

Shared state follows these rules:

-   Shared state has one owner that answers requests.
-   Work that must not overlap across awaits takes a named gate from
    `shared-gates`.
-   An owner publishes read-mostly data as a snapshot through `watch`.
-   Code never holds a lock guard or `RefCell` borrow across an `.await`.

### Blocking work

CPU work over about a millisecond, and every blocking call, runs off the async
threads, through `spawn_blocking` or a thread of its own.

### Cancellation

Cancellation follows these rules:

-   Cancellation follows ownership, through a `CancellationToken` tree.
-   Guards release on drop, on success, failure and cancellation alike.
-   Cancelling stops waits and never a commit. Code never races a store save.
-   A release that must wait is an explicit `close()`. Drop is the fallback,
    and it kills.

### Tests and time

Tests handle time by these rules:

-   Timer logic inside one process runs on Tokio's paused clock, so a test
    asserts exact times.
-   A test whose timers run while it waits on a real runner, service or
    database uses real time with short windows, because a paused clock jumps
    to the next timer whenever the runtime is idle.
-   Wall time comes from an injected `Clock`.
-   A backend test without a real runner runs the shards on the test's own
    runtime (`ShardPlacement::Inline`).

## Dependency graphs

Each graph lists every member once, as `name -> dependency, dependency`, or as
`name -> none`. The crate check (`crates/xtask/src/boundaries.rs`) and the
package check (`scripts/__tests__/package-boundaries.test.ts`) read these
blocks and fail until every manifest agrees with its line. Each graph
therefore stays one fenced `text` block in this form.

### Rust crates

A line names a workspace member by its directory. The following graph lists the
Rust crates:

```text
shared-types -> none
log-capture -> none
command-protocol -> none
conversation-socket-protocol -> shared-types
command-sdk -> shared-artifacts, command-protocol
command-declarations -> none
command-package-file-protocol -> shared-types
command-package-fetch-protocol -> shared-types
command-package-browser-protocol -> shared-types, live-view-protocol
command-package-mobile-protocol -> shared-types, live-view-protocol
live-view-protocol -> shared-types
command-package-claude-code-protocol -> none
runner-protocol -> command-protocol, command-declarations, shared-types
machine-manager-protocol -> command-protocol, runner-protocol
web-api-protocol -> conversation-socket-protocol, command-protocol, shared-types, runner-protocol
shared-gates -> none
shared-artifacts -> none
shared-cli -> none
provider-common -> shared-types, shared-gates, shared-artifacts
provider-anthropic-api -> shared-types, provider-common
provider-openai-api -> shared-types, provider-common
provider-google -> shared-types, provider-common
provider-bytedance -> shared-types, provider-common
provider-elevenlabs -> shared-types, provider-common
provider-minimax -> shared-types, provider-common
provider-codex -> shared-types, provider-common, shared-artifacts
provider-grok-build -> shared-types, provider-common
provider-claude-code -> shared-types, provider-common, host-interface, shared-artifacts
host-interface -> command-protocol, command-declarations, shared-types
agent-store -> conversation-socket-protocol, command-protocol, shared-types, provider-common, host-interface
agent-transcript -> conversation-socket-protocol, agent-store, shared-types, provider-common
agent-session -> conversation-socket-protocol, agent-store, agent-transcript, command-protocol, shared-types, shared-gates, provider-common, host-interface
agent-tools -> conversation-socket-protocol, agent-session, agent-store, agent-transcript, command-protocol, shared-types, provider-common, host-interface
agent-server -> conversation-socket-protocol, agent-session, agent-store, agent-tools, agent-transcript, shared-types, shared-gates, provider-common, host-interface
plugin-interface -> command-declarations, shared-types, host-interface, web-api-protocol
plugin-file -> plugin-interface, command-declarations, command-package-file-protocol, host-interface
plugin-browser -> plugin-interface, command-declarations, command-package-browser-protocol, host-interface, web-api-protocol
plugin-mobile -> plugin-interface, command-declarations, command-package-mobile-protocol, host-interface, live-view-protocol, web-api-protocol
plugin-skills -> plugin-interface, host-interface, shared-types
plugin-web -> plugin-interface, command-declarations, command-package-fetch-protocol, host-interface
plugin-media -> plugin-interface, host-interface, shared-types
plugin-image -> plugin-media, plugin-interface, host-interface, shared-types
plugin-video -> plugin-media, plugin-interface, host-interface, shared-types
plugin-audio -> plugin-media, plugin-interface, host-interface, shared-types
plugin-changes -> plugin-interface
plugin-file-browser -> plugin-interface
backend-remote-host -> command-protocol, command-declarations, shared-types, shared-gates, runner-protocol, host-interface
runner-process -> shared-artifacts, command-protocol, command-sdk, runner-protocol
runner-host -> shared-artifacts, command-protocol, command-sdk, runner-process, runner-protocol
runner-direct -> live-view-protocol, runner-protocol
runner-command-packages -> shared-artifacts, command-protocol, command-sdk, runner-process, runner-protocol
runner-shell -> command-protocol, command-sdk, runner-process, runner-protocol
runner-jobs -> command-protocol, command-sdk, command-declarations, runner-process, runner-protocol, runner-command-packages
live-view -> command-sdk, live-view-protocol, shared-types
command-package-browser-chrome -> command-package-browser-protocol, command-protocol, command-sdk, live-view, live-view-protocol, shared-artifacts, shared-types
backend-page-sync -> backend-database, web-api-protocol
backend-database -> agent-store, conversation-socket-protocol, shared-types, shared-gates, backend-remote-host, machine-manager-protocol, runner-protocol, host-interface, web-api-protocol, plugin-interface
backend-blobs -> agent-store, shared-types, web-api-protocol
backend-accounts -> backend-database, command-protocol, shared-types, web-api-protocol
backend-providers -> backend-database, backend-page-sync, command-package-claude-code-protocol, shared-types, provider-common, provider-anthropic-api, provider-claude-code, provider-openai-api, web-api-protocol
backend-runners -> shared-artifacts, backend-blobs, backend-database, backend-page-sync, command-protocol, shared-gates, backend-remote-host, runner-protocol, host-interface, web-api-protocol
backend-idle-watch -> shared-gates
backend-cloud -> backend-idle-watch, backend-providers, backend-runners, backend-database, backend-page-sync, shared-gates, backend-remote-host, machine-manager-protocol, runner-protocol, host-interface, web-api-protocol
backend-plugins -> plugin-interface, command-declarations, shared-types, host-interface, backend-database, backend-page-sync, web-api-protocol, provider-common
backend-permissions -> command-declarations, shared-types, host-interface, backend-database, backend-page-sync, web-api-protocol
backend-host-access -> agent-store, agent-tools, backend-cloud, backend-blobs, backend-runners, backend-database, command-protocol, command-declarations, shared-types, shared-gates, backend-remote-host, runner-protocol, host-interface, web-api-protocol, plugin-interface
backend-user-shard -> agent-server, conversation-socket-protocol, agent-session, agent-store, agent-tools, agent-transcript, backend-accounts, backend-cloud, backend-host-access, backend-idle-watch, backend-blobs, backend-permissions, backend-plugins, backend-providers, backend-runners, backend-database, backend-page-sync, command-package-claude-code-protocol, command-protocol, command-declarations, shared-types, shared-gates, plugin-interface, backend-remote-host, machine-manager-protocol, provider-common, provider-claude-code, runner-protocol, host-interface, web-api-protocol
backend-http -> conversation-socket-protocol, agent-store, shared-artifacts, backend-accounts, backend-cloud, backend-host-access, backend-blobs, backend-providers, backend-runners, backend-user-shard, backend-database, backend-page-sync, command-protocol, shared-types, backend-remote-host, provider-common, runner-protocol, host-interface, web-api-protocol, backend-permissions, backend-plugins, plugin-interface
backend -> backend-accounts, backend-blobs, backend-cloud, backend-database, backend-host-access, backend-http, backend-providers, backend-runners, backend-user-shard, command-declarations, command-package-browser-protocol, plugin-browser, plugin-changes, plugin-file, plugin-file-browser, plugin-interface, plugin-mobile, provider-anthropic-api, provider-claude-code, provider-codex, provider-bytedance, provider-common, provider-elevenlabs, provider-minimax, provider-google, provider-grok-build, provider-openai-api, runner-protocol, shared-artifacts, shared-cli, shared-gates, shared-types, web-api-protocol, plugin-skills, plugin-web, plugin-image, plugin-video, plugin-audio
machine-manager -> shared-artifacts, shared-cli, machine-manager-protocol, runner-protocol
server -> backend-database, machine-manager-protocol, runner-protocol, shared-artifacts, shared-cli
runner -> command-protocol, command-sdk, runner-direct, runner-host, runner-jobs, runner-process, runner-protocol, runner-command-packages, runner-shell, shared-artifacts, shared-cli
command-package-file -> shared-artifacts, command-protocol, command-sdk, shared-types, command-package-file-protocol, shared-gates, shared-cli
command-package-fetch -> command-protocol, command-sdk, command-package-fetch-protocol, shared-cli, shared-types, shared-artifacts
command-package-browser -> command-package-browser-chrome, command-package-browser-protocol, command-protocol, command-sdk, shared-cli
command-package-claude-code -> command-package-claude-code-protocol, command-protocol, command-sdk, shared-cli
command-package-mobile -> command-package-mobile-protocol, command-protocol, command-sdk, live-view, live-view-protocol, shared-artifacts, shared-cli, shared-types
xtask -> conversation-socket-protocol, shared-artifacts, command-package-browser-protocol, command-package-mobile-protocol, live-view-protocol, command-package-claude-code-protocol, command-protocol, shared-types, command-package-file-protocol, command-package-fetch-protocol, machine-manager-protocol, runner-protocol, web-api-protocol, backend, backend-providers, provider-common, plugin-interface
```

### TypeScript packages

A line names a workspace package by its name without the `@demicodes/` scope.
The following graph lists the TypeScript packages:

```text
protocol -> none
utils -> none
conversation-client -> protocol, utils
web-ui -> conversation-client, protocol, utils
plugin-sdk -> web-ui
plugin-browser -> plugin-sdk, utils
plugin-mobile -> plugin-sdk, utils
plugin-changes -> plugin-sdk
plugin-file-browser -> plugin-sdk
plugin-skills -> plugin-sdk
plugin-web -> plugin-sdk
web -> conversation-client, plugin-browser, plugin-changes, plugin-file-browser, plugin-mobile, plugin-sdk, plugin-skills, plugin-web, protocol, utils, web-ui
browse -> web
web-gallery -> conversation-client, plugin-browser, plugin-changes, plugin-file-browser, plugin-mobile, plugin-sdk, plugin-skills, plugin-web, protocol, utils, web-ui
```
