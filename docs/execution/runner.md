# Runner connections and shell jobs

`demi-runner` turns a device into an execution target. It connects to the backend,
performs Host filesystem and process operations, and owns shell jobs. The backend
retains agent sessions, provider selection, and conversation history.

This document owns registration and job lifetime. [Commands](commands.md) defines
declared command dispatch, and [native execution](native-runtime.md) defines
artifact installation and resident command services.

The runner keeps control work apart from the work it controls. Its control
thread reads the backend connection, routes each message to the work it
belongs to, and drives the byte paths of pipes and streams, which only wait on
the network; it never blocks. File and git requests, hashing, and other
blocking work run on blocking threads a few at a time ([Load](#load)), and
shell jobs run on a shell runtime of their own ([Shell jobs](#shell-jobs)). A
burst of requests, a slow disk, or a busy job therefore never stops the runner
from reading its connection. [Concurrency](../architecture/concurrency.md#runner)
gives the threads and the owner of each piece of state.

## Connection and identity

Each backend registration has its own credentials, cache, local endpoint, and
selected runner release. The runner keys installation state by normalized backend
URL and holds an OS lock while that installation is active. Separate
registrations do not share their authorization context.

A paired device stores its device token in private installation state. A managed
guest receives a token at boot and keeps temporary state. The backend owns device
claiming and user ownership. The runner opens an outbound WebSocket and sends its
`hello` first; the backend closes a connection that has sent nothing within 30
seconds of opening. Both ends decode every MessagePack message into the types of
the runner wire's contract crate, which they both link, and validate it at
entry; a message that fails closes the connection, since its sender broke the
protocol ([Validation at entry](../architecture/contracts.md#validation-at-entry)).
Integer fields travel as MessagePack integers, and byte fields as MessagePack
binary.

The authenticated local management endpoint exposes status and drain. Draining
stops admission, waits for active work, and releases the installation lock so
the next runner, such as a newer release, can start.
[External command clients](commands.md#external-command-clients) defines local
endpoint access and installer verification.

A message on the connection is at most 4 MiB, the limit the runner wire's
contract crate defines for both ends. The side about to send a larger one fails
the one request that message belongs to with `too_large` and keeps the
connection; a directory with tens of thousands of entries, for example, cannot
be listed. A receiver still closes a connection that delivers a larger message,
since its peer broke the protocol. Bulk bytes never need a large message: file
contents and command IO travel through [pipes](#pipes-and-output).

The limit is eight times the largest regular message, a 5,000-file working
tree list or a shell command as long as a model can write, about 0.5 MiB
each. One connection carries everything, so a message in flight holds up the
rest; at 4 MiB a message still leaves within the 30-second write deadline on
an uplink of a little over 1 Mbit/s, and a full inbound queue of eight
messages stays within 32 MiB.

The runner reads messages into that queue of eight. Handling a message only
routes it to the job, request, or stream it belongs to, which runs apart from
the connection, so the queue drains without waiting for a disk, a service, or a
job. When the queue is full anyway, the runner stops reading until a message
leaves it, and the backend's writes wait meanwhile. A full queue never closes
the connection; only a message that breaks the protocol, by its size or its
content, does. In the other direction the backend queues at most 64 messages
for a runner, and a sender waits for room instead of buffering without limit.

## Host operations

Filesystem and raw process requests do not require a shell job. The runner
performs them on the target device as the account it runs under, whose own
permissions decide which paths they reach. A Host's default working directory
is where work starts when a request names none; it is not a sandbox, a
workspace boundary, or a permission check. A process the backend starts outside
a shell job, such as a provider's CLI, is a raw process request. Signals travel
by name from one closed set that raw processes and jobs share. When the backend
drops the handle of a raw process that has not ended, it asks the runner to kill
the process and does not wait: nothing controls such a process any more.

Raw process environment selection follows these rules:

| Request | Child environment |
| --- | --- |
| No `env` | Inherit the device process environment. |
| Explicit `env` | Use the supplied values. |
| `env` with `inheritEnv: true` | Overlay supplied values on the device environment. |
| Null value in an overlay | Remove that inherited variable. |

Runner-owned variables, the local endpoint, the opaque context handle,
`DEMI_HOME` and the alias directory on `PATH`, override caller values. The
opaque handle ties command callbacks to the live execution, registration, and
pinned manifest; the live execution also holds the job's
[command context](native-runtime.md#command-context).
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
| `files` | One entry per path `git status` lists under the directory, path relative to it: its `status`, git's two status letters for it; its `kind`, how the working tree differs from `head`: `added`, `modified`, `deleted`, or `renamed` (with the old path in `from`); and the lines added and removed against `head`. |
| `truncated` | True when the list stopped at 5,000 files. |
| `watched` | True when the runner answered from a watched baseline, described below. |

The list holds the paths `git status --porcelain --untracked-files=all` lists,
ignored files being absent, and `status` is the two letters it prints before
each: the index against `head`, then the working tree against the index. For
example, a new file nobody staged is `??`, a staged new file edited again is
`AM`, a file edited without staging is ` M`, a conflicted one is `UU` or
another conflict pair. A rename staged in the index is one entry at its new
path, with `R` first; a file moved in the working tree without staging is a
deletion (` D`) and an untracked file (`??`), as git reports it. Two cases
differ from git's list: a path git lists twice, a deletion staged in the
index and an untracked file on the disk, is one entry, `??`; and a path in
neither `head` nor the working tree, added to the index and then deleted from
the disk, is left out, having no side to compare. The browser marks each file
from its `status` the way VS Code's Git does.

`kind`, `from` and the line counts compare `head` with the working tree,
whatever the index holds: a path whose status git lists but whose content
matches `head`, such as an executable bit changed or a staged edit undone on
the disk, is `modified` with 0 and 0. Line counts skip files that are not
text, binary and non-UTF-8 alike as in
[edit tracking](edit-tracking.md#scope), and files over 8 MiB; they report 0
and 0. `git_show` refuses a blob over 8 MiB with
`too_large`, answers `ENOENT` for a path the last commit does not have, and
`not_repository` outside a repository.

The first request for a directory walks its whole tree. It also starts a
filesystem watch of the directory, and of the repository's `.git` when that
lies outside it, which records only the paths changed since: reading a file
changes nothing, and neither does metadata alone under `.git`. On macOS the
watch is an FSEvents stream, which covers a tree of any size with one
subscription; notify's kqueue backend there would hold a file descriptor for
every file, more than a repository and the usual limit of 256 allow. No
request waits for the watch to start: FSEvents can take seconds to start a
stream when the system is busy. The first request that finds the watch
running walks the whole tree once more, since the watch saw nothing before
it ran.

The next request re-examines the recorded paths, with the other path of any staged
rename among them so that the two still pair, and merges them into the
previous result. It walks the whole tree again instead when a path under
`.git` changed (a commit, a checkout, a staging), when a `.gitignore` or
`.gitattributes` changed, since those decide what git lists for other paths,
or when more than 100 paths changed: a walk over some paths checks every index
entry against each of them, so past about a hundred it costs more than a whole
walk. The watch covers only the directory, so each request also compares the
`.gitignore` and `.gitattributes` files in the directories above it, up to the
work tree, with what the last walk found. Settings outside the repository, such
as the user's global ignore file, apply from the next whole walk. When the
watch cannot be created (an inotify limit, permissions, an unsupported
filesystem) or fails, the runner walks the whole tree for every request; when
it lost events or the platform asks for a rescan, the next request walks the
whole tree. `watched` reports whether a watch is running.

The runner keeps at most eight watched directories per connection and drops one
after fifteen minutes without a request; closing the connection drops them all.

Working-tree work runs on blocking threads off the connection's control thread.
At most two computations run at a time; a request beyond that waits for one to
finish ([Load](#load)), and requests for the same directory share one
computation. A computation stops at its next check when the connection closes
or after thirty seconds of running (`timeout`). A failure inside the git
library answers `internal` for that request and affects nothing else.

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
It names the stream, its [command context](native-runtime.md#command-context),
the declared package and operation, optional `args` and `json` that the
invocation receives as a command's does, the conversation's directory, which
becomes the invocation's `cwd`, and two pipes ([Pipes and output](#pipes-and-output)): `input`, whose bytes the
runner delivers to the invocation as input chunks when the operation asks for
them, and `output`, into which it writes the invocation's standard output. Its
standard error goes to the [Host's log](#host-log). The runner
starts the invocation in the resident service that holds the conversation's
state, starting the service when needed, and answers `service_opened`, or
`service_error` with `unknown_operation`, `service_failed`, or `refused`; no
bytes move before that answer. Starting the service may need its executable:
the runner asks the backend for its location as it does for a job's command
([Install the selected executable](native-runtime.md#install-the-selected-executable)),
naming the stream instead of a job, and the backend answers only while the
stream is open. The input pipe ending ends the invocation's
input; the invocation's completion ends the output pipe, which is how the
backend learns the stream is over; a pipe failing or the connection to the
backend closing cancels the invocation. The runner reports each pipe end with `pipe_done` like any other
pipe. When the invocation completes, the runner also sends `service_done` with
the stream, the invocation's exit code and the bounded tail of its standard
error: a one-shot call has no page to tell, so its caller learns a failure
and its words from this message, not from a stream that merely ended. Like a network stream, the service stream is generic mechanism: the
runner does not parse what flows through it. The backend uses it for the
[live browser view](../browser/live-view.md).

## Host log

A Host that cannot say what went wrong cannot be debugged. For example, the
live view module fails to list the browser's tabs on a Cloud and writes why to
its standard error; without a log those words are gone, and the page only
sees a view that never learns its tabs.

The runner keeps one log per Host, in its own data directory: text lines, each
with a time, a source and, when the work belongs to one, a conversation id.
Every part of the runner writes its diagnostics as `tracing` events, the
logging interface Demi's Rust programs share, with the source and the
conversation as event fields, and the Host log is a `tracing` layer that writes
those events as lines. No module holds a handle to the log's files. A resident
service's standard error enters the same way, one event per line as it arrives.

| Source | Lines |
| --- | --- |
| `runner` | Connecting and losing the backend, starting and stopping services, a [preinstalled executable](native-runtime.md#preinstalled-executables) it does not use and why, opening, refusing and ending streams, failed Host operations, guest boot |
| `service:<package>` | Every line a resident service writes to its standard error, as it arrives |
| `stream:<operation>` | The standard error of a [service stream](#service-streams)'s invocation, for example `stream:browser.live` |

- The log is bounded: two files of 4 MiB, the older replaced when the newer
  fills. It outlives a runner restart and, on a Cloud, a stop and a wake; a
  reset starts it again. A paired device keeps it in `log/` under its
  installation state. A managed guest's state directory is temporary, so the
  guest keeps it in `/var/log/demi`, on the
  [system layer](../cloud/managed-hosts.md#images).
- Writing never holds up the work it describes: the layer queues each line,
  and one writer thread owns the files. A line the queue has no room for is
  dropped and counted, and a write the disk refuses loses that line alone.
- A `log_read` request returns lines after a cursor, up to a limit of 1000,
  optionally of one source; `log_lines` answers with the lines oldest first
  and the cursor to continue from, or `log_error` when the files cannot be
  read. Without a cursor the answer ends at the newest line. The cursor is a
  line number that grows across both files and across restarts; one whose
  lines are gone, or that comes from a log a reset removed, continues from
  the oldest line still kept. The writer thread answers the request between
  two lines it writes, so the answer holds every line queued before the
  request and a read never races a write. The backend serves it as the
  [device log route](../product/web-api.md#device-log).
- What a source writes is diagnostics: what it tried and why it failed. Page
  content, typed text, cookies, tokens and file contents never go to the log.
- A failure the user's page is waiting on is also told to the page, by the
  request's answer or a stream's `notice`. The log is where the cause is
  found, never the only place a failure is told.
- A resident service that exits or breaks the protocol is recorded with its
  exit status after the standard-error lines that led to it, and every call it
  failed carries the last part of that output as well
  ([Invoke and retire a service](native-runtime.md#invoke-and-retire-a-service)).

The same log, route and bound serve a paired device and a Cloud.

## Load

The runner never refuses, drops or fails a request because others are in
flight. Work that lasts as long as its caller wants is not counted at all:
native command calls, commands the backend implements, user streams, network
streams and local command connections. Each is paced by its own backpressure,
a stream window or a pipe that moves only what the other side accepts, so a
slow reader holds back only itself and memory grows only with what runs. An
agent's `demi browser wait` or a user's open live view holds nothing another
command needs.

Host work that finishes on its own runs on blocking threads, off the control
thread, a few at a time, and a request beyond that waits for a slot:

| Work | At a time |
| --- | --- |
| Filesystem requests | 32 |
| Working-tree requests | 8 |
| Working-tree computations | 2 |
| Filesystem syncs | 4 |

A waiting request still ends when its connection closes.

Every job, stream and request shares the runner's one table of open files. A
job's login shell alone can hold about a hundred while it reads a profile that
loads a version manager such as nvm, and launchd gives a service on macOS a
limit of 256. So when it starts, the runner raises its own limit as far as the
system allows: to the hard limit, and on macOS to at most the kernel's limit
for one process. Every process it starts gets back the limit the runner was
started with, the one it would have from a terminal, unless its job's `ulimit`
set another ([Builtins that act on a process](#builtins-that-act-on-a-process)),
because some programs
misbehave with a very high one: a program that uses `select()` cannot watch a
descriptor numbered 1024 or above, and some programs close every descriptor up
to their limit before they start. Windows has no such limit.

Every pipe, local command connection and open file holds one while it lasts,
and a network stream holds three: its socket and two pipes. When none is left,
whatever needs one waits until another closes, instead of failing: pipes,
network streams, local command connections, filesystem and working-tree
requests, file transfers, process and job starts, and native service starts.
Inside a running job, every descriptor its shell makes waits too: pipes and
redirections, the copies it makes for subshells, pipeline stages, builtins and
`2>&1`, a here-document's file, a background list's `/dev/null`, and the
standard streams and descriptors a standard utility or a program it starts
receives. For example, out of open files, `echo one | cat > piped.txt` waits,
then writes the file. Nothing tells the runner when one closes, so it tries
again, at most a tenth of a second apart; a job's wait ends when the job is
cancelled. Running out is never an answer either: a working-tree request does
not report a directory as outside a repository because it could not open the
repository's files.

A standard utility waits too when it opens a file or starts a program. Its
other file work, such as `ls` reading a directory or `sort` spilling to a
temporary file, fails as it would on any system out of open files: making
every utility wait would change most of them, and the raised limit makes
running out rare.

A process start also waits, for about a second, while its program is busy.
Linux refuses to run a file that any process holds open for writing, and the
runner causes that itself. For example, it copies a service's executable into
its cache and starts it while another thread starts a job's `git`. Starting a
process copies the runner process first, and the copy holds every file the
runner had open, the cache file too, until it runs `git` a moment later. A
service start in that moment fails with `Text file busy` although the runner
has closed the file, and a job that writes a script and runs it can meet the
same. Every copy the runner makes runs its program at once, so the wait is
short; a program still busy after a second is open for writing elsewhere, and
its start fails. Every process the runner starts waits this way: services, raw
processes, a job's commands and the programs its utilities start, such as
`env` and `xargs`.

Two refusals remain. A browser command that conflicts with another command on
the same tab answers `tab_busy`; that is about the page, not load
([Conversation browser](../browser/browser.md#one-tab-registry)). An expose answers 503
beyond 64 concurrent connections: anyone on the internet can reach it, so it
sheds load instead of queuing it ([Host expose](expose.md#the-public-relay)).

## Shell jobs

A shell job owns its working directory, environment, IO, and asynchronous work.
It also carries the [command context](native-runtime.md#command-context) the
backend built for it; the runner never derives that context from the job's
environment.
Brush runs inside the runner process, on a shell runtime separate from the
control thread. Declared roots call the shared command dispatcher; external
tools such as Git, Python, and Node run as child processes. Every in-process
file write passes through the job's scope, which reports the files the job
created or modified when it exits ([Edit tracking](edit-tracking.md)).

Each interpreter unit, meaning the job's script and every pipeline stage,
subshell, background list, and process substitution, runs on a thread of its
own, and so does each embedded utility, from a large pool that serves only
shell work. On Unix the runner reads and writes its end of each job pipe
asynchronously, so a job's input and output take no threads; on Windows each
end takes one thread from the shell pool.
[Concurrency](../architecture/concurrency.md#runner) says why shell work has a
pool of its own.

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

The user login profile is the one in the job's home, the directory the job's
`HOME` names, and the profiles see that directory as `$HOME`. A job's `HOME`
is the one its request sets, else the device environment's, else the home
the runner reports for the device
([Resolve a target](sessions-and-targets.md#resolve-a-target)), so every job
has one. For example, a runner that a service manager starts without `HOME`
gives its jobs its account's home: they read `~/.profile` there, and a line in
it such as `. "$HOME/.local/bin/env"` finds its file. Without that default,
brush would read the same profile with `$HOME` unset, and the line would look
for `/.local/bin/env`.

Tests that start jobs give each job a home of its own, so no test reads the
login profile of the machine's user. The system profile belongs to the
machine, and jobs in tests read it too. A test therefore does not assume how
long a job takes to start or how many open files its start holds. A system
profile that loads version managers such as nvm can take a job a quarter of a
second to source and hold about a hundred pipes at once, since every unit of
a job shares the runner's open-file table ([Load](#load)).

A job waits for background tasks and process substitutions before reporting
completion. For example:

```sh
(sleep 2; echo done) & echo started
```

The caller sees `started`, then a running job, then `done` and completion.
A tool timeout returns the running job's handle. `shell_status` observes that job,
and `shell_abort` cancels it. Background tasks remain job-owned rather than
becoming detached services. Brush's internal tasks do not expose OS PIDs in `$!`.

### Builtins that act on a process

In bash, a few builtins act on the shell's own process: `exec` replaces it
with a program, `ulimit` and `umask` set the limits and file mode mask that it
and its children have, `kill $$` signals it, and `suspend` stops it. A job's
shell has no process of its own: it runs in the runner, whose process every
job, stream and request shares. A job that ran `exec node server.js` the bash
way would turn the runner into `node`, which ends every other job and the
device's connection, and a job's `umask 000` would make every file the runner
creates afterwards writable by anyone. So in a job each of these acts for the
job's shell instead, or fails. A job's shell starts with what the runner gives
the processes it starts: the runner's umask and limits, and for open files the
limits the runner was started with ([Load](#load)).

| Builtin | In a job |
| --- | --- |
| `exec CMD` | Runs CMD as `command CMD` would, a standard utility in the runner or a program through the job's process start, then ends the shell with CMD's status. In a subshell it ends the subshell. With only redirections, they stay with the shell, as in bash. |
| `ulimit` | Sets and shows the limits of the processes the shell starts from then on; a subshell keeps its own. Without `-S` or `-H` it sets both limits, as in bash. The system checks a new limit when it is set: the shell starts `/bin/sh -c :` with it, and a limit the system refuses, such as a hard limit raised without privilege or open files above macOS's cap, fails there and changes nothing, as it would in bash. |
| `umask` | Sets and shows the mask of the processes the shell starts and of the files its redirections and standard utilities create. The runner's own mask still applies beneath it inside the runner, so there a job's mask can only take permissions away. For example, under the usual runner mask `022`, a job's `umask 002` gives its programs group-writable files, but its redirections still create files with mode `644`. |
| `kill` | Signals any process but the runner. `$$` is the runner's process ID and 0 its process group, so `kill $$` and `kill 0` fail with a message. |
| `suspend`, `fg` | Fail: a job has no job control, as a bash script has none, and `suspend` would stop the runner. |

A job's limits apply to its processes only: its builtins and standard
utilities run in the runner, with the runner's limits. The other builtins act
on the shell alone already: `cd` and the directory stack, `trap`, which
installs no signal handler in the runner, `set` and `shopt`, `exit`, `wait`,
`jobs` and `bg`. `times` shows the runner's processor time, not the job's.

### Cancellation and completion

Completion means the job has released its local work and IO, not merely that its
foreground script returned. The shell scope tracks interpreter tasks, utility
workers, and external children until they finish.

| Outcome | Cleanup |
| --- | --- |
| Success | Join remaining job-owned work and preserve produced output. |
| Failure | Cancel remaining work, join it, and report the failure. |
| Cancellation | Stop shell work, command invocations, and external descendants; release IO and reap children. |

Embedded execution cooperates with cancellation. Blocking IO must be
interruptible: on Unix, a unit blocked on a read or write waits on the file and
on its job's cancellation together, so cancelling the job wakes it at once; on
Windows, the runner cancels the blocked call. External children belong to a Unix
process group or Windows Job Object. The runner continues handling control
requests while a job blocks on input or output.

A cancelled job reports the signal that requested its cancellation, or `SIGKILL`
when cancellation had no signal request and forcibly terminates external descendants.

A job or raw process ends with its exit code, or else the name of the signal
that ended it. With neither, the runner says why it has no status: the work
could not start, or the runner failed it before its end was known, in the
runner's own words. A failure of the runner's beside a known status goes to
the Host log.

A cancellation request alone does not establish that execution stopped. If the
backend cannot confirm remote termination, it reports an unknown outcome.
Cancellation does not undo completed file changes or other side effects.

## Command lifetime

Builtin calls and external forwarding use the same execution context. Releasing
that context releases its command bindings and its leases on resident services.
The command set and package catalog follow the backend's
[startup binding contract](native-runtime.md#bind-an-exact-package).

Connection loss invalidates contexts, cancels their work, and shuts every
resident service down, which ends the conversation state they hold. Reconnection
creates fresh contexts; it does not resume streams or replay commands.
Reconnecting the network does not itself change the backend's command set.

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
the sender splits larger writes into ordered frames before sending EOF. The
runner holds at most 64 of these frames for a job or process that has not
taken them; one more cancels that job or process, because routing a message
never waits for work that does not read its input.

## Managed guests and verification

On a Cloud, the runner is an ordinary UID 1000 process under init inside a
gVisor/systrap sandbox. It decodes an explicitly selected temporary managed-boot
file with the runner wire's contract types and fails if credentials are missing
or invalid; it never falls back to pairing. Filesystem operations and jobs use
that account. The runner performs no PID 1 boot, host mount, or network setup.
[Managed hosts](../cloud/managed-hosts.md#container-initialization) owns that
responsibility split, boot credentials, and lifecycle.

[Crates and packages](../architecture/crates-and-packages.md#crates) defines
what the runner crate owns and must not do. Tests that exercise a runner start
the built executable through the testing feature of the crate that owns the
backend's end of a runner, and tests that exercise a native service start its
built binary through the command-service library's testing feature; both find
the binary the way every test finds a built program
([Module layout](../architecture/crates-and-packages.md#module-layout)). The
[build guide](../delivery/builds-and-releases.md#validation) defines target
execution checks; their results belong to acceptance reports.

Acceptance on a paired device and on a Cloud observes these outcomes:

- A burst of file-browser requests during a slow manifest install or disk sync
  keeps the connection and every running job.
- Many concurrent jobs running pipelines of embedded utilities all complete;
  none waits for a thread another job's unit holds.
- A connection that never sends its `hello` is closed after 30 seconds.
- A resident service that exits reports its exit status and standard-error
  tail both in the Host log and in the calls it failed.
