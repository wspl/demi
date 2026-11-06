---
status: proposed
---

# One core runs Demi's agents, in the server and in any program

## Problem

A team builds a chat bot. Each group chat is a conversation; each person who
speaks has the agent work in a sandbox of their own; the agent has
subagents, compaction, a browser and a pool of subscription accounts. Demi
does all of that, but only as a whole server:

- The agent server (`crates/agent-server`) knows no product, yet everything
  it needs lives in backend crates bound to accounts and HTTP: the
  conversation store in `backend-database`, beside the accounts' records;
  the runner connections in `backend-runners`, beside pairing, installer
  scripts and release publication; the credential vault and account
  rotation in `backend-providers`; the conversations, schedules, plugins and
  media in `backend-user-shard`.
- A runner connects to a backend and nothing else. A sandbox the bot starts
  can be a Host only through a paired device of a server.
- So the bot either runs a Demi server and drives it as a client, which
  turns every rule of its own (who a turn works for, which node of the bot
  serves a group, what each turn carries) into an API Demi must offer, or
  it builds the agent server again.

## Decision

Everything Demi does for one person's agents is one **core**, a Rust
library. The server runs one core per user. Any program runs cores of its
own through a binary that speaks the web API over stdio, and a TypeScript SDK
wraps that binary. Both run the same code.

```text
a program (TypeScript)                    Demi's server
  @demicodes/sdk                            accounts, sessions, pages,
    │ the web API, HTTP/2 over stdio        pairing, Clouds, shares, push
    ▼                                         │ in process, one per user
  demi-core binary ──────┐                    ▼
                         ├──────────▶  the core library
                         │               conversations and agent trees,
                         │               Hosts, plugins, providers and
                         │               accounts, schedules, media, usage
                         │                    │ the runner wire
                         ▼                    ▼
              runners: one the core starts on its own machine, one in a
              sandbox or container, one on another computer
```

### What a core holds

A core is one principal's world, what a user shard holds today:

- conversations, their agent trees (`agent-server`), compaction, steers and
  queues, forks and schedules;
- Hosts: the runners connected to it, with jobs, files, transfers, watches
  and shell environments;
- the plugin host (`plugin-host`) and the one `ProductPort` it asks, so
  every plugin, the browser, files, media, mobile and the rest, works
  wherever a core runs;
- provider entries, their accounts and sign-ins, rotation, model catalogs
  and context limits, and media generation;
- attachments and media, the usage ledger and the search index;
- permission requests, which it asks of whoever runs it.

What stays above it, in the server alone:

- accounts, web sessions, passkeys, email and push subscriptions;
- pairing codes and installer scripts, which give a device to a user's core;
- the Cloud's machine, which the server wakes and stops; to the core a Cloud
  is a Host like any other;
- what crosses users: shares, a provider entry lent to another server, and
  which cores a shared instance's vault reads, so each user infers with the
  master's entries;
- the HTTP listener, the session gate and the routes of what is above the
  core. The core's routes, with its product state and its synchronization
  channel, serve the server's pages and a program alike (0297).

### Where a core keeps its data

Whoever starts a core names its data directory, and the core owns it:

```text
<data>/
  core.sqlite                  conversations' index, Hosts, workspaces and
                               projects, preferences, profiles,
                               instructions, plugins, credentials, provider
                               entries and accounts, schedules, usage
  conversations/<id>.sqlite    one agent tree each, as today
  search.sqlite                derived, rebuilt when missing
  blobs/                       attachments and media, unless the starter
                               gives an object store
```

- A program reads and changes what is there through the core: a
  transcript's page, a conversation's list, an account's sign-in. It never
  implements the store.
- A conversation is open in one core at a time. A second core that opens
  the same directory is refused, so two processes of a program never write
  one conversation.
- Credentials are sealed under a key the starter gives: the server's
  instance secret, as today, or a secret in the core's own directory
  (0297).

The server keeps `users/<user>/` as each user's data directory.
`control.sqlite` keeps only what is above the core. The upgrade that brings
this moves each user's records out of `control.sqlite` into their
`core.sqlite`, and `conversations/` and `search/` into their directory, with
the upgrade page's progress. The blobs stay in the server's object store,
which keeps each user's apart already and which S3 cannot move cheaply: the
server gives each core its user's part of the store.

### What the starter gives a core

| Given | The server gives | A program gives |
| --- | --- | --- |
| Data directory and sealing key | `users/<user>/`, the instance secret's key | A directory for each core, with its own secret |
| Answers to permission requests | The user, on the page | Its code, or always allow |
| Notifications | Push and the page | Its code |
| Commands and context of its own | None | Served over stdio, as plugin commands are served |
| The services its cores share | One set for every user's core: the vault over each user's store, the model catalogs, account rotation | One set for the cores it opens |
| Waking a Host that sleeps | The Cloud's machine | None |

### Runners connect to a core

A runner connects to a core with a token the core issues. The server
issues one at pairing and hands the runner's socket to the user's core, as
it adopts it today. A program asks its core for a Host and starts the
runner where it wants its agents' commands to run. How a runner reaches a
core without a server is the subject of 0298.

### The series

Each record lands as its own work package, and each leaves Demi working as
before:

1. This record: the core's boundary and data directory. The backend crates
   that hold the core's parts become core crates, the server runs one core
   per user, and the data moves.
2. 0297: a program opens cores in a process of its own and drives them
   with the web API's core routes and the page connection, over HTTP/2 on
   stdio.
3. 0298: a program's runners connect to its process, which serves them as
   the server does.
4. 0299: a program's plugins run in the program, over a lane of each
   core's connection.
5. 0300: a command declares what its output holds and what a failure
   leaves, which the SDK's commands declare too.
6. 0301: the TypeScript SDK, over the web app's own clients, with the core
   and runner binaries of its build.
7. 0302: a program's page shows a conversation and its work panel with
   the web app's own views.

## Spike

- The upgrade's move of a server's records into per-user directories ran
  on a copy of a server's data, about 200 MB of `control.sqlite` and 140
  conversations: 5.5 seconds in a debug build, every table's count the same
  before and after, and `control.sqlite` left at 180 KB once vacuumed.
- One core under 300 conversations at once, the load 0294 ran on a user
  shard, which the core now is: still to run.

## Prior art

- **Codex** ([openai/codex](https://github.com/openai/codex),
  `codex-rs/`): `codex-core` is the agent library. `codex app-server` and
  `codex exec` are its fronts, the app server speaking JSON-RPC over stdio
  to the VS Code extension and any client; the TypeScript SDK
  (`sdk/typescript`) starts the bundled binary. Its data lives in
  `CODEX_HOME` (`codex-home`), which the starter can name.
  `codex-exec-server` runs processes and files on another machine; an app
  server adds it as an environment and reaches it with a bearer token, as
  a core will reach a runner.
- **Claude Agent SDK**: the TypeScript and Python packages start the
  bundled Claude Code, which keeps its sessions in its own directory.
- **OpenClaw**: a messaging gateway that embeds pi's agent session
  (`pi-agent-core`) in its own process and owns the sandboxes and channels
  around it.

Demi departs in one place: the server is not a separate product beside the
library. It is the core run once per user, with accounts and pages around
it, so a fix to the agent reaches every user of either at once.

## Alternatives

- **An API on the server for programs** (integrations): every rule of the
  program becomes an API the server must offer, and a program runs a whole
  server to use one agent.
- **A program implements the store through callbacks**: a node's create,
  save, close and restore are each one atomic commit (`agent-store`), which
  a round trip per write to another process cannot keep, and every write
  waits on it.
- **Keep one `control.sqlite` and give the core a store trait over it**:
  two implementations of every record, one per kind of starter.

## Open questions

- None. The core is the crate `core` (`demi-core`), its parts are `core-*`
  crates after Codex's `codex-core`, such as `core-store` and `core-hosts`,
  and the server's side of its users' cores stays `backend-user-shard`.
