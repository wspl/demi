# Concurrency

This document defines how Demi's Rust programs use threads and tasks, where
each piece of state lives, and how work is serialized, moved off async threads,
cancelled and cleaned up. [Crates and packages](crates-and-packages.md) names
the crates; the behavior of each subsystem is in its own document.

For example, a browser downloads a file from a conversation on the user's
Cloud:

```text
browser --GET--> edge (any thread)
                  | call the user's shard
                  v
                 user's shard (one thread)
                  - admits the download: the conversation's host access takes
                    the file gate and wakes the stopped Cloud
                  - opens the runner's read stream
                  | returns a lease and the pipe's reading end
                  v
                 edge copies the bytes to the browser
                  | the download ends, fails or is abandoned
                  v
                 the edge drops the lease; the shard releases the admission
```

The decision (may this download run, and on which Host) and the per-user state
it touches stay on the user's shard. The bytes and their stall timer stay at
the multi-threaded edge. Dropping the lease is the release: no code path has
to remember to call it.

## Programs and threads

Each program picks its model by what its state belongs to.

| Program | Threads and runtimes | Why |
|---|---|---|
| Backend | A multi-threaded edge; shard threads, each with a single-threaded runtime; one thread per open SQLite connection; the blocking pool | A user's conversations, runner connections and Cloud state change on every tool call, so they stay on one thread; bytes and cross-user services run in parallel at the edge |
| Runner | A single-threaded control runtime; a shell runtime with its own large blocking pool; a Host log thread | Its state belongs to one registration and one backend connection, so there is nothing to run in parallel; shell work must not share a bounded pool with control work |
| `demi-commands` | A multi-threaded runtime with owner tasks | Its state belongs to many conversations whose browsers must make progress in parallel |
| `demi-claude` | A current-thread runtime | Installs are rare and wait on IO |
| Machine manager | A single-threaded loop, one worker task per device, the blocking pool and one-shot namespace threads | Little state and much blocking work |

All programs use Tokio. The backend's shard threads, the runner's control
thread and the machine manager's loop use Tokio's `LocalRuntime`, so their
state can be `!Send` (`Rc`, `RefCell`) and a test's paused clock reaches them.
Each runtime builds its own `reqwest::Client`, because hyper ties a pooled
connection's task to the runtime that created it.

### Backend

```text
edge: multi-threaded runtime with axum
  HTTP routes, the session gate, body limits, static files
  byte paths: pipes, file transfers (60 s stall), user streams, the expose relay
  shared services: control records, conversation stores, blobs, the change
    store, the vault, provider assembly and catalogs, credential refresh gates,
    the machine-manager client, Cloud capacity, pending runner claims,
    the login limiter
       |  shards.of(user).call(|shard, cancel| ...)  owned data in and out
       |  adopt(socket)                              conversation and runner sockets
       |  lease and pipe ends back to the edge       transfers, user streams, exposes
       v
shard threads: each a LocalRuntime hosting the shards of the users pinned to it
  one user's shard: conversations (file gates, transfers, idle watches),
    agent trees, runner connections, pipe records, the Cloud machine,
    exposes, titles, forks, the command router, the request rate limit
       |  owned data in and out
       v
threads: one per open SQLite connection; the blocking pool for disk,
  password hashing, and request-body and transcript serialization
```

The edge uses axum. axum requires handler futures to be `Send`, which costs
nothing because edge handlers are thin: they parse, authenticate, and call a
shard or a shared service. [Backend](../backend/backend.md) owns the modules;
[The user shard](#the-user-shard) owns the rules of the boundary.

The shard threads are Demi's own small pool of `LocalRuntime` threads fed by a
job channel. tokio-util's `LocalPoolHandle` is not used: it gives each worker
a runtime of its own, which a test's paused clock cannot reach
([Tests and time](#tests-and-time)).

### Runner

```text
control thread: LocalRuntime
  registration: installation state, the device token and status (watch),
    the local command endpoint, the service registry, the reconnect loop
  connection (one per backend WebSocket): routes each message, and owns the
    job table, execution contexts, the callback relay, the artifact locator,
    volumes and the git service; its child tasks carry transfers, streams
    and job IO
       |  Admission: the load semaphores (Runner § Load)
       v
  blocking pool: whole filesystem requests, git computations, hashing
shell runtime: one worker and a large dedicated blocking pool
  interpreter units (the job's root, pipeline stages, subshells, background
  lists, substitutions) and standard utilities, each joined by its job
Host log thread: owns the log files; sources write through a tracing layer
```

The control runtime is single-threaded because its state belongs to one
registration and one connection. Plain `&mut` ownership then needs no locks,
tests run deterministically on a paused clock, and the byte paths wait on IO
rather than CPU. CPU and blocking work leave the thread through `Admission`,
whose semaphores are the [Load](../execution/runner.md#load) limits.

Shell work has its own runtime. Each interpreter unit and utility needs an OS
thread: builtins and utilities do synchronous IO, utilities keep their state in
thread-locals, and pipeline stages run at the same time. A stage blocked
writing a full pipe waits for a sibling to read it. If both waited in one
bounded pool shared with short control work, a full pool would deadlock them,
so the shell pool is separate and far larger than a job can use. On Unix, the
runner's end of each job pipe is asynchronous and owned by a control task, and
a job's cancellation is a pipe polled together with the file.

When `demi-runner` runs as a command alias, it serves one invocation on a
current-thread runtime.

### demi-commands

```text
multi-threaded runtime
  one task per invocation (the command-service SDK), routed by operation
  file mutations: one SerialGate, the work on the blocking pool
  per conversation: a browser owner task (absent, starting, ready, closing)
    per running Chrome: tab registry, capture channel and live hub owner tasks,
      all joined through one TaskTracker when the browser retires
```

Several conversations' browsers must make progress at once, and the Chrome
DevTools connection decodes large messages (screenshots arrive as megabytes of
base64 JSON) on whichever worker polls it. Owner tasks remove the locks, so
per-conversation threads would add nothing but a hop, and would pin a
conversation's agent commands, live view frames and DevTools decoding to one
thread.

### demi-claude and the command-service SDK

`demi-claude` runs on a current-thread runtime. Hashing an installed
executable and writing files each run in one blocking call.

The command-service SDK does not choose a runtime. Handler futures are `Send`,
which all three programs satisfy; the SDK runs one task per invocation and
joins them with the connection.

### Machine manager

```text
loop thread: LocalRuntime; owns the manager state
  the socket server and its connections; one task per request
  one DeviceWorker per device: owns its sandbox and runs that device's
    operations in order; a sandbox death is one more message in its queue
  Admission: a fair semaphore; device operations share it,
    reconcile and shutdown take it whole
blocking pool, through one entry point: file work and fsync, mounts, loop and
  freeze ioctls, sparse copies, hashing, firewall updates
one-shot threads: work inside another namespace; each exits after one job
child processes: runsc, mke2fs, e2fsck, resize2fs, bsdtar, nft
```

Namespace work never runs on a pool thread: a pool thread that entered another
namespace would run later jobs in the wrong one. Recovery of the saved mount
namespace is a separate process that the manager starts by re-executing itself
through `/proc/self/exe`, entering the namespace before `exec`, so every thread
and child of that process lives in it.

## The user shard

A user's shard holds everything that belongs to one user: the user's
conversations with their file gates, transfers and idle watches, the agent
trees, the runner connections and pipe records of the user's devices, the
user's Cloud machine, exposes, title requests and forks. A shard thread hosts
the shards of every user pinned to it.

A shard runs on one thread, so nothing else runs between two of its awaits.
Admission logic therefore needs no locks: a check and the state change it
guards happen with no await between them. For example, a new transfer checks
that the conversation's transfers are not being closed and registers itself in
the same synchronous step, so a switch that closes transfers cannot miss it. A
section like this carries a comment at the site and a test that interleaves a
competitor at that point; adding an await inside it reopens the race.

**Placement.** Decisions and per-user state live in the shard; bytes and
cross-user state live at the edge and in shared services.

| In the user's shard | At the edge or in shared services |
|---|---|
| File gates, transfer admission, idle watches, target switch, archive and detach | The bytes of downloads, uploads, user streams, pipes and the expose relay, with their stall and idle timers |
| Agent trees and their sessions | HTTP routing, the session gate, body limits and static files |
| Runner connections, one task each, and pipe records | Runners not yet paired, which have no user |
| The Cloud machine and its reset intent | Cloud capacity, counted across users, and the machine-manager client |
| Live expose connections, titles, forks, the command router and the request rate limit | Storage, the vault, provider assembly and catalogs, credential refresh gates and the login limiter |

**Crossing the boundary.** The boundary is crossed per request, WebSocket
upgrade or stream admission, never per tool call. What crosses is owned data
in both directions, `Send` pipe ends and leases:

- An ordinary request calls `shards.of(user).call(|shard, cancel| ...)`: the
  closure is `Send`, the future it starts runs on the shard and need not be,
  and the result is `Send`. The edge checks ownership first, so a request only
  reaches its owner's shard.
- A conversation socket or a runner's socket, once accepted, moves into its
  user's shard, which handles its messages one at a time. The edge reads a
  runner's first message: a known token moves the socket into its owner's
  shard, and an unpaired runner waits at the edge.
- A transfer, user stream or expose connection returns a lease with its pipe
  ends. The edge copies bytes while it holds the lease; dropping the lease
  releases the admission in the shard, and the shard can end the lease itself,
  for example when the conversation is archived.

**Call semantics.** A shard call always runs to completion. When the requester
leaves, the edge cancels the call's token, and only waits observe it, such as
a wait for the file gate or for a Cloud to wake, never a step after a commit.
For example, a target switch that has won its storage write still restarts the
conversation's idle watch, even if the browser disconnected. Transitions such
as switch, archive and detach run as spawned shard tasks once they hold the
conversation.

**`Send` bounds.** Shard code uses `Rc`, `RefCell` and local futures, and the
traits it calls through, such as `Host`, `ShellEnvironment`, `RpcHandler` and
`ProviderRuntime`, have no `Send` bound. Shared services, such as a provider
entry, the vault and the control records, are `Send + Sync`.

**Threads and users.** A user is the unit of placement at both levels: a
multi-worker deployment pins each user to one worker process
([Deployment and user ownership](../backend/backend.md#deployment-and-user-ownership)),
and inside a process each user is pinned to one shard thread. The backend
starts with one shard thread. More threads are configuration, provided
per-user state stays in the shard and cross-user state stays in shared
services.

**Composition.** The shard's behavior is written as `impl Shard` blocks, one
per module. Each component owns its state, and `Shard` combines them, so no
component holds a reference back to another.

## Locks

The runner's device token is read by every pipe request and changes only when
the device is claimed. The registration owns it and publishes it through a
`watch` channel; each reader copies the current value when it needs it, and
no reader can hold up the registration.

"Lock" covers three needs, and each has one tool:

| Need | Tool | Examples |
|---|---|---|
| State shared by tasks | One owner: shard-local state, or a task that owns it and answers requests | The runner's job table belongs to its connection; `demi-commands`' tab registry is an owner task |
| Serializing an operation across awaits | A named gate from the `gates` crate, or a purpose-named gate built on a semaphore | A conversation's file gate; the file mutations of `demi-commands`; a credential refresh that must run once |
| Read-mostly data | An immutable snapshot published through `watch` | The runner's device token; the tab snapshot commands look tabs up in, so commands on different tabs run in parallel |

**Gates.** An `ActivityGate` is a first-in, first-out semaphore. A lease is one
permit: work in progress, for example one host access. A reservation takes
every permit: a transition that needs the gate quiet, such as a target switch.
A reservation that is waiting holds back every later entrant, and trying to
reserve succeeds only when nothing holds the gate. The gate publishes its state
as a snapshot, including when demand last ended, which idle watches read. A
`SerialGate` admits one holder at a time in arrival order, and its permit can
move into a task. Leases, reservations and permits release when dropped.

**Inside a shard.** Ownership comes first: ids, owned maps and `&mut self`.
`RefCell` is for the few places where sharing is inherent, one per component,
behind synchronous methods, so no borrow crosses an await. A component that
emits events collects them while its state is borrowed and delivers them in
order after the borrow ends; a listener is taken out of its slot while it runs,
so it may call back into the emitter, and a subscription ends when it is
dropped.

**`std::sync::Mutex`** stays only where state is shared across threads for a
short section that never awaits, such as the backend's pending runner claims,
which edge threads share. Each use says why in a comment. The native programs
keep a handful of these, for example the set of changed paths that a file
watcher's own thread records.

**Enforcement.** The workspace lints deny `await_holding_lock` and
`await_holding_refcell_ref`. `std::sync::RwLock` and `tokio::sync::RwLock` are
disallowed types (publish a `watch` snapshot), and so is `tokio::sync::Mutex`
(give the state one owner, or use a named gate).

**Queues.** Every queue is bounded; `tokio::sync::mpsc::unbounded_channel` is a
disallowed method. A queue's owner decides what a full queue means, and the
rule lives with the owner: a conversation socket's outbox closes a connection
that lags, and the client reconnects to the running session
([Frame protocol](../agent/runtime.md#frame-protocol)); the runner's
connection stops reading until it has room ([Runner](../execution/runner.md)).

## Blocking work

A login hashes the password with argon2id, which takes many milliseconds of
CPU. The edge hands the hash to the blocking pool, which runs at most as many
hashes at once as the machine has CPUs, and the edge's workers keep serving
other requests meanwhile.

CPU work over about a millisecond, and every blocking call, leaves the shards,
the edge and control threads through `spawn_blocking` or a dedicated thread.
What crosses is owned data in both directions.

| Program | Where blocking work runs |
|---|---|
| Backend | One thread per open SQLite connection, because rusqlite is synchronous and a transaction stays on one thread; the blocking pool for other disk work, password hashing, and serializing request bodies with media and transcripts |
| Runner | The control blocking pool behind `Admission` for filesystem requests, git computations and hashing; the shell runtime's pool for interpreter units and utilities; its own thread for the Host log |
| `demi-commands` | The blocking pool for file mutations, archive extraction, image decoding and encoding, output publication, process-table scans and hashing |
| `demi-claude` | One blocking call each for hashing and file writes |
| Machine manager | One entry point to the blocking pool; the storage and Linux functions take a token only that entry point creates, so running off the loop is checked at compile time. Child processes start from the loop, because spawning is quick and waiting is event-driven, and dropping one kills it; firewall updates run on the pool because their library spawns synchronously |

`tokio::task::block_in_place` is a disallowed method everywhere. Crates whose
code runs on async threads, the backend's shard and edge modules, the runner's
control modules and the machine manager's loop, also disallow `std::fs`,
`std::thread::sleep` and SQLite calls outside their blocking modules. A
blocking module carries a module-level
`#[expect(clippy::disallowed_methods, reason = "...")]`, so the exception is
visible where it applies.

## Cancellation and cleanup

In the opening example, the browser closes the tab halfway through the
download. The edge's response body stops, the edge drops the lease, and the
lease's owner task in the shard fails the pipe and releases the file gate. If
instead the conversation is archived mid-download, the shard ends the lease,
and the edge cuts the response short so it never looks complete.

- **Tokens mirror ownership.** Cancellation is a `CancellationToken` tree that
  follows ownership. Cancellation is not an error value.
- **Every task has an owner.** A spawned task belongs to its owner's
  `TaskTracker` or `JoinSet`; there is no detached spawn. Shutdown cancels,
  then joins. Timers are such tasks, owned by their component, for example a
  session's persister and yield driver, idle watches and expiry timers; no
  record stores a timer handle.
- **Guards release on drop.** Gate leases, reservations, transfer leases and
  admissions release when dropped, on success, failure and cancellation alike.
- **Cancel waits, not commits.** Each await decides whether cancellation
  reaches it. A turn races provider streams, tools, backoff sleeps and
  admission waits against its token and drops them; a store save is never
  raced. Work that must outlive its waiter is owned by a task of its owner,
  never by the waiting future.
- **`select!` is cancellation-safe.** Every branch is safe to drop, or its work
  is spawned under the owner.
- **Asynchronous release is explicit.** Something whose release takes awaits
  has a `close()` that does it, and dropping it is the fallback that kills. For
  example, a retained Claude Code process moves into a run and back to its
  runtime only at a point where it may be kept; `close()` sends SIGTERM, waits
  5 seconds and then sends SIGKILL.
- **Answers are channels.** A result delivered from elsewhere is a `oneshot`;
  a settled state is a `watch`.
- **Ordering is a protocol.** When one outcome must follow another, the code
  says so: the session's worker records an abort, and `abort()` waits for its
  acknowledgement.
- **Writes whose loss matters are awaited.** A usage ledger row is written and
  awaited before the response reaches the agent, and a failure is logged.
- **Ignored errors are explicit.** An ignored error is a branch whose comment
  says why ignoring it is safe.

The machine manager never cancels a device operation: shutdown drains the
device queues. A checkpoint's frozen window is one synchronous job whose guard
always thaws the filesystem. Each program's shutdown order is in its own
document, for example the backend's in [Backend](../backend/backend.md).

## Tests and time

- **Paused clock in process.** Timer logic tested inside one process runs on
  Tokio's paused clock (`start_paused = true`, with the `local` or
  `current_thread` flavor), so a one-hour idle window elapses at once and
  assertions use exact times, not tolerances.
- **Real time with real processes.** Tokio advances a paused clock whenever
  the runtime is idle, including while it waits on a real process, so every
  timer would fire early. A test with a real runner or service uses real time
  and short configured windows.
- **Wall time is injected.** Wall-clock time comes from an injected `Clock`,
  and durations use `tokio::time::Instant`. The test clock follows Tokio time
  (a start time plus elapsed Tokio time), so paused tests move record
  timestamps too; a settable clock serves tests that move only wall time, such
  as expose expiry.
- **The backend runs inline.** A backend test without a real runner runs the
  shards on the test's own runtime instead of on shard threads, so the whole
  backend runs on one current-thread runtime that the paused clock reaches.
- **Real programs are built binaries.** A test that needs a real runner or
  another program starts the binary that `cargo xtask test` built and named in
  an environment variable ([Module layout](crates-and-packages.md#module-layout)).
