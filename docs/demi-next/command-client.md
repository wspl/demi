# Local command forwarding

`demi-runner` also acts as the external command client. Each declared root receives
an alias to the executable; its basename selects the root. The runner owns CLI
parsing and validated dispatch. The client forwards raw argv, cwd, environment and
a live execution context, then streams stdin, stdout, stderr and completion.
It contains no native command implementations.

The embedded brush shell calls the same dispatcher through registered builtins
inside the resident runner process. It does not launch a forwarding executable
or use the local IPC endpoint for these calls. The executable remains available
for callers outside brush, including scripts, `xargs` and subprocesses. Both
entry paths use the same command definitions and execution-context lifetime.

## Transport and lifetime

`runner/src/local.rs` creates an owner-restricted Unix domain socket on macOS and
Linux, or a local Windows named pipe restricted to the current account. Each client
opens a direct HTTP/2 connection and one invocation stream. The shared framing,
flow control and completion contract belongs to `command-protocol` and
`command-service`; see [native-runtime.md](native-runtime.md).

The raw CLI request carries root, argv and context. The runner authenticates the
context and validates the pinned declaration before constructing a native service
invocation or application callback. A package never receives an unvalidated raw
CLI request. The endpoint and context are injected into each job by its runner.

Input is read only in response to explicit demand. HTTP/2 window capacity does
not authorize reading stdin. EOF is distinct from cancellation. Connection/reset
processing remains active while command output is blocked; client loss cancels
its invocation. Cancelling a native invocation preserves unrelated streams in the
same resident service.

## Installation

A runner release contains one executable per target. Command aliases use the
selected release of their owning backend registration. Status and drain use an
authenticated management operation on that registration’s local endpoint.
The Unix and PowerShell installers verify release metadata, size and SHA-256
before publishing an executable, and use installation locks for upgrades.

Local endpoint, fragmented/binary IO, input demand, stale context and cancellation
tests live under `runner/tests` and `command-service/tests`. Cross-compilation and
native target execution are separate checks in the native CI workflow.
