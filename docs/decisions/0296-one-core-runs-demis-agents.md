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
library. The server runs one core per user. Any program runs a core of its
own through a binary that speaks JSON-RPC over stdio, and a TypeScript SDK
wraps that binary. Both run the same code.

```text
a program (TypeScript)                    Demi's server
  @demicodes/sdk                            accounts, sessions, pages,
    │ JSON-RPC over stdio                   pairing, Clouds, shares, push
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
  the master's entries a shared instance's users infer with, which the
  server lends to each user's core through the relay's borrowing
  (`provider-relay`), as another server's are lent today;
- the pages' synchronization channels and the HTTP routes.

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
- Credentials are sealed under a key the starter gives, as the server's
  instance secret seals them today.

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
| Data directory and sealing key | `users/<user>/`, the instance secret's key | Its own |
| Answers to permission requests | The user, on the page | Its code, or always allow |
| Notifications | Push and the page | Its code |
| Commands and context of its own | None | Served over stdio, as plugin commands are served |
| Entries lent to the core | The master's, on a shared instance | Its own, if any |
| Waking a Host that sleeps | The Cloud's machine | None |

### Runners connect to a core

A runner connects to a core with a token the core issues. The server
issues one at pairing and hands the runner's socket to the user's core, as
it adopts it today. A program asks its core for a token and gives it to the
runner it starts in a sandbox; the core starts a runner on its own machine
itself. How a runner reaches a core without a server is the subject of the
next record in this series.

### The series

Each record lands as its own work package, and each leaves Demi working as
before:

1. This record: the core's boundary and data directory. The backend crates
   that hold the core's parts become core crates, the server runs one core
   per user, and the data moves.
2. The core's protocol over stdio: JSON-RPC with the reads a page makes
   today over HTTP, the conversation lane's frames, and the requests the
   core sends back (commands, context, permissions).
3. Runners connect to a core directly.
4. The TypeScript SDK: the types made from the protocol's schema, and the
   core and runner binaries of one build version in one package.
5. The web app's conversation client over any transport, so its components
   run on a program's page through the program's own connection.

## Spike

None yet. The build measures:

- The upgrade's move of a server's records into per-user directories, on a
  copy of a server's data, for its time and its peak memory.
- One core under 300 conversations at once, the load 0294 ran on a user
  shard, which the core now is.

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

- The core crates' names: `core-*` after Codex's `codex-core`, such as
  `core-store` and `core-hosts`, beside the agent crates that keep theirs.
