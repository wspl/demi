# Native command execution

This document defines native command execution: binding a command to a release,
installing its executable, and running it in a shared service. The execution
contract comes first; protocol, publication, and build requirements follow.

The runner is a Rust process with an embedded brush shell. Native command
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

The shared SDK owns no artifact or process management. Brush builtins and external
command clients use the same dispatcher, which supplies validated operation
metadata to the native service. Application callbacks return to the embedding
application instead. The native service never receives raw CLI requests.

Related contracts define the surrounding behavior:

- [Command declarations](commands.md): CLI parsing and manifest semantics.
- [Local forwarding](commands.md#external-command-clients): external clients and endpoint access.
- [Runner jobs](runner.md#shell-jobs): brush, profiles, and whole-job cleanup.
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

A release consists of its descriptor and the six executables in the
[release target matrix](#publish-a-complete-release). The descriptor contains:

| Descriptor field | Meaning |
| --- | --- |
| `id` | Stable namespaced identity, such as `demi.builtin`. |
| `version` | Human-readable release version, immutable within its publisher. |
| `protocolVersion` | Command-service wire major version. |
| `operations` | Unique operation IDs supplied by the package. |
| `targets` | All required target triples, each with executable SHA-256 and byte size. |

The descriptor hash is the SHA-256 of its canonical JSON. Paths, object keys,
and URLs locate artifacts but do not participate in package identity.

The runner independently validates descriptors and references. Release validation
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
| Execution context is disposed | Release its bindings and service references. Live contexts, invocations, or retained resources keep their service owned. |
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

## Retained resources

A native operation can create a live resource
that must survive the shell job that created it. For example, a conversation's
browser must keep its tabs between agent turns. An explicit resource owner keeps
the resident service alive; no fake long-running invocation is required.

The embedding application acquires an opaque grant through its normal Host
access. Acquisition pins the runner connection generation, execution security
context, package artifact, resource kind, and application owner scope. The runner
validates the catalog and resolves the artifact using the same installation path
as invocations. Concurrent requests for the same owner/kind/artifact share one
acquisition; different owners receive isolated resource identities. A grant is
never accepted from shell arguments or forwarded environment variables.

The protocol must carry these distinct operations and acknowledgements:

| Operation | Contract |
| --- | --- |
| Acquire | Accept authenticated owner scope, resource kind and pinned package identity; return a grant and generation after initialization succeeds |
| Bind job | Associate a live job with authorized grants; supply trusted resource context to each native invocation |
| Invoke/subscribe | Address a grant through the authenticated Host adapter; carry validated operation input, cancellation and bounded output/events |
| Release | Stop admission, cancel resource calls/subscriptions, release resource state and acknowledge completion; repeated release of the same retired grant is harmless |
| Lost | Report an ended connection, failed service or retired generation; invalidate every affected handle without replay |

The native service exposes `POST /v1/resource` using the invocation framing,
bounded output, and cancellation contract below. Its operation is `acquire`,
`release`, or `status`; its trusted `resource` field contains `{id, kind}`.
These operations are not declared commands and the local command client cannot
invoke this endpoint on the native service. Acquire initializes a dormant owner;
domain startup may remain lazy. Status returns `{state: "ready" | "released"}`.
Release and service shutdown await domain cleanup. An unknown released ID is
harmless to release but cannot be invoked or recreated under that same grant.

A native command binding may require a `resource` kind. Authenticated runner
messages acquire grants against an owner and exact package descriptor, bind
their IDs to a job, and release them. The runner validates every job binding and
injects the matching `{id, kind}` into native invocation metadata. Raw argv and
forwarded environment cannot supply this field. Invocations also carry their
requested JSON output mode, so native output uses the same declared result
schema as the dispatcher.

The native package owns the resource's domain state and cleanup. Runner and the
shared SDK own grant routing, service references, cancellation and transport.
Resource input and events use authoritative schemas; browser data does not
become generic SDK business logic. Product operations may use this authenticated
resource path without manufacturing a shell job. Both paths address the same
native owner and operation implementation.

Retained references are independent of invocation and shell-context references.
Disposing one shell context does not release another owner's resource. A release
acknowledgement means subscriptions ended, child processes were reaped, and
resource-owned temporary files were removed. Cancellation and shutdown use the
existing protocol grace limits; failed cleanup retires the faulty service and
reports affected resources as lost, rather than claiming successful release.

Runner/backend connection loss releases all grants from that connection.
Reconnect does not revive old grants. An artifact change creates a new binding;
a resource stays on its pinned artifact until released, and a mismatching job
fails explicitly rather than sharing incompatible state. The application must
release or finish the resource before moving it to a new artifact. Acquisition
failure leaves no reference or detached child behind.

A retained reference does not hold device activity or an application file gate.
Actual calls and active data subscriptions acquire admission through the embedding
application's Host access. Grant-scoped lifecycle/domain-state notifications can
be emitted on the existing connection without a permanent operation lease; they
cannot initiate Host work. Snapshot repair is an explicit short operation.
Release calls use the embedding application's admitted lifecycle cleanup scope.
The backend's [resource coordinator](resource-lifecycle.md) decides when to release;
runner and the native package execute that release and report its outcome.
This permits a stopped Cloud to invalidate an idle resource without promising
page restoration. Conversation policy belongs to
[Browser ownership](browser.md#ownership); transition admission belongs to
[Host operations](sessions-and-targets.md#host-operations).

## Invocation protocol

Protocol version 1 uses direct HTTP/2 requests. The shared SDK owns process IO.
Handlers use logical output writers. Executable stdout contains only protocol
bytes. Process stderr carries service diagnostics, drained independently by the
runner with bounded retention.

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

HTTP status reports rejection before execution, such as invalid metadata or
admission failure. After execution starts, command failure uses completion.
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
| Concurrent admitted service streams | 32 |
| Queued output records per invocation | 4 |
| HTTP/2 handshake, service info, and invocation metadata timeout | 10 seconds per phase |
| Cooperative cancellation grace | 5 seconds |

These are fixed SDK limits. Flow control and bounded queues, both per stream and
across streams, prevent a slow consumer from blocking unrelated calls or growing
memory without bound.

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

Both command packages and runner releases cover the following targets. Linux is
the build environment for all six. Cross-compilation and native execution are
separate acceptance checks.

| Platform | Target triples | Release toolchain |
| --- | --- | --- |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | cargo-zigbuild with Apple SDK |
| Linux | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | cargo-zigbuild, static musl |
| Windows | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc` | cargo-xwin, LLVM and Microsoft SDK, static CRT |

Release validation requires all of the following:

- Pin release tools and SDKs.
- Inspect Linux artifacts for unexpected shared-library dependencies. Selecting
  musl alone does not establish a self-contained executable.
- Run the same protocol and command conformance cases on every target.
- Supply native artifacts. Emulation or source-only support is insufficient.

The [build guide](../native-builds.md) defines commands and toolchain setup.
Managed guest images consume the Linux runner. Their image lifecycle and
execution-surface verification follow the
[managed host design](managed-hosts.md#images).

### Publish artifacts before enabling commands

The backend artifact module, `packages/backend/src/runner/artifacts/`, owns S3
and OSS publication through separate adapters. Before accepting application
requests, the module completes these steps:

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
checksums. Its configuration also accepts `forcePathStyle`.

OSS uses `"provider": "oss"`, bucket, and region. It reads these credentials:

- `ALIBABA_CLOUD_ACCESS_KEY_ID`
- `ALIBABA_CLOUD_ACCESS_KEY_SECRET`
- `ALIBABA_CLOUD_SECURITY_TOKEN` (optional)

OSS verifies MD5 upload checksums and retains the descriptor's SHA-256 in metadata.
Both adapters accept an optional HTTPS endpoint.

Backend assembly injects `nativeCommands: { packages, resolveArtifact }` into
`createBackend`. Cross-host commands receive the calling session's catalog.
Local test fixtures may supply local resolvers and host-only descriptors. Those
descriptors are not valid publication inputs.

## Contract generation and validation

Design rules above define the contract. Their executable schemas are maintained
once in the owning TypeScript packages. Generated Rust must enforce the same
constraints without a second manually maintained definition.

| Schema owner | Consumer |
| --- | --- |
| `packages/runner-protocol/src/schemas.ts` — runner messages | `crates/runner/build.rs` |
| `packages/command-loader/src/manifest/schema.ts` — manifests | `crates/runner/build.rs` |
| `packages/browser-protocol` — browser business schemas | `crates/demi-commands/build.rs` |
| `packages/command-protocol/src/index.ts` — native wire and descriptors | `crates/command-service/build.rs` |

Cargo transforms the schema into boundary validation during the build:

```text
Zod schemas -> Cargo build -> generated Rust -> boundary validation
```

Normal Cargo builds run generation, with Bun and installed workspace dependencies
as prerequisites. Build scripts may invoke JS/TS tooling. Generated files stay
in `OUT_DIR`, are included by the crate and are not committed. The shared
command-service library exposes its bindings to the runner and native programs.

Rust validates fixed contracts directly, including literals, enums, ranges,
string and array constraints, record keys, required/optional/null values, and
operation uniqueness. Unsupported Zod constructs or refinements fail generation.

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
