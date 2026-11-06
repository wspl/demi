---
status: proposed
---

# A runner reaches its core over a channel whoever runs the core gives

## Problem

Everything a Host does runs on a runner: commands, files, terminals, the
browser, the simulators, port forwards and a provider's requests through a
device. A runner reaches its core one way: it dials the server's URL with
its device token, or, in a Cloud, the server's runner socket mounted into
its machine (`Backend` in `runner-process`). Whoever runs a core has no
say in the channel.

A program that runs its agents in its own sandboxes, such as a bot that
gives each user a Docker container, reaches them already: `docker exec`,
an SSH session, its own gateway's connection. To run Demi's runner there,
it would have to open the container's network to the core, or mount a
socket into it, because the runner can only dial.

## Decision

A runner can run over a byte stream it is given, its standard input and
output, and whoever runs the core carries that stream to the core however
it reaches the runner. Dialing a URL stays the server's way.

```text
program                                         demi-core
  docker exec -i box-7 demi-runner run --stdio
     runner's stdio <---- bytes ----> CONNECT /cores/{id}/api/devices/{device}/runner
                                        HTTP/2 inside: the runner's routes,
                                        the Host's own channel
```

### The runner

- `demi-runner run --stdio` speaks to its backend over its standard input
  and output, with HTTP/2 inside, as the program speaks to the process
  (0304). Its routes are the ones it uses today: its socket
  (`/api/runner`), its pipe connection, its pipes, offers and commands'
  outputs, its update and the command packages' executables, each a
  request or a WebSocket on the stream.
- `Backend`, the runner's way to its backend, becomes its transport: a URL
  it dials, directly or through a socket, as today, or the stream it was
  given. Its requests and WebSockets go through the transport either way.
- A runner on a stream presents no token: the channel names its device.
- Its standard input and output carry nothing but the stream: the runner
  takes them first of all, and what it prints, and what the programs it
  starts write, goes to its standard error.
- A core hands a runner the executables in its own keeping as a path from
  its root, as it hands pipes and offers, and the runner requests the path
  the way it reaches the core. A signed URL of object storage stays a URL
  on the network. Today a server hands out its own address, which a runner
  on a stream cannot reach.
- When the stream ends, the runner ends. Its jobs go on in their shells'
  processes, which outlive it, and the next runner started in the same
  installation takes them over and lists them in its hello, as after an
  update. The Host is returning meanwhile, as when any runner drops.

### The core

- `POST /cores/{id}/api/devices` creates a Host, with a default name, and
  answers its id.
- `CONNECT /cores/{id}/api/devices/{device}/runner` opens the Host's
  channel: the process serves the runner's routes on it, for that device,
  as the server serves them to a runner whose token names it. A second
  channel for a Host whose runner is connected is refused, as a second
  runner with the same token is (`AlreadyConnected`).
- The runner's routes move out of `backend-http` into `core-api`, which
  both serve: the server to a runner its token names, the process to the
  runner its channel names.
- The process serves the command packages of the release `DEMI_RELEASE`
  names, as the server serves its release's: their descriptors from its
  `commands/`, their executables from its `files/`, on each channel. A
  release in several directories, a list of paths as `PATH` writes one,
  names the packages of all their `commands/`, and serves each executable
  from the directory whose `files/` holds it.

### The server

Nothing changes: its runners dial it with their tokens, pair by codes and
install from its scripts. A program's Hosts have no token, no pairing and
no start command; the web app shows none for them.

## Built on

| What the design uses | Where it is today | New |
| --- | --- | --- |
| The runner's routes | `backend-http`, found by the device token | in `core-api`, found by the token or by the channel |
| The runner's way to its backend | `Backend`: a URL, or a socket in a Cloud | a stream it is given |
| HTTP/2 on a stream, WebSockets by extended CONNECT | 0304's wire | |
| Jobs that outlive their runner | `demi-runner shell`, taken over by the next runner's hello | |
| A runner that drops | the Host returning for a while | |
| One runner for a Host | `AlreadyConnected` | |

## Spike

None: the runner's routes run unchanged on the stream, which carries
HTTP/2 and WebSockets as 0304's spike ran them.

## Prior art

- **VS Code Remote**: an extension may resolve a remote to a channel of
  its own (`ManagedResolvedAuthority`, whose `makeConnection` answers a
  byte channel), over which VS Code speaks to its remote server as over a
  socket. SSH, containers and tunnels are such extensions.
- **The Language Server Protocol**: the client chooses the transport,
  standard input and output, a socket or a pipe; the protocol does not
  change.
- **The MCP SDKs**: a `Transport` the program implements, `start`, `send`
  and `close`, carries the protocol's messages over any channel.

## Alternatives

- **The process listens, and runners dial it.** A sandbox would need a
  network route or a socket mounted in, and the program could not use the
  channel it has.
- **The core starts runners itself, with a command it is given.** The
  core would know of containers and SSH sessions, which only the program
  knows how to reach and when.
- **A channel of messages rather than bytes.** The runner's routes are
  HTTP requests and WebSockets with streamed bodies; a stream of bytes
  carries them as they are, as VS Code's channel does.

## Open questions

- None.
