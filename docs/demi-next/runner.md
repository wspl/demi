# Demi Next: Runner

The native `demi-runner` executable turns a machine into an execution target.
It owns filesystem/process RPC, shell jobs, local command dispatch and its outbound
backend connection. It has no agent or model implementation. The detailed native
service contract is in [native-runtime.md](native-runtime.md); declarations are in
[commands.md](commands.md).

## Connection and identity

`runner/src/mode.rs` owns a registration’s connection, jobs and command contexts.
`connection.rs` establishes the outbound WebSocket and validates MessagePack
messages against the generated runner-protocol contract. A connected user device
pairs with the backend, persists its device token in private installation state,
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

`tasks.rs` starts a fresh runner process in `shell-job` mode. `shell.rs` creates a
brush shell with explicit cwd, environment and IO, and registers embedded standard
utilities from `native-utils`. External programs such as git, Python and Node
resolve on the device. Declared command roots use aliases to the runner executable
and dispatch through its local HTTP/2 endpoint.

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

Cancellation terminates the entire job process tree through Unix process groups
or Windows Job Objects. Connection loss closes jobs, callback contexts, transfers
and resident services. The runner continues serving control requests while a job
produces output or waits for input.

## Pipes and output

Jobs retain full output in device-local files and send bounded output views.
`shell_status` returns new output since the preceding view. Consumers needing the
complete output read its retained file or accumulate views. Binary command payloads
remain bytes through local HTTP/2 and backend HTTP pipe transfers.

`pipes.rs` owns cancellable direct HTTP transfers. Backend `runner/pipes.ts` owns
the broker’s rendezvous and lifetime. EOF ends input; cancellation aborts work.
Live input is chunked and demand-driven. A command that does not read stdin does
not consume the next pipeline or interactive input. `rpc.rs` forwards callbacks
and owns their running hints; `native.rs` routes resident service invocations.

## Managed guests

`init.rs` handles Linux PID 1 initialization: validated kernel parameters,
filesystem mounts, network configuration, temporary state, child reaping and
permanent privilege drop. `volumes.rs` handles volume usage and growth.
Filesystem RPC and jobs use the same guest account after initialization. The VM
is the security boundary. See [managed-hosts.md](managed-hosts.md).

## Build and checks

`scripts/native/build.ts` builds runner and command-package executables for the
six target triples. `runner/runtime/release.ts` verifies all bytes, creates an
immutable hash-named release and atomically advances its manifest.

Rust integration tests cover Host IO, jobs, local endpoints, dispatch, streaming,
connection lifetime and shell behavior. TypeScript tests start the actual Rust
runner through `runner/testing`; `LocalHost` is a test-only Node fixture. Provider
integration tests use mock upstreams. Platform runtime evidence and remaining
acceptance work belong in [progress.md](progress.md).
