---
status: proposed
---

# A program's runners connect to its process, which serves them as the server does

## Problem

Everything a Host does runs on a runner: commands, files, terminals, the
browser, the simulators, port forwards and a provider's requests through a
device. A runner reaches only a server. Its socket, its pipes, its offers
and its commands' outputs are the server's routes; the server pairs it,
serves its installers, its releases and the executables of its command
packages, and finds its device by its token in the server's control
database. A core in a program's process has no Host, so its agents run no
command.

## Decision

A runner connects to a program's process as it connects to a server: the
same routes, the same device token, the same pairing. The process serves
the runner routes the server serves, on a Unix socket of its own, and the
program serves them to runners elsewhere through its own HTTP server, as
it serves the core's routes to its pages.

```text
runner on the program's machine   runner in the program's container
  `-- the process's Unix socket <--' (its directory mounted, as a
                 |                     Cloud's socket is mounted)
             demi-core  <-- HTTP/2 on stdio -- program
                 ^                                 ^
                 `------ forwarded by the program -- runner on another
                         (demi.fetch, demi.accept)    computer, paired
```

### The routes

The runner routes move out of `backend-http` into `core-api`, beside the
core's routes, as one router both mount:

- the runner's socket, `/api/runner`, and its pipe connection,
  `/api/runner/pipes`;
- its pipes, offers and commands' outputs: `/api/pipes/{id}`,
  `/api/offers/{id}`, `/api/call-outputs/{id}`;
- its downloads: `/install.sh`, `/install.ps1`, `/runner-artifacts/...`
  and the command packages' executables;
- the schedules' URLs, `/hooks/{id}`, which carry no session either.

The server mounts them where it does today: on its listener and on its
runner socket. The process mounts them on its Unix socket and on its
stdio, so the program forwards a request it takes for them to the process.
The process serves the runner releases and the command packages of its own
release, as the server does from `DEMI_RELEASE`.

### How a request finds its core

A schedule's URL names the schedule, and its core checks the secret. A
runner's device works the same way from now on:

- A runner names its device and presents its token, in its hello and in
  the header of each pipe request (`demi-runner-device` beside
  `demi-runner-token`).
- Whoever runs the cores finds the core that holds the device or the
  schedule: the server in its control database, as it finds a schedule's
  core today; the process among its open cores.
- The core checks the token against the hash it keeps with the device.

So a device's token hash moves from `control.sqlite` into the core's
`devices` table, and the server's `device_owners` keeps only which user
holds each device, as `schedule_owners` keeps which user holds each
schedule. Both moves happen in the upgrade that splits each user's records
out of `control.sqlite` (0296), which no release has shipped yet. A runner
of an earlier release updates before its socket opens, as the server's 409
already makes it, so every hello the new server reads names its device.

### The Hosts a program gets

- **A Host it starts.** `POST /cores/{id}/api/devices` creates a Host and
  answers the record its runner boots with (`ManagedBoot`): the device, its
  token, and the process's socket. The program starts `demi-runner run
  --managed-boot <file>` where it wants the agents' commands to run, on its
  own machine or in a container it mounts the socket's directory into. The
  SDK does this for a Host on the program's machine in one call.
- **A computer someone pairs.** A runner started with the program's public
  URL waits under the code it prints, as with a server, and `demi host
  pair` or the program's page claims it. The waiting runners move from the
  server's state into `core-api`, which both serve.

The process starts no runner by itself: the program decides where its
agents' commands run.

### The public URL

What a browser or another system reaches, a plugin's OAuth callback, a
schedule's URL, the installers and a paired runner's connection, is at the
public URL the program gives the SDK: where its HTTP server forwards the
core's routes. Without one, none of them is offered: the core already
leaves each out while it has no public URL.

### What a runner and its records meet

- `Hello` gains `device`, the device's id, which the runner keeps from its
  claim or its boot record; `ManagedBoot` gains `device` and the socket's
  path, for a runner outside a machine the machine manager starts.
- The pipe routes read `demi-runner-device` beside the token.
- `device_owners` loses `token_hash`; the core's `devices` gains it.

## Built on

| What the design uses | Where it is today | New |
| --- | --- | --- |
| The runner's routes and downloads | `backend-http`: `runners_and_downloads`, the pipe routes, `hooks` | moved into `core-api` |
| A runner on a Unix socket | the server's runner socket, which the machine manager mounts into a Cloud | the process's socket |
| A Host born with its token | a Cloud's `ManagedBoot` | the program's Hosts |
| Pairing by a code | the server's waiting runners and claims | moved into `core-api` |
| Finding a core by an id, the core checking the secret | a schedule's URL (`schedule_owners`) | devices follow it |
| A runner's update before its socket opens | the 409 with the required release | |
| Several backends on one runner | a registration for each backend URL | |
| The public URL | `DEMI_BACKEND_PUBLIC_URL`, optional in the core | given by the program |

## Spike

Not run yet, and run before this record is accepted:

- **A runner through the program.** A runner on another computer pairs and
  runs a command through a program's HTTP server that forwards to the
  process (`demi.accept`), with a file of 100 MB moved through a pipe.
- **A runner in a container.** A runner boots in a container with the
  process's socket directory mounted, from a record the process made.

## Prior art

- **GitHub Actions self-hosted runners**: a runner registers with a token
  the repository's settings issue, keeps its own credentials, and polls
  the service; an organization or an enterprise is only another URL to
  register with.
- **Docker**: the CLI reaches the daemon on its Unix socket, and a
  container that should reach it has the socket mounted.

## Alternatives

- **A listener of the process on a TCP port.** A port and its address for
  every program, beside the HTTP server the program runs anyway for its
  pages.
- **Each core at a path of its own, `/cores/{id}/api/runner`.** A runner's
  backend URL would name the core, but its pipe requests use absolute
  paths at the origin, and a schedule's URL already routes by its id.
- **The token hash kept by whoever runs the core.** A program's process
  has no database of its own; its records would be a file in each core's
  directory beside the core's own.
- **The process starts a runner on its own machine.** A program whose
  agents must never run on its own machine would have to stop it.

## Open questions

- None.
