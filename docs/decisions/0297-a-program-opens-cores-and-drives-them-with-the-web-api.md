---
status: proposed
---

# A program opens cores in a process of its own and drives them with the web API, over HTTP/2 on stdio

## Problem

A program that runs Demi's agents itself, such as a bot that answers in two
group chats, each with its own conversations and providers, needs
everything a page does: start a conversation, send a message, follow the
transcript as it streams, answer a permission request, upload an
attachment, add a provider account and sign in to it. Today all of it is
the server's web API, HTTP routes and the page connection's WebSocket
behind a session cookie, and a core runs only inside the server, one for
each of its users. A program that runs no server has no way to any of it.

A second API made for programs would be a second list of operations, types
and refusals beside the web API's, and every feature would land in one and
wait for the other.

## Decision

A program starts `demi-core`, a process that runs any number of cores, and
speaks the web API's core routes to it as HTTP/2 over the process's
standard input and output. Each core is a directory the program names; the
process opens and closes cores as the program asks. Under
`/cores/{id}/api`, a program reads and changes exactly what a page does,
with the same paths, types, refusals and page connection.

```text
program                                      demi-core
  HTTP/2 over the child's stdin and stdout, cleartext
  POST /cores { dir: /srv/bot/a } ------->  locks a, opens its core
                               <---------- 201 { id }
  POST /cores/{id}/api/conversations ---->
                               <---------- 201 { conversation, metadata }
  CONNECT /cores/{id}/api/connect, :protocol websocket
    open lane 1 { conversation } -------->
                               <---------- opened, transcript patches
    open lane 2 { starter } ------------->
                               <---------- notify { turn finished }
  DELETE /cores/{id} -------------------->  closes the core, unlocks a
```

The records after this one decide how a program's runners connect to the
process (0298) and how a program's own plugins run in it (0299).

The wire is the SDK's, not a program's: a program calls the TypeScript
SDK, whose own interface a later record of this series decides, and the
SDK speaks this wire with the types and schemas the web app's client is
generated with (`bun run contracts`).

### The process

- The SDK starts `demi-core` with its release's directory, as the server
  is started (`DEMI_RELEASE`); a program sets nothing. The process runs one
  shard thread, as the server does, and the plugins its release builds in.
  It runs on every target the runner is built for.
- The process logs to its standard error. The connection's end closes
  every open core, as the server's stop closes its users' cores, and the
  process exits.

### A core and its directory

`POST /cores { dir }` opens the core whose data is in `dir`, creating it
when `dir` is absent or empty, and answers its id. `DELETE /cores/{id}`
closes it.

```text
<dir>
+-- core.lock                  held while a process has the core open
+-- principal                  the core's principal id, written at creation
+-- core-secret                32 random bytes, owner-only, written at creation
+-- core.sqlite                the core's records
+-- conversations/<id>.sqlite  one agent tree per conversation
+-- search.sqlite              the search index, derived
+-- blobs/                     the core's blobs
```

- The id is the core's principal, the `UserId` its credentials are sealed
  with, made at creation and kept in `principal`, so a directory opens under
  the same id in any process. While a directory is open, a second open of
  it, or of a copy of it, answers 409.
- `core-secret` is to a core what `instance-secret` is to the server: the
  core's keys derive from it, so the directory alone reads its
  credentials. A server's cores keep using the instance secret.
- The blobs live in the directory. A server's cores keep theirs in the
  server's object store, under `blobs/<user>`, which may be S3.
- An open does for its core what the server's start does for each of its
  cores before it serves: it migrates the core's databases, recovers the
  forks, finishes the pending deletions, withdraws the credential requests
  a stop cut off and expires the credentials that are due, delivers the
  decisions a stop cut off, resumes the interrupted turns, arms the idle
  compactions and the schedules, follows the media jobs a vendor took, and
  indexes the conversations for search. Both run the same functions; the
  server's start runs them over all of its users.
- A close is the close the server's stop runs for each core
  (`Core::close`): the running turns are marked, so the next open resumes
  them. A call that reaches a closing core is refused as one that reaches
  a stopping server (`CoreUnavailable::Closing`).
- A core runs only while it is open. Its schedules fire what they missed
  at its next open, as the server's do after a stop.
- A directory that holds `core.sqlite` and no `principal`, such as a
  server user's, answers 409: the process opens only cores a program
  created.

### The principal

The core knows its principal by id alone. No code of the core reads a
role: the role decides only the server's parts of the product state, the
accounts, the Cloud's isolation and the server's upgrade. So
`Starter::principal` goes, and the provider settings take the principal's
`UserId` instead of a `UserDto`.

### Providers

Each core has its own provider entries and accounts: a program's process
is an isolated instance (`InstanceMode::Isolated`), and has no switch to
share one core's entries with another. Rotation, sign-ins and usage are the
core's, through the routes a page uses.

### One set of routes, two ways in

The core's routes move out of `backend-http` into a crate of their own,
`core-api`: an axum router whose handlers take the principal from the
request, reach the core through `Cores`, and reach its store through the
process's one lookup of its cores' stores, which the vault already uses
(`VaultStores::store_of`). A handler that reads a store at the edge today,
as the drafts and the saved messages do, still does: no call moves onto the
core's thread.

The server mounts the router under `/api` behind its session gate, which
puts the signed-in user's id in the request. The process mounts it under
`/cores/{id}/api`, and puts `{id}` in the request.

```text
backend-http: accounts, passkeys, sessions, users, the server's upgrade,
              the instance mode, the Cloud's isolation and network,
              public shares, provider lending, pairing, installers,
              schedule URLs
   `-- mounts core-api under /api, behind the session gate
demi-core:    /cores, and core-api under /cores/{id}/api
core-api:     conversations, drafts, saved messages, permissions,
              providers and accounts, devices, workspaces, projects,
              schedules, plugins, files, changes, search, cost, usage,
              preferences, attachments and blobs, the product state,
              the page connection and its lanes; and with no session,
              the File view's HTML files and the plugins' files
```

A page's sandboxed frames load two kinds of files with no session: an HTML
file the File view shows, by a token in its path, and a file a plugin's
page loads, such as a library its document runs. Both move into
`core-api`, and a program's page gets them through the program, as it
gets the rest. A token of the File view lasts while its core is open in a
program's process, and while its session lasts on the server, as today.

### The product state and its channel

A page reads the product state once and follows its changes on the
synchronization channel. Both move into `core-api` for the core's parts:
the conversations and their order, the preferences, workspaces, projects,
devices, providers, sign-ins, models, subagents, instructions,
credentials, port forwards and plugins. The server adds its own parts, the
user, passkeys, sessions, users, the Cloud and its isolation, the server's
upgrade and the instance mode, to the same read and the same channel. A
program follows its core's changes as a page does.

### What the starter gives, from a program

| `Starter` | A program's core |
| --- | --- |
| `notify` | sends the notification on the `starter` lane; the program answers how it was shown |
| `reach`, `agent_reaches` | one receiver while a `starter` lane is open, none otherwise |
| `cloud`, `cloud_network_limited` | none: a program's core runs no Cloud |
| `pair`, `install_commands`, `device_removed` | the process's, as the next record decides (0298) |
| `host_log` | none |
| `schedule_called_at_url`, `schedule_deleted` | nothing to keep: the process finds a schedule among its open cores (0298) |
| `conversation_deleted`, `provider_deleted` | nothing: the process keeps no record of them |
| `conversation_shares` | none |
| `closing` | ends the core's `starter` lane |

What needs an address that a browser or another system reaches, a
plugin's OAuth callback, a schedule's URL and a device's address, uses the
public URL the program gives (0298), as the server's use its own.

## Built on

| What the design uses | Where it is today | New |
| --- | --- | --- |
| The web API's routes, their types and refusals | `backend-http`, `web-api-protocol` | moved into `core-api` |
| The principal in the request | `AuthUser`, the session gate | the process's `/cores/{id}` |
| The page connection, its lanes, binary messages and credit | `/api/connect`, `shared-lanes` | |
| Opening a core | the pool opens a user's core on its first call | `POST /cores` |
| Closing one core | `Core::close`, run for every core at the server's stop | `DELETE /cores/{id}` |
| The work an open takes up | the server's start passes over all cores | run for one core at its open |
| A core's directory | `users/<id>/` under the server's data directory | `principal`, `core-secret`, `blobs/` |
| The sealing key | one instance key per process | one key per core |
| The blobs' place | the object store, `blobs/<user>` | the core's `blobs/` |
| The provider entries | the vault, isolated or shared | isolated only |
| The product state and its channel | `backend-user-shard/src/sync/` | moved into `core-api`, server parts added by the server |
| What the core asks of whoever runs it | `Starter` | the `starter` lane; `principal` removed |
| The files a page's frames load with no session | public routes of `backend-http` | moved into `core-api` |

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

The spike ran on macOS. The process runs on every target the runner is
built for: macOS, Linux and Windows, each on x86-64 and ARM64. Tokio reads
the standard input and writes the standard output on a blocking thread on
every platform, so the spike measured the same reads and writes Linux and
Windows run. On Windows, Tokio also cuts a long write short at a UTF-8
boundary; HTTP/2's writer writes the rest, as it does after any short
write.

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

- **One core per process, its directory a flag.** A program that serves
  two chats would run and supervise two processes, and the process's
  services, the provider catalog and the plugin registry, twice.
- **One key per process, which the program keeps.** A directory would not
  open without a secret kept elsewhere, and a lost secret would lose every
  core's credentials at once.
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
