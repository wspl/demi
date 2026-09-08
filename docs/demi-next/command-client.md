# Demi Next: Native Command Client and Local IPC

| | |
|---|---|
| Date | 2026-09-08 |
| Status | Implemented for macOS/Linux; Windows platform acceptance remains open |
| Scope | Separate command client and runner, command dispatch, IPC discovery and access |

## Executables and responsibilities

`demi` and `demi-runner` are separate executables with separate entrypoints.
The command client does not select a runner mode by its invocation name and
does not point at the runner executable through a symlink.

| Component | Owns | Does not own |
|---|---|---|
| `demi` — C with statically linked libuv | Local endpoint connection, raw arguments and caller context, byte streams, cancellation and exit status | JS runtime, command manifests, argument schemas, help generation, command implementations, backend credentials |
| `demi-runner` — TypeScript bundled with txiki.js | Device connection, Host operations, shell jobs and output, manifest cache, command parsing and dispatch, local execution scheduling, backend RPC forwarding | Agent sessions, provider logic, transcript storage, definitions of business commands |
| Backend and command-defining packages | Command definitions and schemas; backend handlers for conversation/platform state | The native client's transport implementation |

The native source location is `packages/command-client`, outside
the TypeScript runtime dependency graph. It builds the `demi` executable.
The runner remains in `packages/runner`. `@demicodes/command-loader` remains
the single parser/dispatcher, hosted by the runner with injected Host and
RPC dependencies. The native client must not reimplement it.

Third-party root names use the same client transport and identify the root
in the invocation. Packaging those names must resolve to the client, never
to `demi-runner`.

## Runner instances and version ownership

A physical device may run multiple runner processes. Each local backend
registration owns one runner instance, not one runner shared by all
backends. The registration consists of the backend service identity/address
and the paired account/device relationship; a local opaque installation ID
names its state. Different registrations must not share credentials,
manifest caches, jobs, IPC endpoints or upgrade state. One registration
serves all of its projects; projects do not each start a runner.

Each installation owns a matched release of two executables: `demi` and
`demi-runner`. Different backends may install different releases on the
same machine. Their clients must not overwrite one global `demi` binary.
The runner puts its own release's client directory first in the jobs' PATH.
The backend wire, local client wire and command-module ABI are checked for
compatibility at their respective boundaries; incompatible versions fail
explicitly rather than attempting another instance or a compatibility shim.

```text
backend A <-> runner A (release A) --spawn--> job A -> demi A
                  ^                                   |
                  +----- endpoint A + context A -------+

backend B <-> runner B (release B) --spawn--> job B -> demi B
                  ^                                   |
                  +----- endpoint B + context B -------+

Both installations may live on the same physical device.
```

An installation script downloaded from a backend carries that backend's
service address and release selection. It installs/registers only that
backend relationship and starts or reuses that installation's runner.
Repeated installation for the same registration is idempotent; installing
B must not replace A's binaries or backend configuration. Device identifiers
issued by a backend remain scoped to that registration.

A per-installation process lock/service identity prevents duplicate active
runners for that registration. There is no machine-wide singleton lock.
An upgrade affects only its installation: stop admitting jobs, drain the
current invocations, then switch the matched client/runner release and
restart. If draining cannot finish, report that the upgrade is waiting;
do not silently reroute or kill jobs. Recheck compatibility when reconnecting
because a backend may have upgraded independently.

Registration, releases, credentials and caches belong under distinct
installation directories. Runtime sockets may use a shorter protected
runtime directory to fit platform path limits. These installations share
an OS filesystem when run as the same user; routing and version isolation
are not filesystem isolation. For managed Cloud, one backend still manages
one device per user; this does not require separate backends to share a
runner process.

## Command execution

```text
shell -> demi (C client) -> local IPC -> runner command loader
                                         |
                       +-----------------+------------------+
                       |                                    |
                 runtime command                       rpc command
                 target's Host                         backend handler
                       |                                    |
                       +------- output / exit response ------+
                                         |
                                  IPC -> demi -> shell
```

`demi file read notes.md` sends the raw root/arguments and caller context
to the runner. The runner parses the command and schedules its runtime
module against that machine's Host. It returns file bytes through IPC to
the client's stdout. No backend round trip is required for this local work
when the runner has the command manifest cached.

`demi todo add "fix tests"` takes the same client path. After parsing, the
runner forwards the RPC command to the backend under the authenticated
invocation context and streams the result back. `--help` and `--json` are
also interpreted by the loader, not by C command-specific code.

Each invocation carries its own cwd, environment and stream context.
Execution must not change the daemon's process-global cwd or environment.
The runner schedules potentially blocking command scripts in cancellable
execution units rather than allowing a script to block its connection and
job-management loop. The worker/process mechanism is an implementation
decision still to be resolved.

The client requires a running local runner. Backend-dependent commands also
require a backend connection; local commands with a cached manifest do not.
There is no standalone manifest-loading fallback in the native client.

## Endpoint names: finding the correct runner

libuv's pipe API provides UDS on Unix and named pipes on Windows. Every
runner instance opens its own endpoint with a random per-start component:

- Unix: a short socket path in a private runtime directory, scoped to the
  installation and current runner start.
- Windows: a local named pipe such as
  `\\.\pipe\demi-<installation-id>-<random-start-id>`.

Randomness avoids collisions and stale names; the per-installation lock
prevents duplicate processes. Random names are not access controls.

Before starting a job, the runner allocates an opaque execution context
bound to that instance/start, its backend registration, authorized
session/shell/job and manifest version. It injects its exact endpoint and
context ID into the child environment and selects its own client via PATH.
These values override conflicting backend-provided environment entries.
Context-aware raw spawns use the same mechanism. A spawn without an
associated session cannot manufacture session authority by naming an ID.

`demi` forwards the context and raw arguments to that exact endpoint. The
runner validates the live context before selecting its manifest or routing
backend work. Each command invocation receives a separate invocation ID so
concurrent calls within one job have independent streams and cancellation.
A context identifies the execution relationship; it is not a backend URL
supplied by the client. Job completion invalidates its contexts.

There is no search for a convenient live runner and no fallback to another
backend. An endpoint/context mismatch, stale context after restart, or
missing exit reply fails explicitly. For an ordinary terminal invocation,
the user must select/activate an installation and establish an appropriate
context; the CLI must not infer a backend from cwd or the most recently
installed instance.

Installation and job startup handle Unix names and permissions automatically.
The injected fields are `DEMI_RUNNER_ENDPOINT` and `DEMI_CONTEXT_ID`.
Ordinary terminal invocations without a live job/spawn context fail explicitly;
a terminal activation command is not currently exposed.

## Access: who may connect

An endpoint name locates a runner; it does not authorize its caller.

On Unix, the runner creates its state directory with owner-only access and
its socket with owner-only read/write access (0700 and 0600 respectively).
The Windows target design requires a local-only named pipe with an explicit
access-control list for its owning user. A user/instance identifier in the
pipe name prevents collisions but does not replace that access control.
The native client and runner use the same OS account, including the guest
account after managed boot drops privileges.

These controls exclude other ordinary users of the same machine. They do
not distinguish every process running as the same user. Runner/backend
validation must still bind session, shell and job attribution to authorized
live state; client-supplied identifiers alone grant no authority. The client
does not receive the runner's backend device token.

libuv unifies connection and I/O calls, not application endpoint discovery
or all permission policy. In particular, the current runner's unconditional
`chmod` of a Unix socket cannot simply be reused for Windows named pipes.
Keeping the client in JavaScript would not remove this platform work.

## IPC behavior and acceptance

The local protocol must distinguish invocation metadata, input bytes, input
EOF, stdout, stderr, cancellation and final exit status. Preserve binary
bytes and bounded buffering in both directions. Input EOF ends input, not
the invocation; an unexpected disconnect before an exit response is failure.
Cancellation must remain observable even when output is backpressured.
The job's interactive input and redirected command input remain distinct.

The local contract belongs to `@demicodes/runner-protocol`; the C client and
TS runner must be checked against the same fixtures. Boundary decoders must
reject invalid frames. `local-contract.json` defines protocol version, frame tags and limits; C build
code generates its header from that file. Frames contain a four-byte
big-endian body length, a one-byte type, then the body. Metadata is strict
JSON; stream chunks are raw bytes. The maximum body is 1 MiB, with 64 KiB
stream chunks. `invoke` and `watch` include the protocol version. An
invocation opens its control connection after the first `ready`; only the
validated watch starts dispatch. `pull` requests one stdin chunk at a time.
`exit` carries a four-byte status after all output writes. Disconnecting
either connection cancels the invocation.

Acceptance includes concurrent invocations with different cwd/env, binary
pipes and file redirection, help and argument errors, local/runtime and
backend/RPC commands, cancellation under backpressure, runner disconnect,
Unix and Windows endpoint isolation, two backends with different releases,
installation/upgrade independence, stale-context rejection, Windows terminal/Unicode handling,
and separate runnable client/runner artifacts.

## Current implementation and evidence

`packages/command-client` implements the C transport. The runner hosts
`command-loader`; runtime leaves run in separate txiki.js workers so their
cwd/env and CPU execution do not block or mutate the runner's JS context.
Worker termination uses a QuickJS interrupt flag. Backend leaves retain
the existing authenticated RPC and HTTP pipe transport.

`runner.lock` holds a nonblocking OS lock for the installation lifetime.
`active.json` publishes its random endpoint, management secret and release.
`status` queries that endpoint; `drain` refuses new jobs/spawns, waits for
existing contexts, stops the runner and waits for its OS lock to release.
The backend exposes `/install.sh` and immutable artifact downloads when
`runnerReleaseDir` (or `DEMI_RUNNER_RELEASE_DIR`) is configured. The installer
checks both hashes, rejects a state directory bound to another backend,
serializes installation changes and upgrades only that installation.
`DEMI_INSTALLATION_ID` selects another registration of the same backend.
Each job's PATH points to its pinned manifest's root aliases; updating the
manifest does not change already-running jobs.

Build a matched release with:

```sh
bun run --conditions development packages/runner/runtime/release.ts /path/to/releases macos-arm64 linux-arm64
```

The release directory contains `manifest.json` plus
`<release>/<target>/{demi,demi-runner}`. Supported release targets are
`macos-arm64`, `macos-x64`, `linux-arm64`, and `linux-x64`. Windows is not
published: native runner packaging, explicit local-only pipe ACLs and
Windows execution acceptance remain required. `/install.ps1` returns 503
with that limitation rather than distributing an unverified runner.

Automated tests cover two simultaneous runner registrations, separate
cwd/env, 3 MiB binary pipes, stale/mismatched contexts, duplicate ownership,
CPU-loop cancellation, fragmented and oversized frames, disconnects and
cancellation while stdout is backpressured. Installer tests exercise real
paired binaries, reuse and an independent upgrade across two backends.

The [IPC size experiment](../experiments/demi-ipc-size/README.md) measured
C + libuv transport probes at 105 KiB on macOS ARM64, 113 KiB on Linux ARM64,
and 141 KiB for a Windows x64 cross-build. Equivalent txiki.js probes were
2.50 and 2.75 MiB on macOS and Linux. Unix transfer checks passed; Windows
execution and complete command semantics were not tested. These results
support the implementation choice but do not constitute acceptance of a
production client.
