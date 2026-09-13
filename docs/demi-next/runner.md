# Demi Next: Runner

The native `demi-runner` executable turns a machine into an execution target.
It owns filesystem/process RPC, shell jobs, local command dispatch and its outbound
backend connection. It has no agent or model implementation. The detailed native
service contract is in [native-runtime.md](native-runtime.md); declarations are in
[commands.md](commands.md).

## Connection and identity

`crates/runner/src/connection/` owns the registration's outbound WebSocket and
validates MessagePack messages using bindings generated from the TypeScript
runner-protocol package. Runner owns the associated jobs and command contexts.
A connected user device pairs with the backend, persists its device token in private installation state,
and reconnects using that token. A managed guest receives its token at boot and
keeps it in temporary state. The backend owns pairing policy and user ownership.

`state.rs` selects an installation by normalized backend URL, holds its OS lock,
and publishes active endpoint metadata. Independent registrations have independent
credentials, caches, endpoints and release selection. `management.rs` implements
status and draining over the authenticated local endpoint. Draining closes job
admission, waits for active work, and releases the installation lock.

## Host operations

`host.rs` dispatches filesystem and raw process requests independently of running
shell jobs. `fs.rs` validates paths and performs machine-local IO. `process.rs`
owns child IO and cancellation. Raw spawn with an explicit environment replaces
the device environment; `inheritEnv: true` extends it, and undefined entries remove
inherited values. Omitting the environment uses the device environment. Provider
CLI assembly explicitly requests inheritance; the runner has no provider logic.

A spawn or job receives an opaque execution context. Contexts bind callbacks to
the live job, registration and exact manifest. They are released on completion,
cancellation and connection loss. The runner overrides its owned context variables
so callers cannot substitute another job’s attribution.

## Shell jobs

Brush executes inside the resident runner process. Shell jobs own their cwd,
environment, IO and execution state; a shell job does not launch another runner
process. `crates/runner/src/shell/` registers and adapts embedded standard
utilities and registers declared command roots as builtins. Declared builtins
call the runner's dispatcher directly. External programs such as git, Python and Node remain child
processes. An external program calling a declared command uses the forwarding
executable and local endpoint described in `command-client.md`.

The job’s foreground exit status and final cwd are reported to the backend.
Subsequent jobs start a fresh login shell at that cwd. Brush loads the system
profile and the first readable user login profile, so toolchain installers can
add environment setup for subsequent jobs. Shell variables and functions do not
persist except through those files. After profile loading, the runner restores
its owned execution context, prepends its command aliases to the resulting PATH
and restores the requested cwd. The job joins its background tasks before reporting completion.
For example, `(sleep 2; echo done) & echo started` emits `started`, remains running,
then emits `done`. Tool timeout returns a handle; it does not stop the job.
Background tasks are job-owned. Brush does not expose their OS PIDs through `$!`.

The runner owns cancellation of the whole job, including background work,
command invocations, IO and external children. Brush and embedded utilities
cooperate with cancellation inside the runner process. Cancelling one job must
preserve the runner and unrelated jobs. Native builtins use invocation-local
state and IO rather than process-global cwd, environment or exit.

`shell/job.rs` owns the job's input/output pumps and completion task.
`scope.rs` tracks interpreter tasks, utility workers and external children until
they release their resources. Interpreter steps and utility IO, sleeps and
unbounded evaluation loops check the job's cancellation token. Unix pipe IO
checks readiness before bounded reads/writes; Windows cancels pending synchronous
IO on the owning worker. External children use a Unix process group or Windows
Job Object and are reaped before completion. After the foreground script and
background jobs finish, the owner joins process substitutions and other work
tracked by the scope. Failure or cancellation stops the remaining work before
joining it; successful completion preserves all produced output.

`declared.rs` adapts shell descriptors to pull-driven command-service input and
bounded output records, then calls the shared dispatcher. A declared command
reads stdin only when its handler requests it. External forwarding and direct
builtins share parsing, validation, native-service acquisition and callback
routing.

A cancellation request is not proof that execution stopped. Job completion waits
for local work and resource cleanup; unconfirmed remote execution is reported as
an unknown outcome. Cancellation does not undo completed side effects. The runner
continues serving control requests while a job produces output, waits for input
or is being cancelled.

## Command lifetime

The server or SDK agent host program determines its command definitions and
implementation catalog at startup. They remain fixed during that program's
lifetime; changing them requires restarting that program. There is no live
command-update broadcast across running shells.

Builtin bindings and external forwarding use the same execution context.
Releasing that context releases its bindings and references to command services.
Brush may be forked to provide the necessary registration and removal APIs.
Connection loss invalidates its execution contexts and cancels their work.
Reconnection creates fresh contexts; it does not resume streams or replay
commands. A network reconnect does not change the host program's command set.
Restarting the host program establishes a new lifetime with its startup catalog.

## Pipes and output

Jobs retain full output in device-local files and send bounded output views.
`shell_status` returns new output since the preceding view. Consumers needing the
complete output read its retained file or accumulate views. Binary command payloads
remain bytes through local HTTP/2 and backend HTTP pipe transfers.

`pipes.rs` owns cancellable direct HTTP transfers. Backend `runner/pipes.ts` owns
the broker’s rendezvous and lifetime. EOF ends input; cancellation aborts work.
Live input is chunked and demand-driven. A command that does not read stdin does
not consume the next pipeline or interactive input. The commands module forwards
callbacks, owns their running hints and routes resident service invocations.
It also owns artifact downloads, verification, caching and service processes;
the shared command-service library supplies communication only.

## Managed guests

`init.rs` handles Linux PID 1 initialization: validated kernel parameters,
filesystem mounts, network configuration, temporary state, child reaping and
permanent privilege drop. `volumes.rs` handles volume usage and growth.
Filesystem RPC and jobs use the same guest account after initialization. The VM
is the security boundary. See [managed-hosts.md](managed-hosts.md).

## Build and checks

`scripts/native/build.ts` builds runner and command-package executables for the
six target triples. `scripts/native/release-runner.ts` verifies all bytes, creates an
immutable hash-named release and atomically advances its manifest.

Rust integration tests cover Host IO, jobs, local endpoints, dispatch, streaming,
connection lifetime and shell behavior. TypeScript tests start the actual Rust
runner through `host-remote/testing`; `LocalHost` is a test-only Node fixture. Provider
integration tests use mock upstreams. The native CI workflow executes the same tests on each supported platform.
