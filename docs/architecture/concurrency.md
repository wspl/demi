# Concurrency

This document defines how Demi's Go programs own state and goroutines, bound
work, serialize decisions, cancel waits and release resources.
[Packages](packages.md) defines package responsibilities;
each subsystem's document defines its behavior.

For example, the user's browser downloads a file from a conversation on their
Cloud:

```text
web app --GET--> edge request goroutine
                  | calls the user's Shard directly
                  v
                 conversation's host access
                  - checks and registers admission under the shard mutex
                  - waits outside the mutex for the file gate and Cloud wake
                  - opens the runner's read stream
                  | returns a lease and the pipe's reading end
                  v
                 edge copies the bytes to the web app
                  | the download ends, fails or is abandoned
                  v
                 deferred lease.Release() ends the admission
```

The decision (may this download run, and on which Host) and the per-user state
it touches stay in the user's shard. Bytes and their stall timer stay at the
edge. Go has no release on drop: the acquiring scope defers an explicit,
idempotent `Release`, or explicitly transfers that duty to another owner.
Garbage collection does not release an admission.

## Programs and threads

Go's scheduler runs every program's goroutines. A goroutine may block on IO or
CPU work; there is no application blocking pool or runtime per component.
Semaphores bound scarce work, and ownership determines who stops and joins it.
OS threads are an implementation detail except for namespace entry and
platform APIs that require thread affinity.

| Program | State owners | Concurrency bounds |
| --- | --- | --- |
| Backend | One `Shard` per user behind one mutex; edge request and copy goroutines; shared services own cross-user state | Password hashes, at most one per CPU; at most 64 open conversation writers and 64 cold-read connections; conversation gates |
| Runner | Registration owns installation, token, status and reconnect state; each backend connection owns its jobs and child work; each shell job owns its interpreter and descendants; a log writer owns Host log files | The runner's Load semaphores for finite Host work; streams use their own backpressure |
| `demi-browser` | Invocation goroutines and one browser owner per conversation, with child tab, capture and live-hub owners | Bounded transport and output, tab admission and operation gates |
| `demi-file` | Invocation goroutines; one file-mutation gate | Mutations run serially; reads use independent invocation streams |
| `demi-claude-code` | Invocation goroutines own installation work and IO | Installation serialization and command transport backpressure |
| Machine manager | Manager owns the server and device registry; one worker goroutine per device owns its sandbox | Fair operation admission; per-device queues; exclusive reconcile and shutdown |

### Backend

```text
edge: net/http handlers and owned byte-copy goroutines
  routes, session gate, body limits, static files
  pipes, transfers (60 s stalled-client timeout), user streams, expose relay
  shared services: control records, conversation stores, blobs, change store,
    vault, provider assembly/catalogs, credential refresh gates,
    machine-manager client, Cloud capacity, pending runner claims,
    login limiter, registry of open synchronization channels
       | direct Shard methods; owned arguments and results
       | socket ownership in; leases and pipe ends out; change marks
       v
one Shard per user, short sync.Mutex critical sections
  conversations, file gates, transfers, idle watches, agent trees,
  runner connections, pipe records, Cloud, exposes, titles, forks,
  command router, request rate limit, page synchronization channels
       | store operations outside the mutex
       v
shared database service: one connection per writable database
```

Handlers parse, authenticate, check ownership and call the shard or a shared
service. A request's disk work and serialization run in its own goroutine;
password hashes are the one CPU-bound step with a limit, one per CPU. HTTP
body limits and stream backpressure bound bytes.

### Runner

The registration owns the local command endpoint, service registry and reconnect
loop. Each backend connection handles incoming messages in order and owns its
job table, execution contexts, callback relay, artifact locator, volumes and
git service. Its child goroutines carry transfers, streams and job IO. Shared
control state is accessed through short synchronized methods; a filesystem
request never blocks the connection's message reader while holding its state.
The Host log has one writer goroutine, joined and flushed on shutdown.

Finite filesystem requests, working-tree requests and computations, and syncs
acquire the semaphores defined by [Load](../execution/runner.md#load), then
perform their work in the owning goroutine. Waiting is cancellable. Native
calls, backend commands, user and network streams, and local command
connections do not hold these permits for their caller-controlled lifetimes.
They retain their independent backpressure and the runner's no-load-refusal
rule. Load also owns descriptor exhaustion and process-start retry behavior.

Each shell job owns a context, interpreter, output drains, edit recorder and
external process group (a Job Object on Windows). The patched `mvdan.cc/sh`
interpreter shares job ownership across pipeline stages, subshells, background
lists, substitutions, heredoc writers and reader adapters. After `Run`, the job
joins all descendants with the interpreter's `Wait` before closing retained
redirections, finishing output and finalizing edits. Failure or cancellation
first cancels the context and external processes, then joins. Handlers and IO
must cooperate with cancellation, including unopened process-substitution
FIFOs and blocked reads and writes.

Pipeline siblings must be able to run together: a writer waiting for a reader
must not consume the last slot needed to start that reader. There is no bounded
shell thread pool, nor a finite-Host-work permit held for the whole shell job.
System utilities are child processes, not embedded thread-local utilities.
The shell's platform and utility limitations, including the open questions
about utility edit tracking and availability, belong to
[Shell jobs](../execution/runner.md#shell-jobs).

Command-alias mode serves one invocation with its own cancellation and joined
IO, using the same scheduler and command transport.

### demi-browser and demi-file

`demi-file` runs one goroutine per invocation through `internal/commandsdk`.
File mutations pass one `gates.Serial`; the invocation performs disk work and
output publication while holding its permit, outside any state mutex.

`demi-browser` routes invocations to a per-conversation browser owner whose
phase is absent, starting, ready or closing. A running browser owns its tab
registry, capture channel and live hub. Retirement cancels and joins all of
those owners and reaps Chrome. Different conversations and different tabs can
make progress concurrently; same-tab conflicts retain the browser's `tab_busy`
behavior. Large DevTools messages are decoded by the connection's owned
transport goroutines under its message and queue bounds. Archive extraction,
image encoding/decoding, hashing, process-table scans and output publication
run in their owning operation, without holding a state mutex.

### demi-claude-code and the command-sdk

`demi-claude-code` owns installation, executable hashing and file writes in its
invocation goroutines, with installation changes serialized. `internal/commandsdk`
owns one goroutine per invocation and cancels and joins invocations with their
connection; it does not choose a separate scheduler.

A native service owns one explicit HTTP/2 connection, created through
`net/http`'s `Transport.NewClientConn`, and its IO lifetime. Child stdin/stdout
carry protocol bytes; stderr is drained independently. Per-invocation contexts,
bounded output and stream flow control prevent a stalled invocation from
stalling other streams. The SDK configures the 64 KiB stream window,
concurrent-stream limit and early-reset guard according to the
[native runtime](../execution/native-runtime.md). A service abort resets the
stream; it does not promise a particular reset code. Connection/process owners
close bodies and pipes, join work and reap the process on shutdown. Shared
HTTP clients belong to their service owners, not to an OS thread.

### Machine manager

The manager owns the socket server and its request goroutines. One worker per
device owns its sandbox, runs device operations in order and takes sandbox exit
between operations. A fair semaphore admits device operations; reconcile and
shutdown take all its permits. File work, fsync, mounts, loop and freeze ioctls,
sparse copies, hashing and firewall updates run within admitted operations.
An admitted device operation is never cancelled by its requester's departure;
shutdown stops admission and drains device queues.

Namespace work has one entry point. It starts an owned goroutine that calls
`runtime.LockOSThread`, unshares `CLONE_FS`, creates or enters the namespace,
does the whole job and returns **without** calling `runtime.UnlockOSThread`.
The runtime then terminates that thread, including on failure. No subsequent
job may inherit the altered namespace. Ordinary storage work needs no locked
thread. Namespace-bound sockets and file descriptors are closed by that job.

Recovery enters the saved mount namespace on that locked thread and starts
`/proc/self/exe` from that same goroutine through `os.StartProcess` or
`os/exec`. The child runtime starts inside the namespace, so all its threads
and children inherit it. Starting in another goroutine, entering only in the
child's `main`, or manually forking and resuming Go code does not satisfy this
contract. The owner waits for the recovery child; if a parent-death signal is
used, the creating thread stays alive until the child finishes.

Network namespace jobs create netlink handles after entry and close them before
returning; they do not use a helper whose failed namespace restoration could
return a changed thread to the scheduler. nftables uses `WithNetNSFd` to select
its namespace explicitly. External tools such as runsc and filesystem tools
have explicit process owners that terminate when required and always reap
them; losing a Go reference never kills a process.

A checkpoint's frozen window is one synchronous freeze/copy/thaw job. It
registers deferred thaw before attempting freeze and handles thaw failure under
the storage recovery policy. Cancellation cannot interrupt it halfway through.
Defers cannot survive process death; durable namespace and filesystem recovery
remain necessary.

## The user shard

One `Shard` holds everything belonging to one user. Its mutable state is behind
one `sync.Mutex`, with short critical sections. A check and the state change
it guards happen in the same critical section. For example, a transfer checks
that transfers are open and registers itself atomically, including while it
waits for admission, so a concurrent switch cannot miss it. The site documents
the invariant, and a test interleaves a competing transition there.

No IO, gate acquisition, channel operation, callback or join runs under the
shard mutex. Capture the needed values and actions, unlock, do the work, then
recheck the relevant state when applying its result. A wait can invalidate an
earlier target or ownership check. A gate protects the longer operation; a
mutex protects only its short decisions. Returning a mutable map or component
pointer for unsynchronized use would defeat the boundary.

**Placement.** Decisions and per-user state live in the shard; bytes and
cross-user services live at the edge.

| In the user's shard | At the edge or in shared services |
| --- | --- |
| File gates, transfer admission, idle watches, target switch, archive, detach and per-user retention | Download, upload, user-stream, pipe and expose bytes with stall and idle timers |
| Agent trees and sessions | HTTP routing, session gate, body limits and static files |
| Runner connection owners and pipe records | Unpaired runners, which have no user |
| Cloud machine and reset intent | Cross-user Cloud capacity and machine-manager client |
| Live expose records, titles, forks, command router and request rate limit | Storage, vault, providers, catalogs, credential refresh gates and login limiter |
| Page synchronization channels and their changed-part sets | Registry of open channels, on which any goroutine may mark a change |

**Crossing the boundary.** The edge calls shard methods directly, per request,
WebSocket upgrade, stream admission or change mark. There are no shard threads,
mailboxes or `call` closures, and no scheduling hop per agent tool call.

- Requests reach only their owner's shard after the edge checks ownership.
  Arguments and results are owned values, immutable views or opaque handles.
- Accepted conversation, synchronization and runner sockets transfer to their
  user-owned connection goroutines, which handle messages in order. The edge
  reads a runner's first message: a known token selects its owner's shard;
  an unpaired runner stays at the edge.
- Transfers, user streams and exposes return leases and pipe ends. The edge
  owns byte copying and defers release. The shard can revoke admission and
  interrupt the pipe without waiting for a stalled client to cooperate.
- Whoever commits a page-visible change marks the affected part in the
  registry of open synchronization channels, including shared services and
  other users changing shared provider entries. Each channel accumulates the
  mark and wakes its owner to read the part through the shard
  ([Page synchronization](../backend/backend.md#page-synchronization)).

**Host admission.** All conversation Host work uses the conversation's
[host access](../execution/sessions-and-targets.md#host-operations). In
particular, release the file gate before waiting for Cloud admission and
recheck after taking it again; never enter a file gate while already holding
its lease. A dispatched operation runs once, never retries after a reset or
lost device. Transitions first close new transfer/stream admission atomically,
then end registered operations and await admission release outside the mutex,
then reserve the file gate according to the transition's busy/conflict policy.
They must not wait for a reservation while still holding the transfers it
needs. Concurrent transitions cannot reopen admission until all closing
transitions have finished. The Host operations document owns the detailed
wake, activity, stall and lifecycle rules.

**Call completion.** Request cancellation affects waits, not commits. Once a
transition starts its commit, storage and subsequent bookkeeping finish even
if the requester leaves: a successful target write still restarts its idle
watch. Switch, archive and detach belong to shard-owned work once they hold
the conversation. Shutdown joins this work before destroying its state.

**Users and processes.** A multi-worker deployment pins each user to one worker
process ([Deployment and user ownership](../backend/backend.md#deployment-and-user-ownership)).
Within that process, Go may run the user's goroutines on any thread; the shard
mutex supplies serialization.

**Composition.** Components own their state and `Shard` combines them. No
component holds a reference back to another. A component needing coordinated
operations, such as Cloud reset across conversations, defines a narrow Go
interface such as `CloudShard` in the consuming package. Its operations accept
that interface; `internal/backend/usershard` implements it. Lower packages do
not import the concrete `Shard`. Component state participating in shard
invariants uses the shard's critical section; independently owned services
provide their own synchronized methods. Interfaces do not expose mutable shard
state or invite recursive acquisition of its mutex.

## Locks

The runner's device token changes only when claimed but is read by every pipe
request. Registration publishes an immutable snapshot through `atomic.Pointer`;
readers load it without holding up registration. A snapshot includes a change
channel for consumers that need to wait for a newer value.

| Need | Tool | Examples |
| --- | --- | --- |
| Short shared-state decisions | `sync.Mutex`, or exclusive ownership by a goroutine | Shard admission; connection job table; browser tab owner |
| Serialize a whole operation, including waits | Named `internal/gates` gate backed by `semaphore.Weighted` | Conversation file gate, file mutations, credential refresh |
| Read-mostly state | Immutable snapshot through `atomic.Pointer` | Device token, tab lookup snapshot, activity state |

**Gates.** `gates.Activity` uses FIFO weighted semaphore admission. An activity
lease takes one permit; a reservation takes all permits. A waiting reservation
holds back later entrants, and a nonblocking reservation succeeds only with
no holders or queued predecessors. Its published snapshot includes demand,
maintenance, reservation state and when demand last ended; idle watches use
that timestamp. `gates.Serial` admits one holder in arrival order;
`gates.KeyedSerial` does so per key, allowing unrelated keys to proceed.
Key entries remain owned while holders or waiters use them. All leases,
reservations and permits have an idempotent `Release` and may be explicitly
handed to another goroutine.

**Snapshots and notifications.** Published values, including referenced maps
and slices, never mutate. Publish the new value and replace its notification
channel, then close the old channel to announce change. A reader obtains the
value and its channel from one immutable publication, tests the predicate,
then waits outside locks and reloads after notification. Writers serialize the
publication swap under the state mutex; each writer closes only the old channel
it replaced, after unlocking. A delayed notification is harmless because
readers reload the latest publication. Notifications may coalesce; the
snapshot retains the current state and last-demand timestamp.

**Critical sections.** Use `sync.Mutex` only for short state changes. Each use
states what it protects, including shared pending runner claims, blob-use
records and watcher changed-path sets. No lock is held across a blocking call
or channel operation, including sends, receives and notification delivery.
When locks nest, document one order at the site and never reverse it. A gate
must not call back into a shard while holding its own lock. Read-mostly data
uses snapshots rather than `sync.RWMutex`; serialization across waits uses a
gate rather than a long-held mutex.

Events are collected under the state lock and delivered in order after unlock.
An emitter takes a listener out of its slot while invoking it so it may call
back into the emitter. Subscriptions have explicit release, deferred by their
owner; losing a reference does not unsubscribe.

**Queues.** Every channel and queue is bounded, including result and owner
mailboxes. An unbuffered channel is a zero-capacity bound. The owner states
what full means: a conversation socket's outbox closes a lagging connection,
whose client reconnects to the running session
([Frame protocol](../agent/runtime.md#frame-protocol)); the runner's connection
stops reading until it has room ([Runner](../execution/runner.md)). Bound
producer concurrency too; queue capacity alone does not bound blocked senders.

One queue is unbounded on purpose: the backend's per-job and per-process
output queue on a runner link (`internal/backend/remotehost`). Output is
lossless, so a full queue could only stop the link, and one unread job would
then stall every other job and request on that runner. Its consumer drains
it as it routes; the runner's own output limits bound what one job sends.

## Blocking work

A login acquires the backend's CPU-sized hash semaphore and runs argon2id in
its request goroutine. Other goroutines keep serving requests. Disk IO,
serialization and CPU work run in the owning operation without a state lock;
they do not require a helper goroutine solely because they block. When work
needs parallel execution, its owner starts and joins bounded workers.

The backend's hash, writer and cold-read limits, the runner's Load limits,
command operation gates and the machine manager's operation semaphore are the
bounds described above.
Neither the scheduler nor the number of OS threads is an admission policy.
Cancellation of a context does not interrupt arbitrary IO: use context-aware
APIs or have the resource owner close or set deadlines on the actual pipe or
connection to unblock reads and writes.

### Databases

`internal/backend/database` owns SQL through `database/sql` with the pure-Go
`modernc.org/sqlite` driver. Each writable database has one `sql.DB`, configured with
`SetMaxOpenConns(1)` and `SetMaxIdleConns(1)`. Every store operation begins a
`sql.Tx`; every statement in that operation uses the transaction, never the
parent `sql.DB`, which would wait for the connection the transaction holds.
Defer rollback, close result sets, and commit or roll back before returning.
Transactions do not wait on a model or network operation. No database thread
or `runtime.LockOSThread` is needed.

Stable lazy conversation handles use an explicit global LRU of at most 64 open
conversation writers. In-flight operations pin their writer; eviction drains
it and closes the database before reusing the slot. If every slot is pinned,
new admission waits with its context. Shutdown drains and closes all writers.
Per-database pool sizes and idle lifetimes do not enforce the global limit.
Cold history and summary reads use short-lived read-only connections, never
create a missing database, and at most 64 are open at once across the
backend, so a page that reads hundreds of summaries cannot exhaust file
descriptors; a read waits for a free connection with its context. Their result sets, transactions and connections are closed in the
acquiring scope. [Storage](../backend/storage.md) owns schema, transaction,
blob publication, SQLite configuration and lifecycle coordination rules.

## Cancellation and cleanup

If the browser closes the download tab, the edge stops its copy and releases
the lease. The admission owner fails the pipe and releases the file gate. If
an archive ends the download first, the shard revokes its admission and the
edge cuts the response short; it must never appear complete.

- **Contexts follow ownership.** `context.Context` is the first parameter of
  every function that waits. Owners derive child contexts and defer their
  cancellation. Request departure also cancels request-scoped waits; durable
  work belongs to the component that must finish it. Cancellation errors use
  `context.Canceled` or `context.DeadlineExceeded`, compared with `errors.Is`.
- **Every goroutine has an owner.** Register work before starting it with
  `sync.WaitGroup.Go` or an `errgroup`; shutdown stops new work, cancels, then
  waits. An error group does not replace resource cleanup. Timers belong to
  component workers, such as a session persister, yield driver, idle watch or
  expiry worker; records do not store timer handles. Workers stop timers on
  change, cancellation and exit.
- **Release explicitly.** Defer idempotent `Release` where each lease,
  reservation or permit is acquired, on success, failure and cancellation.
  A handoff states which receiving goroutine now defers release; failure to
  hand off leaves the sender responsible. Owners also close files, listeners,
  pipes and subscriptions. Tests audit outstanding handles after owners have
  joined, even if revocation already freed the underlying admission.
  `runtime.AddCleanup` is not a correctness mechanism.
- **Cancel waits, not commits.** Provider streams, tools, backoff and admission
  waits take the cancellable context. At the commit boundary, use
  `context.WithoutCancel` for the storage operation and all required
  bookkeeping and cleanup, and await completion. For SQL this begins before
  `BeginTx`, so request cancellation cannot roll back the transaction behind
  the commit. A commit still reports actual IO errors; it is never abandoned
  by racing its result against request cancellation. Its owner waits for it
  during shutdown.
- **Select branches leave no work behind.** A Go `select` does not stop another
  goroutine. A losing branch either owns no unfinished work or explicitly
  cancels and joins its worker. Work outliving its waiter remains registered
  with its component owner.
- **Asynchronous release is explicit.** Cleanup that waits has `Close(ctx)`;
  `Release` is not a disguised join. Owners invoke and await close with a
  cleanup context that remains usable after request cancellation. A retained
  Claude Code process moves into a run and back to its runtime only where it
  may be kept; close sends SIGTERM, waits five seconds, then sends SIGKILL and
  reaps it. There is no drop fallback: the owner executes escalation and join.
- **Answers and settled state differ.** One result uses a capacity-one result
  channel or a closed completion channel with an immutable result, so a
  departed waiter cannot strand the sender. Settled state uses an immutable
  snapshot and change notification.
- **Ordering is a protocol.** The session worker records an abort, and the
  abort operation waits for its acknowledgement. Writes whose loss matters
  are awaited: a usage ledger row is written before the response reaches the
  agent, and failure is logged. Every ignored error has a comment explaining
  why ignoring it is safe.

Machine-manager device operations drain instead of being cancelled. Each
program's shutdown order is defined by its own document, including
[Backend](../backend/backend.md). Backend shutdown ends transfers and user
streams before saving and stopping Clouds, joins request/copy and shard-owned
work before disposing state, and audits leases only after their owners exit.

## Tests and time

- **Fake time in process.** Timer scenarios run inside `testing/synctest`
  bubbles. All participating goroutines are created inside the bubble and
  joined there. Fake time advances timers and `time.Now`, so a one-hour idle
  window and its timestamps advance together. Use exact deadlines and
  `synctest.Wait` or observable events to establish quiescence, never a sleep
  to guess that another goroutine ran.
- **Real IO uses real time.** A test waiting on another process uses real time
  and short configured windows, synchronized by protocol events. Real network
  and filesystem/SQLite IO are not virtualized by synctest; tests coupling
  timers to those operations use real time or replace the IO with a controlled
  fake. Timeouts bound failures, not successful synchronization.
- **Wall-time-only tests.** Ordinary timer tests need no separate fake clock.
  Tests that move only wall time, such as an expiry scenario, may inject a
  settable clock at that boundary; duration measurement uses Go's monotonic
  time through `time.Time` and `time.Since`.
- **Same backend, same ownership.** In-process backend tests use the same
  mutex shard and owner lifecycle in the bubble, with controlled external
  dependencies. There is no test-only shard scheduler.
- **Leaks and races are failures.** Every package starting goroutines uses
  `go.uber.org/goleak`, and shutdown tests audit outstanding leases as well as
  joined workers. Run the race detector on macOS with `CGO_ENABLED=0`; Linux
  race test binaries use `CGO_ENABLED=1 -tags netgo,osusergo`. Product builds
  always use `CGO_ENABLED=0`.
- **Real programs are built binaries.** Integration tests start the Go binaries
  built for their test run, not a development server or an implicit compiler
  invocation ([Validation](../delivery/builds-and-releases.md#validation)).
