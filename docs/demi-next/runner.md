# Runner connections and shell jobs

`demi-runner` turns a device into an execution target. It connects to the backend,
performs Host filesystem and process operations, and owns shell jobs. The backend
retains agent sessions, provider selection, and conversation history.

This document owns registration and job lifetime. [Commands](commands.md) defines
declared command dispatch, and [native execution](native-runtime.md) defines
artifact installation and resident command services.

## Connection and identity

Each backend registration has its own credentials, cache, local endpoint, and
selected runner release. The runner keys installation state by normalized backend
URL and holds an OS lock while that installation is active. Separate
registrations do not share their authorization context.

A paired device stores its device token in private installation state. A managed
guest receives a token at boot and keeps temporary state. The backend owns device
claiming and user ownership; the runner opens an outbound WebSocket and validates
MessagePack messages against the generated runner-protocol contract.

The authenticated local management endpoint exposes status and drain. Draining
stops admission, waits for active work, and releases the installation lock so a
replacement runner can start. [External command clients](commands.md#external-command-clients)
defines local endpoint access and installer verification.

A message on the connection is at most 4 MiB, the limit `runner-protocol`
defines for both ends. The side about to send a larger one fails the one
request that message belongs to with `too_large` and keeps the connection; a
directory with tens of thousands of entries, for example, cannot be listed. A
receiver still closes a connection that delivers a larger message, since its
peer broke the protocol. Bulk bytes never need a large message: file contents
and command IO travel through [pipes](#pipes-and-output).

The limit is eight times the largest regular message, a 5,000-file working
tree list or a shell command as long as a model can write, about 0.5 MiB
each. One connection carries everything, so a message in flight holds up the
rest; at 4 MiB a message still leaves within the 30-second write deadline on
an uplink of a little over 1 Mbit/s, and a full inbound queue of eight
messages stays within 32 MiB.

## Host operations

Filesystem and raw process requests do not require a shell job. The runner
performs them on the target device. Raw process environment selection follows
these rules:

| Request | Child environment |
| --- | --- |
| No `env` | Inherit the device process environment. |
| Explicit `env` | Use the supplied values. |
| `env` with `inheritEnv: true` | Overlay supplied values on the device environment. |
| Undefined value in an overlay | Remove that inherited variable. |

Runner-owned context variables override caller values. An opaque context ties
command callbacks to the live execution, registration, and pinned manifest.
The runner releases contexts when execution completes, is cancelled, or loses
its backend connection. Provider CLI assembly can request environment inheritance,
but the runner has no provider-specific behavior.

### File contents

A file's contents never travel in a message; they go through a pipe, the
way command IO does. Messages carry the request, its reply and the file's
metadata.

- `fs_readFile` names a file, an optional byte range (`offset`, and `length` up
  to the end when absent) and an output pipe. The runner opens the file and
  replies once it is open and positioned; only then does it stream the range
  into the pipe. A file that cannot be opened is an error reply, never an
  empty stream.
- `fs_writeFile` names a file, whether to create its parent directories, and an
  input pipe. The runner writes into a temporary file beside the destination
  and renames it into place only when the pipe ends cleanly, then replies. A
  pipe that fails or is cancelled removes the temporary file and leaves the
  destination as it was.
- A pipe that fails, for example because the reader of a preview went away,
  stops the read and closes the file.
- Every pipe end named to the runner is reported with `pipe_done`, including
  one a refused request never used.

For example, the browser seeks a video to the middle of a 300 MB file. The
backend asks for `fs_readFile` from byte 150,000,000 into a new pipe; the runner
opens the file, seeks, replies, and uploads the bytes as the backend accepts
them. When the user picks another file, the backend fails the pipe, and the
runner's upload ends and the file closes.

### Working tree

A `git_changes` request lists the uncommitted changes under a directory, and
`git_show` sends a file as the last commit has it. The runner answers both in
process with gitoxide; the device needs no git executable. Both name the
directory as `root`; a `git_show` also names the file's path relative to it
and an output pipe, and streams the whole file into the pipe like any
[file contents](#file-contents). Git stores the file compressed, often as the
difference from another version, so the runner decodes it whole in memory
before replying.
The `git_changes` reply:

| Field | Meaning |
| --- | --- |
| `repository` | False when the directory is not inside a git repository; the other fields are then empty. |
| `head` | The commit the changes are against; null before the first commit. |
| `files` | One entry per changed file under the directory, path relative to it: `added`, `modified`, `deleted`, or `renamed` (with the old path in `from`), plus the lines added and removed against `head`. |
| `truncated` | True when the list stopped at 5,000 files. |
| `watched` | True when the runner answered from a watched baseline, described below. |

Staged and unstaged changes form one list: what a commit of everything would
contain. Untracked files count as added; ignored files are absent. Line counts
skip binary files and files over 8 MiB, which report 0 and 0. `git_show` refuses
a blob over 8 MiB with `too_large`, answers `ENOENT` for a path the last commit
does not have, and `not_repository` outside a repository.

The first request for a directory walks its whole tree. It also starts a
filesystem watch of the directory, and of the repository's `.git` when that
lies outside it, which only records the paths changed since. The next request
re-examines those paths and merges them into the previous result; a change
under `.git` (a commit, a checkout, a staging) recomputes the whole. Anything
that makes the watch unreliable turns it off for that directory, and the runner
walks again: the watch cannot be created (an inotify limit, permissions, an
unsupported filesystem), events overflowed, the platform asks for a rescan, or
more than 10,000 paths accumulated. `watched` reports the outcome. A rename
between two walks can show as a deletion and an addition until the next whole
walk.

The runner keeps at most eight watched directories per connection and drops one
after fifteen minutes without a request; closing the connection drops them all.

Working-tree work runs on blocking threads off the connection's control loop.
At most two computations run at a time; a request beyond that answers `busy`
at once, and requests for the same directory share one computation. A
computation stops at its next check when the connection closes or after thirty
seconds (`timeout`). A failure inside the git library answers `internal` for
that request and affects nothing else.

### Network streams

A `net_open` request asks the runner to connect to a TCP address on the
device's network and carry bytes both ways. It names the stream, the
`host` and `port` to connect to, and two pipes ([Pipes and output](#pipes-and-output)):
`input`, whose bytes the runner writes to the socket, and `output`, into
which it writes what the socket sends. The runner resolves the host name on
the device, connects within 10 seconds, and answers `net_opened`, or
`net_error` with `refused`, `unreachable`, `resolve_failed`, or `timeout`;
no bytes move before that answer. The input pipe ending shuts the socket's
write side; the socket's end-of-stream ends the output pipe; a pipe failing
or the connection to the backend closing closes the socket. The runner
tracks each open socket and reports its two pipe ends with `pipe_done` like
any other pipe. The stream is generic mechanism: the runner does not parse
what flows through it. The backend uses it for the public relay of
[Host expose](expose.md#the-public-relay).

### Service streams

A `service_open` request asks the runner to open a
[user stream](native-runtime.md#user-streams) and carry its bytes both ways.
It names the stream, the conversation, the user's locale, the declared package
and operation, and two pipes ([Pipes and output](#pipes-and-output)): `input`, whose bytes the
runner delivers to the invocation as input chunks when the operation asks for
them, and `output`, into which it writes the invocation's output. The runner
starts the invocation in the resident service that holds the conversation's
state, starting the service when needed, and answers `service_opened`, or
`service_error` with `unknown_operation`, `service_failed`, or `refused`; no
bytes move before that answer. The input pipe ending ends the invocation's
input; the invocation's completion ends the output pipe and is reported with
`service_exit`; a pipe failing or the connection to the backend closing cancels
the invocation. The runner reports each pipe end with `pipe_done` like any other
pipe. Like a network stream, the service stream is generic mechanism: the
runner does not parse what flows through it. The backend uses it for the
[live browser view](browser-live-view.md).

## Shell jobs

A shell job owns its working directory, environment, IO, and asynchronous work.
Brush runs inside the resident runner process. Declared roots call the shared
command dispatcher; external tools such as Git, Python, and Node run as child
processes. Every in-process file write passes through the job's scope, which
reports the files the job created or modified when it exits
([Edit tracking](edit-tracking.md)).

The diagram shows ownership, not execution order. Cancelling job A releases its
work while preserving the runner and job B.

```text
Runner process
+--------------------------------------------------+
| Job A                    Job B                   |
| +--------------------+   +--------------------+  |
| | Brush execution    |   | Brush execution    |  |
| | Background tasks   |   | Background tasks   |  |
| | IO and child work  |   | IO and child work  |  |
| +--------------------+   +--------------------+  |
+--------------------------------------------------+
```

Each job starts a fresh login shell. Brush loads the system profile and first
readable user login profile. The runner then restores its execution context,
places command aliases first in PATH, and restores the requested cwd. Shell
variables and functions do not carry over to the next job; persisted profile
changes do. The backend receives the final cwd and foreground exit status.

A job waits for background tasks and process substitutions before reporting
completion. For example:

```sh
(sleep 2; echo done) & echo started
```

The caller sees `started`, then a running job, then `done` and completion.
A tool timeout returns the running job's handle. `shell_status` observes that job,
and `shell_abort` cancels it. Background tasks remain job-owned rather than
becoming detached services. Brush's internal tasks do not expose OS PIDs in `$!`.

### Cancellation and completion

Completion means the job has released its local work and IO, not merely that its
foreground script returned. The shell scope tracks interpreter tasks, utility
workers, and external children until they finish.

| Outcome | Cleanup |
| --- | --- |
| Success | Join remaining job-owned work and preserve produced output. |
| Failure | Cancel remaining work, join it, and report the failure. |
| Cancellation | Stop shell work, command invocations, and external descendants; release IO and reap children. |

Embedded execution cooperates with cancellation. Blocking IO must be interruptible;
external children belong to a Unix process group or Windows Job Object. The runner
continues handling control requests while a job blocks on input or output.

A cancelled job reports the signal that requested its cancellation, or `SIGKILL`
when cancellation had no signal request and forcibly terminates external descendants.

A cancellation request alone does not establish that execution stopped. If the
backend cannot confirm remote termination, it reports an unknown outcome.
Cancellation does not undo completed file changes or other side effects.

## Command lifetime

Builtin calls and external forwarding use the same execution context. Releasing
that context releases command bindings and service references. The command set
and package catalog follow the embedding program's
[startup binding contract](native-runtime.md#bind-an-exact-package).

Connection loss invalidates contexts and cancels their work. Reconnection creates
fresh contexts; it does not resume streams or replay commands. Reconnecting the
network does not itself change the embedding program's command set.

## Pipes and output

Jobs retain full output in device-local files and send bounded views to the
backend. `shell_status` returns output since the preceding view. A caller needing
complete output reads the retained file or accumulates those views.

The runner owns its HTTP pipe transfers; the backend's pipe broker owns their
rendezvous and lifetime. Binary payloads remain bytes. EOF ends input, while
cancellation aborts execution. Live input follows
[explicit command demand](commands.md#deliver-io-and-release-an-invocation), so a
command that never reads stdin does not consume subsequent interactive input.

Live `spawn_stdin` and `job_stdin` frames carry at most 64 KiB of bytes, matching
the [native stdin chunk limit](native-runtime.md#request-body-and-input-demand);
the sender splits larger writes into ordered frames before sending EOF.

## Managed guests and verification

In a managed guest, the runner performs Linux PID 1 initialization, mounts
filesystems, configures networking, reaps children, and permanently drops to the
guest account. Filesystem operations and jobs use that account. The VM provides
the isolation boundary. [Managed hosts](managed-hosts.md) owns boot, volumes,
provisioning, and reset policy.

The implementation belongs to `crates/runner`: connection and registration code
owns transport lifetime; `host.rs` dispatches Host operations; `shell/` owns brush
and job cleanup. [Package boundaries](../package-boundaries.md) defines module
ownership without duplicating it here.

Verification fixtures under `crates/runner/tests/` cover connections, processes,
Host operations, shells, pipes, and local clients. TypeScript integration uses the
actual Rust executable through `host-remote/testing`. The
[native build guide](../native-builds.md#validation) defines target execution checks.
Test locations identify the required coverage. Target execution results belong
to CI and acceptance reports.
