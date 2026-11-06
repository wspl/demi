---
status: proposed
---

# A program drives its core with the web API, over stdio

## Problem

A program that runs a core of its own, such as a bot that answers in a
group chat, needs everything a page does: start a conversation, send a
message, follow the transcript as it streams, answer a permission request,
add a provider account and sign in to it. Today all of it is the server's
web API, HTTP routes and two WebSockets behind a session cookie. A program
that runs no server has no way to any of it.

A second API made for programs would be a second list of operations, types
and refusals beside the web API's, and every feature would land in one and
wait for the other.

## Decision

The core binary, `demi-core`, speaks the web API's core routes over its
standard input and output. A program reads and changes exactly what a page
does, with the same paths, the same types and the same refusals:

```text
program                                   demi-core --data <dir>
  initialize { principal } -------------->
                            <------------- { version }
  request { POST /conversations } ------->
                            <------------- { status: 201, body: {..} }
  lane/open { 1, conversation x } ------->
                            <------------- { opened }
  lane/message { 1, open {..} } --------->
                            <------------- lane/message { 1, opened }
                            <------------- lane/message { 1, patch }
                            <------------- notify { turn ended }, a request
  { shown } ----------------------------->
```

### The wire

JSON-RPC 2.0, one message per line, as Codex's app server and the Agent
Client Protocol do. The core logs to its standard error.

| Method | From | Params | Result |
| --- | --- | --- | --- |
| `initialize` | program | the principal the core serves | the core's version |
| `request` | program | `method`, `path` with its query, `body` | `status`, `body` |
| `lane/open` | program | lane number, what it carries (`LaneTarget`) | opened, or the refusal (`LaneRefusal`) |
| `lane/message` | either, a notification | lane, one message | |
| `lane/close` | either, a notification | lane, code, reason | |
| `notify` | core | a notification of the user | how it was shown |

- The lanes are the page connection's (`/api/connect`): the
  synchronization channel, a conversation's lane, a file watch, a user
  stream. Each carries the messages it carries on a page's connection; the
  pipe paces them, so a lane has no credit and a message is not cut into
  pieces.
- A body is the route's JSON; a body that is bytes, such as a blob or a
  file the File view reads, travels as base64 in `bytes`, as MCP and ACP
  carry an image.
- A refusal is the route's own: its status and its `ApiError` body.

### One set of routes, two ways in

The core's routes move out of `backend-http` into a crate of their own,
`core-api`: an axum router whose handlers reach the core through `Cores`
and take the principal from the request. The server mounts it under
`/api` behind its session gate, which puts the signed-in user in the
request. The binary puts its principal in each request and calls the same
router with no listener, through tower's `Service`.

```text
backend-http: /api/auth, /api/users, /api/server-upgrade, shares, installers
   └── mounts core-api behind the session gate
demi-core binary: stdio JSON-RPC ── core-api, the same router
core-api: conversations, providers and accounts, devices, workspaces,
          schedules, plugins, files, changes, search, usage, settings,
          the product state and its channel, the conversation lane
```

The routes the server alone serves stay in `backend-http`: accounts,
sessions and passkeys, users, the server's upgrade, public shares, push
subscriptions, pairing and the installer scripts.

### The product state and its channel

A page reads the product state once and follows its changes on the
synchronization channel. Both move into `core-api` for the core's parts:
conversations, workspaces, devices, providers, settings. The server adds
its own parts, such as the users and the server's upgrade, to the same
read and the same channel. A program follows its core's changes as a page
does.

### What the starter gives, over stdio

| `Starter` | A program's core |
| --- | --- |
| `notify` | asks the program (`notify`) |
| `principal` | the principal `initialize` gave |
| `cloud`, `pair`, `install_commands` | none: the program's core runs no Cloud, and its runners connect as the next record says |
| `conversation_shares` | none |

## Spike

None: the router runs unchanged under both, which the server's tests and a
program's test cover.

## Prior art

- **Codex app server** (`codex-rs/app-server`, `app-server-protocol`):
  JSON-RPC over stdio with methods of its own (`thread/start`,
  `turn/start`), separate from any web API; its TypeScript types are
  generated from the Rust types.
- **Agent Client Protocol** (Zed): JSON-RPC over stdio, `session/new`,
  `session/prompt` and `session/update`, and requests the agent sends the
  editor, such as `session/request_permission`.
- **Electron and Tauri apps** call their own backend's HTTP API through a
  custom transport, so the page's client code is the same.

Demi departs from Codex and ACP in carrying its web API rather than a
second set of methods: the page is the reference client, and a program
gets every feature the day the page does.

## Alternatives

- **Methods of its own, as Codex's app server**: a second list of
  operations, types and refusals to keep with the web API's.
- **The web API on a loopback port**: the same routes with no envelope,
  but a port and a token for every program, and an open port on the
  machine while it runs.

## Open questions

- None.
