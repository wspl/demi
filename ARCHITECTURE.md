# Architecture

The map of the code and the rules that hold across it.

- What Demi is, and one conversation end to end: the
  [overview](docs/overview.md).
- What one module decides: its entry comment (`lib.rs`, `main.rs`,
  `index.ts`), next to the code.
- When this file and the code disagree, the code is right and this file is
  fixed.

## Code map

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

Each program's inside is drawn in its entry comment:

| Part | Entry comment |
| --- | --- |
| The backend's layers | `crates/backend/src/lib.rs` |
| A user's shard | `crates/backend-user-shard/src/lib.rs` |
| The runner | `crates/runner/src/main.rs` |
| The machine manager | `crates/machine-manager/src/lib.rs` |
| The plugin contract | `crates/plugin-interface/src/lib.rs` |
| The page API | `packages/plugin-sdk/src/index.ts` |

## Every capability is a plugin

- The agent runtime, the backend and the web app know a capability only
  through the plugin contract (`plugin-interface`).
- A plugin contributes, all as messages:
  - commands;
  - text for the model's context;
  - Host directories;
  - a part of the page.
- Who supplies what:
  - the agent runtime: its own tools, `demi agent` and `demi shell`;
  - the product: the conversation's Host, `demi host` and
    `demi attachment`;
  - plugins: everything else the agent can do.

### State on a Host

A capability that keeps state on a Host is also a command package behind the
runner's narrow port. The browser, for example:

- `command-package-browser-protocol` holds its operation types;
- the `demi.browser` package (`command-package-browser` with `-chrome`)
  carries them out;
- `plugin-browser` declares them as `demi browser` commands;
- its tabs are keyed by the conversation the runner names on every
  invocation, and end with the generic conversation release;
- no browser type or message exists in the runner crates,
  `runner-protocol`, `backend-remote-host`, the agent or the lifecycle.

### Adding a capability

- Add a `plugin-<name>` crate.
- When it keeps Host state, add a command package, with its operation types
  in a contract crate.
- These do not change: the runner, `runner-protocol`, `backend-remote-host`,
  the agent, `backend-plugins` and the lifecycle. A message, adapter or
  lifecycle hook for one tool in them belongs at the plugin contract or the
  runner's port instead.
- Where a shared mechanism goes:
  - answered without knowing who asked or why (a file operation, a
    working-tree listing, a service stream): the runner wire;
  - usable by any plugin (a Host directory): the plugin contract.

## Dependencies point down

- A crate or package never knows the products, adapters, user interfaces,
  providers or machines above it, in production code or in tests.
- A test reaches a fake through the `testing` feature of the crate that owns
  what it fakes. No crate has a dev-dependency on a crate that depends on it,
  except on itself to turn on its own `testing`.
- A crate's public items are the ones its entry comment names; the rest is
  `pub(crate)` or private.
- Inside the backend, each layer knows only those beneath it. A domain that
  needs another domain's state:
  - defines the narrow trait of what it needs (`CloudShard`, `HostShard`,
    `PermissionShard`, `PluginShard`);
  - `backend-user-shard` implements it;
  - no crate below `backend-user-shard` sees `Shard` or `Services`.
- The runner is one executable of several crates, so a change recompiles
  only its part. Only `runner-shell` links the shell and its utilities, and
  nothing below the executable depends on it.

### The graphs

- The [graphs](#dependency-graphs) below are the required crates and
  packages and the edges between them.
- Manifests follow the graphs. When a `Cargo.toml` or `package.json`
  disagrees with its line, either the manifest changes, or the graph changes
  first with the work package that needs it.
- Both graphs stay acyclic; external and vendored crates are outside them.

## Names and layout

Where things live:

- Rust crates: `crates/`.
- TypeScript packages: `packages/`.
- Vendored crates: `vendor/<upstream>/<crate>/`.

Names:

- A crate directory is `<owner>-<part>`. The owners:
  - `backend-`, `runner-`, `machine-manager-`: one program each;
  - `agent-`: the conversation runtime;
  - `provider-`: model providers;
  - `command-`: the command wire;
  - `command-package-<name>-`: one package, released as `demi.<name>`;
  - `host-`, `plugin-`;
  - `shared-`: no single owner.
- A program is named by its owner alone.
- A library's package is `demi-<directory>`; a program's package is its
  executable's name.
- Suffixes:
  - `-protocol`: the types of one wire or data family, and the decode
    function of each boundary that receives them; no IO, no async runtime;
  - `-sdk`: one end of a wire;
  - `-interface`: traits, and nothing that implements them.
- Words whose meaning differs outside stay out of names: `service`,
  `extension`, `edge`, `core`.
- `plugin` names only a plugin, `browser` appears only in
  `command-package-browser`, and `shell` only for the shell.

When to split:

- A separate crate or package needs a reason: independent use,
  distribution, dependency isolation or build isolation.
- Code that changes together is modules of one crate, never split because it
  is long.
- Cargo rebuilds every crate above a changed one, so a split saves build
  time only when the part split off is not above the code that changes:
  - code that changes often belongs above;
  - types many crates read belong below.

Inside a crate:

- One composition root per executable (`main.rs`, `Backend::start`, the web
  app's `main.ts`): it constructs, injects, mounts and returns, and never
  branches on business state.
- Modules follow responsibilities; there is no `misc`, `helpers` or `utils`
  module.
- Generic Rust comes from std or an established crate, generic web code from
  `@demicodes/utils`, and a domain helper sits beside its module.
- Generated TypeScript (`src/generated/` directories) is never committed.
- A vendored crate keeps its upstream metadata and licenses. Its
  `[package.metadata.demi]` names each change from upstream with its reason,
  and `bun xtask vendor diff` compares it with crates.io.

### Test binaries

- Each crate has one test binary, its integration tests, which test the
  public boundary with a module per area.
- The library and every executable set `test = false`; every library sets
  `doctest = false` and `bench = false`. Otherwise `--all-targets` compiles
  each library twice.
- A crate without programs whose behavior is observable only inside keeps
  unit tests and no `tests/`.
- A test gets a binary of its own only when it changes or exhausts
  process-wide state (such as the open-file limit), or needs what others
  must not have (such as root). Its file says so at the top.
- A test reaches another program as the binary Cargo built.

## Contracts

- Every message and stored document that crosses a process boundary is
  defined once, as a Rust type.
- Its JSON Schema, TypeScript type and validator derive from it
  (`bun run contracts`).
- A second declaration of the same shape is a defect, even while the two
  agree.
- The encoding conventions are in `crates/shared-types/src/lib.rs`; the
  generator is in `crates/xtask/src/contracts/`.

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

- The runner wire and the machine manager socket have both ends in Rust and
  fixed encodings, pinned by a golden corpus of every message kind.
- A vendor's API is not a Demi contract: each provider crate declares only
  what it reads.
- A value from outside a process is decoded at its entry, against its type.
  After that nothing checks it again; before that nothing looks inside it.
- Corrupt data is refused, never repaired or defaulted.

## Concurrency

Where work runs:

- All programs use Tokio.
- State one user or one connection owns lives on one single-threaded
  runtime, so it needs no lock and a test's paused clock reaches it:
  - a backend shard thread;
  - the runner's control loop;
  - the machine manager's loop.
- Bytes, and work across users, run in parallel at the edge.
- Each runtime builds its own `reqwest::Client`: a pooled connection's task
  is tied to the runtime that made it.

Sharing:

- Shared state has one owner that answers requests.
- Work that must not overlap across awaits takes a named gate from
  `shared-gates`.
- Read-mostly data is a snapshot published through `watch`.
- No lock guard or `RefCell` borrow is held across an `.await`.

Blocking work:

- CPU work over about a millisecond, and every blocking call, leave async
  threads through `spawn_blocking` or a thread of its own.

Cancellation:

- It follows ownership: a `CancellationToken` tree.
- Guards release on drop, on success, failure and cancellation alike.
- Cancelling stops waits, never a commit: a store save is not raced.
- A release that must wait is an explicit `close()`, with drop as the
  fallback that kills.

### Tests and time

- Timer logic inside one process runs on Tokio's paused clock, so a test
  asserts exact times.
- A test whose timers run while it waits on a real runner, service or
  database uses real time with short windows: a paused clock jumps to the
  next timer whenever the runtime is idle.
- Wall time comes from an injected `Clock`.
- A backend test without a real runner runs the shards on the test's own
  runtime (`ShardPlacement::Inline`).

## Dependency graphs

- Each graph lists every member once, as `name -> dependency, dependency`,
  or `name -> none`.
- The crate check (`crates/xtask/src/boundaries.rs`) and the package check
  (`scripts/__tests__/package-boundaries.test.ts`) read these blocks, and
  fail until every manifest agrees with its line.
- So each stays one fenced `text` block in this form.

### Rust crates

A line names a workspace member by its directory.

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
