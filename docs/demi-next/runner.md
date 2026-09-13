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

## Shell jobs

A shell job owns its working directory, environment, IO, and asynchronous work.
Brush runs inside the resident runner process. Declared roots call the shared
command dispatcher; external tools such as Git, Python, and Node run as child
processes.

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
