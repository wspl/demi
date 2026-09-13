# Native command execution

This document defines how a declared native command reaches an independently
released executable on its execution target: package binding, distribution,
invocation, and service lifetime. It is the design authority for that contract.
It does not record implementation progress.

The runner is a Rust process with an embedded brush shell. Native command
algorithms run in separate executables that stay running to serve multiple calls.
These *resident services* can be released independently of the runner. The runner
embeds no JavaScript engine.

All native Demi commands belong to the `demi.builtin` package. SDK applications
can supply additional packages using the same contract. Shell utilities and
application callbacks have their own owners.

## One command, from declaration to result

Consider `demi file patch` running on a laptop. Its declaration names
`demi.builtin` and `file.patch`. The application's startup catalog resolves that
logical binding to an immutable package descriptor. The job pins that descriptor,
so the command has an exact meaning throughout execution.

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

On the first call, the runner acquires and verifies the laptop's artifact, starts
its service and checks the handshake. Later calls in the same sharing scope reuse
that service. Concurrent calls use separate HTTP/2 streams.

The sequence below starts with an installed executable. Time runs downward;
arrows show interactions between the runner and service. Each invocation uses
its own stream on the reused connection.

```text
Native command execution: startup and successful calls

Runner                                  Command service
  |                                           |
  |--- Start process ------------------------>|
  |--- GET /v1/info -------------------------->|
  |<-- Protocol version + operations ---------|
  |                                           |
  |    Validate against pinned descriptor     |
  |                                           |
  |--- POST /v1/invoke ----------------------->|
  |                                           | Execute
  |<-- stdout / stderr records ---------------|
  |<-- completion record ---------------------|
  |                                           |
  |--- Next invocation ---------------------->|
  |                                           | Reuse process
  :                                           :
```

The patch handler works beside the file, so patching does not require transferring
the original file through the backend. The caller receives byte output and an
exit status.

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
- [Local forwarding](command-client.md): external clients and endpoint access.
- [Runner jobs](runner.md#shell-jobs): brush, profiles, and whole-job cleanup.
- [Package boundaries](../package-boundaries.md#source-organization): source module ownership.

## Bind an exact package

The embedding application supplies packages through its execution adapter, not
through model settings or environment variables:

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

`AgentHarness.commands` declares the tree. A native leaf includes:

```ts
{
  name: 'patch',
  kind: 'native',
  binding: { package: 'demi.builtin', operation: 'file.patch' },
  // Help and argument schemas complete this declaration; see commands.md.
}
```

Each package ID resolves to exactly one immutable descriptor within a factory
catalog. Duplicate IDs, incomplete packages, and unknown operations fail
initialization. Declarations do not import compiled-in release descriptors.

The factory context supplies the invoking Host, command registry, session identity,
and command storage. Each invocation retains its own authorized context even
when several environments share a catalog.

Core agent initialization has no object-store dependency. Callbacks remain
functions in the embedding application. Their source is not distributed.

A release consists of its descriptor and exactly six target executables:

| Descriptor field | Meaning |
| --- | --- |
| `id` | Stable namespaced identity, such as `demi.builtin`. |
| `version` | Human-readable release version, immutable within its publisher. |
| `protocolVersion` | Command-service wire major version. |
| `operations` | Unique operation IDs supplied by the package. |
| `targets` | All required target triples, each with executable SHA-256 and byte size. |

The descriptor's SHA-256 covers its canonical JSON fields. Paths, object keys, and
URLs are location metadata, excluded from identity. The manifest pins the
descriptor hash and operation. An artifact URL or a later catalog cannot change
that binding.

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

## Publish and install

A complete package and the runner release cover these native targets. Linux is
the build environment for all six. Cross-compilation alone is not evidence of
successful target execution.

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

```text
Local release bundles
         |
Backend validates all six executables
         |
         +--> immutable blobs --> S3 / OSS
         +--> descriptor (only after all blobs exist)
         `--> enabled startup catalog
                         |
Runner requests exact digest location
                         |
Backend returns authorized URL
                         |
Runner downloads directly from storage
         -> temporary file -> size/hash check -> atomic cache install
```

### Publish a complete release

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

### Resolve and install an artifact

For an uncached artifact, the backend validates its active package reference and
returns a location for that exact digest and size. The runner downloads directly
from storage over HTTPS. Credentials remain in the backend.

The resolver supports these deployment choices:

- Private storage: signed URLs without changes to bucket policy. The default
  backend resolver issues signed GET URLs valid for five minutes.
- Public storage: public HTTPS artifact URLs.
- SDK applications: installed files or application-supplied hosting.

The runner obtains URLs on demand. URLs never become package identity or permanent
manifest fields. An expired URL can be refreshed for the same digest. Other
download failures do not count as expiration.

The runner installs only its exact target executable:

1. Download to a temporary file, enforcing the declared size.
2. Verify the size and SHA-256.
3. Publish the verified file atomically into the cache and apply executable
   permissions where required.

Concurrent callers share one download per digest. Cancelling one caller preserves
a download still needed by another. If the download fails or is cancelled, the
runner releases the response and removes the temporary file. The runner never
executes partial or mismatched files.

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

## Invoke and retire a service

Calls share a resident service only when all three values match:

- Runner registration
- Execution user and security context
- Artifact digest

Concurrent startup requests within that scope share one launch. Different
immutable artifacts can run side by side. No service crosses a privilege boundary.
Reuse spreads process and connection startup costs across calls. It does not
establish a performance claim for individual commands.

The runner launches the executable in SDK service mode and owns all three process
pipes. HTTP/2 uses prior knowledge over stdin/stdout, with the runner as client
and service as server. There is no port listener, TLS, HTTP/1 upgrade, or gRPC.

Before admitting calls, the runner checks `GET /v1/info` against the pinned wire
version and operation set. The runner rejects startup on timeout, missing
operations, or version mismatch. The verified file establishes executable
identity, so the handshake does not require the service to report its own hash.

Each invocation owns its cwd, environment, identity, input, output, and cancellation.
Handlers must not change process-global cwd or environment. Concurrent operations,
including blocking or CPU-bound work, must allow connection processing and
cancellation to continue. File algorithms belong in the native package, not in
the runner or shared SDK.

| Event | Required result |
| --- | --- |
| Normal command completion | Deliver its output and exactly one completion record; release invocation resources. Keep the service available to its owners. |
| One invocation is cancelled | Propagate cancellation to its handler and release its resources. Preserve unrelated calls and the shared connection. EOF alone is not cancellation. |
| Handler exceeds cancellation grace | Report a service fault and retire the process. An aborted task or reset stream does not prove native work stopped. |
| Service crashes or corrupts the protocol | Fail affected invocations. Do not automatically replay potentially completed side effects. |
| Execution context is disposed | Release its bindings and service references. Live contexts or invocations keep their service owned. |
| Service has no remaining owners | Stop admission, request shutdown, drain streams, close stdin, and reap the child. A bounded shutdown deadline permits terminating the retiring process. |

For example, two jobs using the same artifact can share one service. Cancelling
one call normally leaves the other running. If its handler refuses to stop within
the grace period, retiring the faulty service also fails the other affected call.
The runner must report that failure rather than claim isolated cancellation.

Changing the startup catalog creates new pinned bindings, not an in-place
replacement of an executable serving an existing context.

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
| Handshake and invocation metadata timeout | 10 seconds |
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
[the local client contract](command-client.md#transport-and-lifetime).

## Contract generation and validation

Design rules above define the contract. Their executable schemas are maintained
once in the owning TypeScript packages. Generated Rust must enforce the same
constraints without a second manually maintained definition.

| Schema owner | Consumer |
| --- | --- |
| `packages/runner-protocol/src/schemas.ts` — runner messages | `crates/runner/build.rs` |
| `packages/command-loader/src/manifest/schema.ts` — manifests | `crates/runner/build.rs` |
| `packages/command-protocol/src/index.ts` — native wire and descriptors | `crates/command-service/build.rs` |

```text
Owning Zod schemas -> consuming crate's build.rs
                   -> OUT_DIR types and validation -> compiled Rust boundary
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
