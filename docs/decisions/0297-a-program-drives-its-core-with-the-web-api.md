---
status: proposed
---

# A program drives its core with the web API, over HTTP/2 on stdio

## Problem

A program that runs a core of its own, such as a bot that answers in a
group chat, needs everything a page does: start a conversation, send a
message, follow the transcript as it streams, answer a permission request,
upload an attachment, add a provider account and sign in to it. Today all
of it is the server's web API, HTTP routes and the page connection's
WebSocket behind a session cookie. A program that runs no server has no
way to any of it.

A second API made for programs would be a second list of operations, types
and refusals beside the web API's, and every feature would land in one and
wait for the other.

## Decision

The core binary speaks the web API's core routes, as HTTP/2 over its own
standard input and output. A program reads and changes exactly what a page
does, with the same paths, the same types, the same refusals and the same
page connection:

```text
program                                    demi-core --data <dir>
  HTTP/2 over the child's stdin and stdout, cleartext, no listener
  POST /api/conversations --------------->
                            <------------- 201 { conversation }
  PUT /api/conversations/x/attachments --> (bytes, streamed)
  CONNECT /api/connect, :protocol websocket
    open lane 1 { conversation x } ------>
                            <------------- opened, transcript patches
    open lane 2 { starter } ------------->
                            <------------- notify { turn ended }
    { shown } --------------------------->
```

- The program starts `demi-core --data <dir>` and opens an HTTP/2
  connection on the child's pipes with prior knowledge, as `h2c` does on a
  socket. Requests run at once, each with its own flow control, so a large
  download never holds a small read back, and a body that is bytes is
  bytes.
- The page connection (`/api/connect`) runs as a WebSocket over HTTP/2's
  extended CONNECT (RFC 8441), with its lanes as they are: the
  synchronization channel, a conversation's lane, a file watch, a user
  stream.
- What the core asks of the program, its `Starter`, comes on a lane of its
  own, `starter`: a notification of the user, which the program answers
  with how it was shown.
- The core logs to its standard error. The connection's end ends the core:
  it closes as the server closes a user's core.
- A second core on the same data directory is refused: the first holds an
  exclusive lock on `<data>/core.lock` while it runs.

### One set of routes, two ways in

The core's routes move out of `backend-http` into a crate of their own,
`core-api`: an axum router whose handlers reach the core through `Cores`
and take the principal from the request. The server mounts it under
`/api` behind its session gate, which puts the signed-in user in the
request. The binary serves the same router on its pipes, and puts its
principal in each request.

```text
backend-http: accounts, sessions, users, the server's upgrade, public
              shares, push, pairing, installers
   └── mounts core-api behind the session gate
demi-core:    HTTP/2 on stdio ── core-api, the same router
core-api:     conversations, providers and accounts, devices, workspaces,
              schedules, plugins, files, changes, search, usage, settings,
              the product state, the page connection and its lanes
```

### The product state and its channel

A page reads the product state once and follows its changes on the
synchronization channel. Both move into `core-api` for the core's parts:
conversations, workspaces, devices, providers, settings. The server adds
its own parts, such as the users and the server's upgrade, to the same
read and the same channel. A program follows its core's changes as a page
does.

### What the starter gives, from a program

| `Starter` | A program's core |
| --- | --- |
| `notify` | asks the program on the `starter` lane |
| `principal` | the core's one principal, made with its data directory, who configures its own entries |
| `cloud`, `pair`, `install_commands` | none: a program's core runs no Cloud, and its runners connect as the next record says |
| `conversation_shares` | none |

## Spike

A binary served an axum 0.8 router, with the same features this project
builds axum with, on HTTP/2 over its stdin and stdout, and a client in
Node.js 24, Bun 1.4 and Deno each started it and spoke `node:http2` over
the child's pipes (`createConnection`):

| What ran | Node.js | Bun | Deno |
| --- | --- | --- | --- |
| 2 GB download | 823 MB/s | 677 MB/s | 845 MB/s |
| A small read during it | 0.2 to 0.9 ms | 0.2 to 0.6 ms | 0.2 to 0.5 ms |
| 50 MB upload | 77 ms | 116 ms | 82 ms |
| WebSocket by extended CONNECT, text and 3 MB binary echoed | yes | yes | yes |
| A small read during the 3 MB WebSocket message | 1.4 ms | 0.8 ms | 0.6 ms |

The server enables extended CONNECT (`enable_connect_protocol`); axum's
`WebSocketUpgrade` takes the CONNECT as it takes an HTTP/1.1 upgrade.
Node's `http2` has no WebSocket of its own over a stream, so the SDK
frames WebSocket messages on the CONNECT stream itself.

Not run: Windows, where Tokio reads the standard input on a blocking
thread.

## Prior art

- **Docker Engine API**: the CLI, Docker Desktop and every SDK speak one
  HTTP API, over a Unix socket on the local machine.
- **Tailscale LocalAPI**: the CLI and the GUIs of each platform speak the
  daemon's HTTP API over a Unix socket, or a loopback port with a password
  where there is none.
- **HashiCorp go-plugin**: the host starts the plugin as a child and speaks
  gRPC, which is HTTP/2, over a Unix socket the child names on its
  standard output. `stdiogrpc` runs the same over the child's stdio.
- **Codex app server** and the **Agent Client Protocol**: JSON-RPC over
  stdio with methods of their own; Codex's own editor extension is a client
  of it, so its programs and its interface share one API too.

Demi departs from Codex and ACP in its wire, not in the principle: its
interface's API is HTTP, so a program gets that API, and HTTP/2 carries
bytes and concurrent streams that one line of JSON at a time cannot.

## Alternatives

- **JSON-RPC over stdio, as Codex and ACP**: a second set of methods, or
  the web API wrapped in an envelope no one else uses; bytes as base64,
  and a large message holds every message behind it.
- **MessagePack-RPC, as Neovim**: bytes travel as bytes, but a large
  message still holds the pipe; cutting and pacing it is HTTP/2 built
  again.
- **The web API on a Unix socket or a loopback port, as Docker and
  Tailscale**: HTTP/1.1 and WebSockets with no change, but a socket file
  or a port, with its permissions or its password, for every program.
  This is the fallback if a platform's pipes cannot carry HTTP/2.

## Open questions

- None.
