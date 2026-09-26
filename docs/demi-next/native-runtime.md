# Native command execution

This document defines native command execution: binding a command to a release,
installing its executable, and running it in a shared service. The execution
contract comes first; protocol, publication, and build requirements follow.

The runner is a Go process with an embedded shell interpreter. Native command
algorithms run in separate executables that stay running to serve multiple calls.
These *resident services* can be released independently of the runner. The runner
embeds no JavaScript engine.

All native Demi commands belong to the `demi.builtin` package. SDK applications
can supply additional packages using the same contract. Shell utilities and
application callbacks have their own owners.

## One command, from declaration to result

The following sections trace `demi file patch` on a laptop. The application
selects the command's release, the runner installs the laptop executable, and a
resident service applies the patch to the local file.

The process diagram shows the native execution path. Each box is a process;
arrows show requests, labeled with their contents and transport. Response paths
are omitted. The runner and command service execute on the laptop.

```text
Native command execution: process communication

+----------------------------+
| Application process        |
| Defines commands and jobs  |
+----------------------------+
              |
              | Job + pinned manifest
              | MessagePack / WebSocket
              v
+----------------------------+
| Runner process             |
| Runs shell and dispatches  |
+----------------------------+
              |
              | Native invocation
              | HTTP/2 over stdio
              v
+----------------------------+
| Command service process    |
| Executes native operations |
+----------------------------+
```

The patch handler works beside the file. The caller supplies the patch and
receives output and an exit status; the backend does not need the original file.
Keeping algorithms in an independent service allows command releases without
rebuilding the runner.

For file mutations the runner also supplies the job-owned `EditContext`. The
service uses the shared recorder when publishing or restoring file contents;
the runner reads its journal at job completion. The recording contract, limits
and lifetime are defined in [Edit tracking](edit-tracking.md).

The responsibility boundaries are:

| Owner | Responsibility |
| --- | --- |
| Agent command tree | Define names, help, argument schemas, logical bindings, and callback handlers. |
| Execution adapter (`host-remote`) | Validate bindings against the startup catalog and resolve artifact locations. |
| Backend artifact module | Publish complete releases and authorize artifact downloads. No command algorithms. |
| Runner commands module | Validate dispatch, install artifacts, own service processes, and route invocations. |
| Shared command-service SDK | Handle framing, HTTP/2, byte IO, and cancellation over supplied transport. |
| Native package | Implement operations, validate their arguments, and release operation resources. |

The shared SDK owns no artifact or process management. Shell builtins and external
command clients use the same dispatcher, which supplies validated operation
metadata to the native service. Application callbacks return to the embedding
application instead. The native service never receives raw CLI requests.

Related contracts define the surrounding behavior:

- [Command declarations](commands.md): CLI parsing and manifest semantics.
- [Local forwarding](commands.md#external-command-clients): external clients and endpoint access.
- [Runner jobs](runner.md#shell-jobs): the interpreter, utilities, profiles, and whole-job cleanup.
- [Package boundaries](../package-boundaries.md#source-organization): source module ownership.

## Bind an exact package

The command declaration identifies an operation; the startup catalog selects
its release. These terms have distinct meanings:

| Term | Meaning in the patch example |
| --- | --- |
| Declaration | Names package `demi.builtin` and operation `file.patch`. |
| Catalog | Maps `demi.builtin` to one selected release descriptor. |
| Descriptor | Identifies that release, its operations, and each platform executable. |
| Manifest | Pins the descriptor hash and operation for the job. |
| Artifact | The executable bytes selected for the laptop's platform. |

Pinning the manifest prevents a later release or download URL from changing what
an existing job executes. The embedding application supplies the catalog and
artifact resolver through its execution adapter:

```ts
const shellEnvironment = createRemoteShellEnvironmentFactory({
  packages: runtimePackageCatalog,
  resolveArtifact,
});

const server = new AgentServer({
  agent,
  providers,
  shellEnvironment,
  store,
});
```

`AgentHarness.commands` owns the tree. The patch leaf supplies this binding;
its help and argument schemas follow the [command declaration contract](commands.md):

```ts
{
  name: 'patch',
  kind: 'native',
  binding: { package: 'demi.builtin', operation: 'file.patch' },
}
```

Each package ID resolves to exactly one immutable descriptor within a factory
catalog. Duplicate IDs, incomplete packages, and unknown operations fail
initialization. Declarations do not import compiled-in release descriptors.

The factory context supplies the invoking Host, command registry, session identity,
and command storage. Each invocation retains its authorized context even when
several environments share a catalog. Core agent initialization has no
object-store dependency; package selection does not belong in model settings.

A release consists of its descriptor and one executable for each target it
carries; a published release carries all six of the
[release target matrix](#publish-a-complete-release). The descriptor contains:

| Descriptor field | Meaning |
| --- | --- |
| `id` | Stable namespaced identity, such as `demi.builtin`. |
| `version` | Human-readable release version, immutable within its publisher. |
| `protocolVersion` | Command-service wire major version. |
| `operations` | Unique operation IDs supplied by the package. |
| `targets` | The target triples the release carries, each with executable SHA-256 and byte size. |

The descriptor hash is the SHA-256 of its canonical JSON. Paths, object keys,
and URLs locate artifacts but do not participate in package identity.

The runner independently validates descriptors and references. Publication
rejects reused id/version pairs with different content, unsupported wire versions,
and missing targets. An input-schema change affecting native behavior requires a
matching release. The runtime does not repair arguments or fall back to another
version.

Command definitions and catalogs remain fixed for the embedding program's
lifetime. New definitions take effect when that program restarts. The catalog
does not support live updates. Existing contexts retain their pinned binding.
Connection loss and context disposal follow the
[runner command lifetime](runner.md#command-lifetime).

## Install the selected executable

For the patch example, an arm64 Mac selects `aarch64-apple-darwin` from the
pinned descriptor. The runner reuses a verified cache entry or asks the artifact
resolver for that executable's exact digest and size.

In a backend deployment, the backend authorizes the package reference and returns
an HTTPS location. The runner downloads directly from storage. Credentials remain
in the backend.

The resolver supports these deployment choices:

- Private storage: signed URLs without changes to bucket policy. The default
  backend resolver issues signed GET URLs valid for five minutes.
- Public storage: public HTTPS artifact URLs.
- SDK applications: installed files or application-supplied hosting.

The runner obtains URLs on demand. URLs never become package identity or permanent
manifest fields. An expired URL can be refreshed for the same digest. Other
download failures do not count as expiration.

For a cache miss, the runner completes these steps:

1. Download to a temporary file, enforcing the declared size.
2. Verify the size and SHA-256.
3. Apply executable permissions where required and publish the verified file
   atomically into the cache.

Concurrent callers share one download per digest. Cancelling one caller preserves
a download still needed by another. If the download fails or is cancelled, the
runner releases the response and removes the temporary file. The runner never
executes partial or mismatched files.

## Invoke and retire a service

The runner launches the verified executable with `--command-service`. The first
patch call checks the service before invoking `file.patch`. Later calls reuse
the process and connection, with one HTTP/2 stream per invocation.

The sequence below starts with an installed executable. Time runs downward;
arrows show interactions between the runner and service. Each invocation uses
its own stream on the reused connection.

```text
Native command execution: startup and successful calls

Runner                                  Command service
  |                                           |
  |--- Start process ------------------------>|
  |--- GET /v1/info ------------------------->|
  |<-- Protocol version + operations ---------|
  |                                           |
  |    Validate against pinned descriptor     |
  |                                           |
  |--- POST /v1/invoke ---------------------->|
  |                                           | Execute
  |<-- stdout / stderr records ---------------|
  |<-- completion record ---------------------|
  |                                           |
  |--- Next invocation ---------------------->|
  |                                           | Reuse process
  :                                           :
```

Calls share a resident service only when all three values match:

- Runner registration
- Execution user and security context
- Artifact digest

Concurrent startup requests within that scope share one launch. Different
immutable artifacts can run side by side. No service crosses a privilege boundary.
Reuse spreads process and connection startup costs across calls. It does not
establish a performance claim for individual commands.

The runner owns the child process and all three pipes. It establishes HTTP/2
directly over stdin/stdout, with the runner as client and service as server.
The transport needs no port listener, TLS, HTTP/1 upgrade, or gRPC.

Before admitting calls, the runner checks `GET /v1/info` against the pinned wire
version and operation set. The runner rejects startup on timeout, missing
operations, extra operations, or version mismatch. The verified file establishes
executable identity, so the handshake does not require the service to report its
own hash.

Each invocation owns its cwd, environment, identity, input, output, and cancellation.
Handlers must not change process-global cwd or environment. Concurrent operations,
including blocking or CPU-bound work, must allow connection processing and
cancellation to continue. File algorithms belong in the native package, not in
the runner or shared SDK.

| Event | Required result |
| --- | --- |
| Normal completion | Deliver output and completion; release invocation resources. Keep the service available. |
| Invocation cancelled | Cancel its handler and release its resources. Preserve unrelated calls and the connection. |
| Handler exceeds cancellation grace | Report a service fault and retire the process. |
| Service crashes or corrupts the protocol | Fail affected invocations. Do not automatically replay potentially completed side effects. |
| Execution context is disposed | Release its bindings and service references. Live contexts, invocations, or held conversation state keep their service owned. |
| No service owners remain | Request shutdown, drain the service, close its transports, and reap the child. |

The runner allows **6 seconds** for the shutdown request and process exit.
If shutdown fails or exceeds that deadline, the runner terminates and reaps the
child. Startup failures also release the process and its transports.

EOF ends input, not execution. Cancellation requires the handler to stop and
release its resources; resetting a stream or aborting a task is insufficient.
The [protocol limits](#validation-and-flow-control) set the cancellation grace.

For example, two jobs using the same artifact can share one service. Cancelling
one call normally leaves the other running. If its handler refuses to stop within
the grace period, retiring the faulty service also fails the other affected call.
The runner must report that failure rather than claim isolated cancellation.

Changing the startup catalog creates new pinned bindings, not an in-place
replacement of an executable serving an existing context.

## Command context

A command often needs to know more than its arguments: which conversation it
serves, who started it, and how to present results to the user. For example,
`demi browser open` keeps its tabs in the invoking conversation's browser, and
that browser starts Chrome in the user's time zone. These facts form the
command context, one value with one schema in `command-protocol`:

| Field | Meaning |
| --- | --- |
| `conversation` | The conversation the work belongs to. |
| `caller` | Who started the work: `agent`, with the agent `node`, or `user`, for a [user stream](#user-streams). |
| `locale` | The time zone, an IANA name, and the languages, BCP 47 tags in preference order, that the conversation's user's browser last reported ([User preferences](web-api.md#user-preferences)), or `UTC` and `en-US` until it reports them. |

The backend is the context's only source, and nothing reads it from
environment variables: a script can change those, and every program the job
runs inherits them.

```text
Backend: builds the context and keeps it in the job's or stream's record
   | job_start { context }, service_open { context }
   v
Runner: keeps it in the job's live execution context
   |-- native invocation { context, edits, cwd, env, args } --> command service
   `-- rpc_call { jobId } --> backend: the handler receives the record's context
```

- The backend builds the context when it starts a job or opens a user stream,
  and keeps it in that job's or stream's record. A job that `demi host shell`
  starts on another Host carries its invoking job's context.
- The runner keeps the context in the job's live execution context and writes
  it into every native invocation record. The invocation's `edits`, `cwd` and
  `env` stay separate: they describe the runner's resources and the process,
  not the origin of the work.
- An application callback names only its job. The backend gives the handler
  the context from its own record of that job
  ([Bind jobs to their caller](sessions-and-targets.md#bind-jobs-to-their-caller)).
- Only declared commands receive the context. Other programs the job runs,
  such as scripts, neither receive nor need it. The job environment carries
  only what a command alias needs to reach the runner: the local endpoint, the
  opaque context handle, `DEMI_HOME` and the aliases on `PATH`. The runner
  finds the job's context through the handle
  ([External command clients](commands.md#external-command-clients)).

A new context field changes the schema and the backend's construction of the
context; its consumers read it where they already receive the context.

## Conversation-scoped state

A native operation can keep state that must survive the shell job that created
it. The conversation browser keeps its tabs between shell jobs and agent turns.
That state belongs to the conversation, and the native package keeps it itself,
keyed by the conversation identity the runner supplies; there is no separate
resource handle, acquisition, or grant.

This section is the port through which every such capability attaches, and it
is deliberately the only one. The runtime provides mechanism, identity and
release, and no policy: it does not know what a tool holds, why, or for how
long. The backend provides one policy, the conversation idle rule in
[Conversation idle and Host resource release](resource-lifecycle.md), and no
mechanism specific to any tool. A tool that needs more than this port is
evidence that the port is missing a generic capability, not a reason to give
that tool a path of its own through the runner or the backend.

Native handlers find the conversation in the invocation's
[command context](#command-context). A script cannot change the context, so it
cannot select another conversation's state.

Calls from every conversation on a device share one resident service per
artifact and security context. The service separates state by conversation.
A service that holds conversation state stays resident: before retiring a
service with no remaining context owners, the runner asks it whether it still
holds any conversation, and keeps it when it does.

The service exposes `POST /v1/conversation` with the invocation framing and
cancellation contract below. Its operation is `release` or `status`, and its
trusted `conversation` field names the conversation; the local command client
cannot invoke it. `status` returns the conversations the service holds.
`release` ends everything the service holds for that conversation, awaits the
domain cleanup, and acknowledges; an unknown conversation is harmless to
release. The runner sends `release` when the backend sends the generic
`conversation_release` message for that conversation, and service shutdown
releases every held conversation. Failed cleanup retires the faulty service and
reports it, rather than claiming a successful release. Which events lead the
backend to send a release is defined in
[Conversation idle and Host resource release](resource-lifecycle.md).

### User streams

An operation can also serve the conversation's user directly, for as long as
the user's page stays open. For example, the
[live browser view](browser-live-view.md) streams pictures of the
conversation's browser tabs to the work panel and the user's input back to
them. It is the same conversation state the agent's `demi browser` commands
use, reached through the same port.

The application declares each user stream by name with a native binding, the
way a command leaf binds an operation: the `browser` stream binds
`demi.builtin` operation `browser.live`. The declarations are fixed with the
command tree for the program's lifetime, and a page can open only a declared
name.

When the user opens a stream, the backend asks the conversation's Host runner
for a [service stream](runner.md#service-streams). The runner invokes the
operation with `POST /v1/invoke`, the contract every command uses, in the
resident service that holds the conversation's state: the invocation uses the
same package binding as the conversation's jobs, so it reaches the same
service. Its [command context](#command-context) names the conversation and
a `user` caller, with the user's locale. Its `cwd` is the conversation's
directory and its environment is empty.

The same mechanism serves a one-shot user call: the backend opens a service
stream on an operation that finishes by itself, passes the operation's `args`
as a command's invocation does, asks for its JSON result, sends no input, and
reads the output to its end. A call whose invocation exits nonzero fails with
what the operation wrote to its standard error, which a JSON invocation writes
as `{ error: { code, message, details } }`. The failure also retains the bounded
standard output, so a package whose contract puts its error document there can
validate and report it without losing its cause
([Service streams](runner.md#service-streams)). The
[conversation browser tab routes](web-api.md#conversation-browser-tabs) call
`browser.tabs`, `browser.open` and `browser.close` this way, so a tab the user
opens is the tab the agent's `open` would have made, created by `user`.

The invocation's input is the bytes the page sends and its output is the bytes
the page receives. The operation frames its own messages; the runner and the
backend forward bytes without reading them. The stream ends when either side
ends it: the runner cancels the invocation when a pipe fails, and the
invocation's completion ends the page's connection.

## Invocation protocol

Protocol version 1 uses direct HTTP/2 requests. The shared SDK owns process IO.
Handlers use logical output writers. Executable stdout contains only protocol
bytes. Process stderr carries service diagnostics, which the runner drains
independently into the [Host's log](runner.md#host-log) line by line; the tail
of a service that fails still accompanies its failure.

| Request | Purpose |
| --- | --- |
| `GET /v1/info` | Return wire version and operation IDs before invocations. |
| `POST /v1/invoke` | Run one invocation on one stream with concurrent request/response bodies. |
| `POST /v1/shutdown` | Stop admission and drain before process exit. |

### Request body and input demand

The invocation request begins with a four-byte big-endian JSON byte length,
followed by JSON containing `operation`, `invocationId`, parsed `args`, `cwd`, and
`env`. Each subsequent stdin chunk has a four-byte big-endian length and at most
64 KiB of binary payload.

The service sends response headers before waiting for input. Input then follows
this exchange:

1. The handler requests its next input chunk.
2. The SDK sends an input-pull record.
3. The caller sends exactly one chunk or signals EOF with request END_STREAM.

The caller preserves the chunk boundary across HTTP/2 DATA frames. A short
live-stdin read does not authorize another read. Receive-window capacity does
not authorize reading stdin either.

A handler may finish without reading all of its input. The service then ends
the request with `RST_STREAM(NO_ERROR)`, and a caller's EOF after that is not
an error.

### Response records and completion

Responses contain records with a one-byte kind, four-byte big-endian payload
length, and payload. DATA frame boundaries are unrelated to record boundaries.

| Kind | Payload |
| --- | --- |
| 1: stdout | Raw bytes. |
| 2: stderr | Raw bytes. |
| 3: completion | JSON: `exitCode` from 0 to 255, optionally `error: { code, message }`. |
| 4: input pull | Empty; requests one input chunk or EOF. |

A normally handled invocation ends with exactly one completion record. Missing
completion is failure even after HTTP 200. Bytes after completion are invalid.

HTTP status reports rejection before execution: invalid metadata, an unknown
operation, or a service that is shutting down. It never reflects how many calls
are in flight. After execution starts, command failure uses completion.
Command output carries any partial-operation result.

### Validation and flow control

The SDK rejects unknown metadata fields and malformed or oversized records.
Handlers reject invalid operation arguments before work. The SDK checks message
sizes while decoding, before allocating unbounded memory. These limits apply:

| Resource | Version 1 limit |
| --- | --- |
| Invocation metadata JSON | 256 KiB |
| Response record payload | 64 KiB |
| HTTP/2 header list | 16 KiB |
| Queued output records per invocation | 4 |
| HTTP/2 handshake, service info, and invocation metadata timeout | 10 seconds per phase |
| Cooperative cancellation grace | 5 seconds |

These are fixed SDK limits. The number of invocations is not limited
([Load](runner.md#load)). Each stream has its own flow-control window and
output queue, and the connection's window is the largest HTTP/2 allows, so the
connection never becomes the constraint: a slow consumer holds back only its
own invocation, and memory grows with the invocations running. The service's
only peer is the runner that started it, so HTTP/2's guard against a flood of
cancelled requests is off; cancelling many just-sent invocations at once is
ordinary.

The SDK returns input receive capacity while assembling one requested, bounded
chunk. It reserves output capacity before sending DATA. Connection processing
continues while output is blocked so that resets and disconnects can be received.

`RST_STREAM(CANCEL)` triggers the invocation's cancellation token. The
[service lifetime rules](#invoke-and-retire-a-service) determine whether cleanup
succeeds or the process is faulty.

Local forwarding reuses framing, demand, and completion rules but has a distinct
raw CLI metadata schema. Its authentication and disconnect behavior remain in
[the local client contract](commands.md#external-command-clients).

## Publish a complete release

A published version supplies the same operations on all supported targets.
Requiring a complete release lets jobs select their execution host without
encountering a platform-specific gap in that version.

Both command packages and runner releases cover the following targets. One
machine cross-compiles all six with the tools below, on Linux or macOS.
Cross-compilation and native execution are separate acceptance checks.

| Platform | Target triples | Release toolchain |
| --- | --- | --- |
| macOS | `darwin/arm64`, `darwin/amd64` | Go toolchain, `CGO_ENABLED=0` |
| Linux | `linux/arm64`, `linux/amd64` | Go toolchain, `CGO_ENABLED=0`, static |
| Windows | `windows/arm64`, `windows/amd64` | Go toolchain, `CGO_ENABLED=0` |

Release validation requires all of the following:

- Pin release tools and SDKs.
- Inspect Linux artifacts for unexpected shared-library dependencies.
- Run the same protocol and command conformance cases on every target.
- Supply native artifacts. Emulation or source-only support is insufficient.

Completeness is a rule of publication, not of the descriptor. A descriptor and
a runner manifest name the targets they carry. The packagers
write all six unless told otherwise, and the backend artifact module refuses to
publish a release that lacks one. A development release, which a developer
hands to a backend on their own machine, carries only the targets of the Hosts
in use: on an Apple silicon Mac with the Cloud in Lima that is
`aarch64-apple-darwin` and `aarch64-unknown-linux-musl`. A Host whose target a
release lacks fails the command with a catalog mismatch, and its runner cannot
be installed from that release; nothing falls back to another target.

The [build guide](../native-builds.md) defines commands and toolchain setup.
Managed guest images consume the Linux runner. Their image lifecycle and
execution-surface verification follow the
[managed host design](managed-hosts.md#images).

### Publish artifacts before enabling commands

The backend artifact module, `packages/backend/src/runner/artifacts/`, owns S3
publication. S3 is the only supported object-storage protocol. Before accepting
application requests, the module completes these steps:

1. Validate every release's descriptor, sizes, and hashes.
2. Upload missing content-addressed blobs. Bound upload concurrency and suppress
   duplicate uploads.
3. Publish the descriptor and immutable package/version mapping.
4. Enable the catalog.

Conditional writes reject conflicting content. Repeated startup reuses existing
objects. Interrupted publication can leave unreferenced blobs, but it cannot
expose a partial release or overwrite an existing version's meaning. Multipart
ETags must not be treated as SHA-256 checksums.

### Backend deployment configuration

`DEMI_NATIVE_CONFIG` names a JSON file read by the backend artifact module.
Each release directory contains `descriptor.json` and one executable under each
target triple. Windows filenames end in `.exe`. Relative directories resolve
against the configuration file's directory.

```json
{
  "prefix": "native",
  "releases": [
    { "directory": "./demi-commands", "executable": "demi-commands" }
  ],
  "store": {
    "provider": "s3",
    "bucket": "demi-native",
    "region": "us-east-1"
  }
}
```

S3 uses the AWS SDK credential provider chain and verifies SHA-256 upload
checksums. Its configuration also accepts `forcePathStyle` and an optional HTTPS
endpoint. An S3-compatible service must support the conditional writes, SHA-256
checksums, metadata, and presigned GET requests required by publication.

Backend assembly injects `nativeCommands: { packages, resolveArtifact }` into
`createBackend`. Cross-host commands receive the calling session's catalog.
Local test fixtures may supply local resolvers and host-only descriptors. Those
descriptors are not valid publication inputs.

## Contract generation and validation

Design rules above define the contract. Every contract has one source. A
contract that a TypeScript program also speaks keeps its Zod schema as that
source: the frontend validates with it, and build tooling such as the guest
image build writes with it. A contract the Rust runner or native programs are
built from keeps its Zod schema while they are Rust, since they generate their
bindings from it; the managed-boot file the runner reads is one of these. Go bindings for both
are generated from the Zod source and enforce the same constraints without a
second manually maintained definition. A contract spoken only between Go
programs, such as the machine manager wire, is defined directly in Go, in the
package that owns it, with validation at the point of entry.

| Schema owner | Go package |
| --- | --- |
| `packages/runner-protocol/src/schemas.ts` — runner messages | `internal/contract/runnerwire` |
| `packages/command-loader/src/manifest/schema.ts` — manifests | `internal/contract/manifest` |
| `packages/browser-protocol` — browser business schemas | `internal/contract/browser` |
| `packages/command-protocol/src/index.ts` — native wire and descriptors | `internal/contract/cmdservice` |
| `packages/agent/src/protocol` — conversation frames, blocks and the shared core types | `internal/contract/agentproto` |
| `packages/machines/src/image-manifest.ts` — Cloud image manifest written by the guest image build | `internal/contract/imagemanifest` |

`scripts/generate-go-contracts.ts` (`bun run go:contracts`) transforms the
schemas into Go types with boundary validation:

```text
Zod schemas -> scripts/go-zod.ts -> generated Go -> boundary validation
```

Generation runs before every Go build and test (`scripts/go-check.sh`), with Bun
and installed workspace dependencies as prerequisites. Generated files
(`zz_generated*.go`) are not committed; each contract package commits only its
package documentation.

Go validates fixed contracts directly, including literals, enums, ranges,
string and array constraints, record keys, required/optional/null values, and
operation uniqueness. Decoding and validation happen in one step, so no caller
holds an unvalidated value. Unsupported Zod constructs or refinements fail
generation.

A check that a schema expresses as code (a `.refine` callback or `z.custom`)
has no generic translation. Such a schema carries a stable name in its Zod
metadata (`.meta({ check: '<name>' })`), and the contract package implements
that check by hand in `checks.go` as a Go function of the same name; generation
fails for a code check without a name, and a named check without its function
does not compile. The recorded conformance corpus, judged by Zod, proves that
each hand-written check accepts exactly what the TypeScript check accepts. The
types in `@demicodes/core` are plain TypeScript interfaces, not schemas; their
Go types are the ones generated from the agent protocol, which validates them
at the wire.

Generated types alone do not establish validation. Fixed contract validation must
not round-trip values through JSON or a separately generated JSON Schema.

Runtime command argument schemas are separate from fixed contracts. Declarations
supply JSON Schema for dispatcher validation. Native handlers also validate their
own arguments before work.

The [build guide](../native-builds.md#validation) identifies release checks.
Acceptance of this design requires the same observable outcomes for successful
calls, blocked IO, malformed input, cancellation, and service failure on all six
targets, as well as verified publication and installation. Synthetic transport
benchmarks do not establish file-command performance.
