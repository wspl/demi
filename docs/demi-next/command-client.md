# Demi Next: Native Command Client and Local IPC

| | |
|---|---|
| Date | 2026-09-08 |
| Status | Accepted target design; production implementation pending |
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

The proposed native source location is `packages/command-client`, outside
the TypeScript runtime dependency graph. It builds the `demi` executable.
The runner remains in `packages/runner`. `@demicodes/command-loader` remains
the single parser/dispatcher, hosted by the runner with injected Host and
RPC dependencies. The native client must not reimplement it.

Third-party root names use the same client transport and identify the root
in the invocation. Packaging those names must resolve to the client, never
to `demi-runner`.

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

libuv's pipe API provides UDS on Unix and named pipes on Windows. Both ends
must agree on an endpoint, but its operating-system representation differs:

- Unix: a socket under the runner's state directory, such as
  `${DEMI_HOME}/runner.sock` (`~/.demi` by default, `/run/demi` in Cloud).
- Windows: a local named-pipe name such as
  `\\.\pipe\demi-<user-id>-<instance-id>`, not a filesystem socket path.

The runner supplies its resolved endpoint to the jobs it starts. The client
uses that endpoint so changing cwd or entering another project does not
select another runner. For an ordinary terminal invocation, default
discovery must resolve the same user's default runner. Multiple instances
must have distinct endpoints. A project's directory is not an instance
identity: one user's Cloud runner serves multiple projects.

Endpoint discovery and installation normally happen automatically. Users
should not need to invent a pipe name or configure permissions manually.
The exact environment field and Windows name derivation are pending wire
and packaging implementation; the examples above specify the scope, not
an already implemented naming algorithm.

## Access: who may connect

An endpoint name locates a runner; it does not authorize its caller.

On Unix, the runner creates its state directory with owner-only access and
its socket with owner-only read/write access (0700 and 0600 respectively).
On Windows, the runner creates a local-only named pipe with an explicit
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
reject invalid frames. Exact framing/version negotiation and the C codec
are pending implementation decisions, not features of the size prototype.

Acceptance includes concurrent invocations with different cwd/env, binary
pipes and file redirection, help and argument errors, local/runtime and
backend/RPC commands, cancellation under backpressure, runner disconnect,
Unix and Windows endpoint isolation, Windows terminal/Unicode handling,
and separate runnable client/runner artifacts.

## Current implementation and evidence

Production still uses a shared txiki.js executable selected by invocation
name. Its command-mode process loads the manifest, executes runtime leaves
and relays RPC leaves. That implementation has not yet been replaced by
this design; `progress.md` tracks the gap.

The [IPC size experiment](../experiments/demi-ipc-size/README.md) measured
C + libuv transport probes at 105 KiB on macOS ARM64, 113 KiB on Linux ARM64,
and 141 KiB for a Windows x64 cross-build. Equivalent txiki.js probes were
2.50 and 2.75 MiB on macOS and Linux. Unix transfer checks passed; Windows
execution and complete command semantics were not tested. These results
support the implementation choice but do not constitute acceptance of a
production client.
