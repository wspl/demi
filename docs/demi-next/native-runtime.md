# Native command execution

This document defines how a declared native command reaches an independently
released executable on its execution target: package binding, distribution,
invocation and service lifetime. It is the design authority for that contract;
it does not record implementation progress.

The runner is a Rust process with an embedded brush shell. Native command
algorithms run in separate resident executables, without rebuilding the runner
or embedding a JavaScript engine. All native Demi commands belong to the
`demi.builtin` package; SDK applications can supply additional packages using the
same contract. Shell utilities and application callbacks have their own owners.

## One command, from declaration to result

Consider `demi file patch` running on a laptop. Its declaration names
`demi.builtin` and `file.patch`. The application's startup catalog resolves that
logical binding to an immutable package descriptor. The job pins that descriptor,
so the command has an exact meaning throughout execution.

```text
Embedding application (Demi backend or SDK application)
  command tree + startup package catalog
                  |
           pinned job manifest
                  v
Runner on laptop
  brush declared builtin ----+
  external command client ---+--> validated command dispatcher
                                     |                  |
                                  native               rpc
                                     |                  `--> application callback
                           acquire pinned executable
                                     |
                          reused HTTP/2 connection
                          over child stdin/stdout
                                     v
                         Resident demi-commands service
                           file.patch -> laptop files
                                     |
                         stdout / stderr / completion
                                     v
                              calling shell job
```

On the first call, the runner acquires and verifies the laptop's artifact, starts
its service and checks the handshake. Later calls in the same sharing scope reuse
that service; concurrent calls use separate HTTP/2 streams. The patch handler
works beside the file, so patching does not require transferring the original
file through the backend. The caller receives byte output and an exit status.

The responsibility boundaries are:

| Owner | Responsibility |
| --- | --- |
| Agent command tree | Names, help, argument schemas, logical bindings and callback handlers. |
| Execution adapter (`host-remote`) | Validate bindings against the startup catalog and resolve artifact locations. |
| Backend artifact module | Publish complete releases and authorize artifact downloads. No command algorithms. |
| Runner commands module | Validate dispatch, install artifacts, own service processes and route invocations. |
| Shared command-service SDK | Validate and frame messages; drive HTTP/2, byte IO and cancellation over supplied transport. No artifact or process management. |
| Native package | Implement operations, validate their arguments and release operation resources. |

[Command declarations](commands.md) own CLI parsing and manifest semantics;
[local forwarding](command-client.md) owns external clients and endpoint access;
[runner jobs](runner.md#shell-jobs) own brush, profiles and whole-job cleanup.
Both command entry paths use the same dispatcher. The native service receives
validated operation metadata, never raw CLI requests. Source module ownership is
specified in [package boundaries](../package-boundaries.md#source-organization).

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

Each package id resolves to exactly one immutable descriptor within a factory
catalog. Duplicate ids, incomplete packages and unknown operations fail
initialization. Declarations do not import compiled-in release descriptors.
The factory context supplies the invoking Host, command registry, session identity
and command storage; each invocation retains its own authorized context even
when several environments share a catalog. Core agent initialization has no
object-store dependency. Callbacks remain functions in the embedding application;
their source is not distributed.

A release consists of its descriptor and exactly six target executables:

| Descriptor field | Meaning |
| --- | --- |
| `id` | Stable namespaced identity, such as `demi.builtin`. |
| `version` | Human-readable release version, immutable within its publisher. |
| `protocolVersion` | Command-service wire major version. |
| `operations` | Unique operation ids supplied by the package. |
| `targets` | All required target triples, each with executable SHA-256 and byte size. |

The descriptor's SHA-256 covers its canonical JSON fields. Paths, object keys and
URLs are location metadata, excluded from identity. The manifest pins the
descriptor hash and operation; an artifact URL or a later catalog cannot change
that binding. The runner independently validates descriptors and references.
Reusing an id/version with different content, unsupported wire versions and
missing targets are rejected. An input-schema change affecting native behavior
requires a matching release; there is no argument repair or version fallback.

Command definitions and catalogs remain fixed for the embedding program's
lifetime. New definitions take effect when that program restarts; there is no
catalog hot update. Existing contexts retain their pinned binding. Connection
loss and context disposal follow the [runner command lifetime](runner.md#command-lifetime).

## Publish and install

A complete package and the runner release cover these native targets. Linux is
the build environment for all six; cross-compilation alone is not evidence of
successful target execution.

| Platform | Target triples | Release toolchain |
| --- | --- | --- |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | cargo-zigbuild with Apple SDK |
| Linux | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | cargo-zigbuild, static musl |
| Windows | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc` | cargo-xwin, LLVM and Microsoft SDK, static CRT |

Release tools and SDKs are pinned. Final Linux artifacts must have no unexpected
shared-library dependencies; selecting musl alone does not establish this.
Every target runs the same protocol and command conformance cases. Emulation or
source-only support cannot substitute for a native release artifact. The
[build guide](../native-builds.md) owns commands and toolchain setup. Managed guest
images consume the Linux runner; their image lifecycle and execution-surface
verification belong to [managed hosts](managed-hosts.md#images).

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

The backend artifact module, `packages/backend/src/runner/artifacts/`, owns S3
and OSS publication through separate adapters. Before accepting application
requests, it validates every release's descriptor, sizes and hashes; uploads
missing content-addressed blobs with bounded concurrency and duplicate-upload
suppression; publishes the descriptor and immutable package/version mapping;
then exposes the catalog. Conditional writes reject conflicting content.
Multipart ETags are not SHA-256. Repeated startup reuses existing objects.
Interrupted publication can leave unreferenced blobs but cannot expose a partial
release or overwrite an existing version's meaning.

For an uncached artifact, the backend validates its active package reference and
returns a location for that exact digest and size. Runners download directly
from storage over HTTPS. Private storage uses signed URLs without changing bucket
policy; deployments with public artifacts can supply public HTTPS locations.
The default backend resolver issues signed GET URLs valid for five minutes.
Credentials remain in the backend. An SDK embedder can instead provide installed
files or its own hosting through the resolver.

URLs are obtained on demand and never become package identity or permanent
manifest fields. Expiration permits refreshing the URL for the same digest;
arbitrary download failures do not count as expiration. The runner selects only
its exact target executable. Concurrent callers share one download per digest;
cancelling one caller preserves a download still needed by another. Failed or
cancelled downloads release the response and remove temporary files. Size and
SHA-256 must match before atomic installation and executable permissions are
applied. Partial or mismatched files are never executed.

### Backend deployment configuration

`DEMI_NATIVE_CONFIG` names a JSON file read by the backend artifact module.
Each release directory contains `descriptor.json` and one executable under each
target triple; Windows filenames end in `.exe`. Relative directories resolve
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

S3 uses the AWS SDK credential provider chain. OSS uses `"provider": "oss"`,
bucket and region, with `ALIBABA_CLOUD_ACCESS_KEY_ID`,
`ALIBABA_CLOUD_ACCESS_KEY_SECRET` and optional `ALIBABA_CLOUD_SECURITY_TOKEN`.
Both accept an optional HTTPS endpoint; S3 also accepts `forcePathStyle`.
S3 verifies SHA-256 upload checksums; OSS verifies MD5 upload checksums and retains
the descriptor's SHA-256 in metadata.

Backend assembly injects `nativeCommands: { packages, resolveArtifact }` into
`createBackend`. Cross-host commands receive the calling session's catalog.
Local test fixtures may supply local resolvers and host-only descriptors; those
are not valid publication inputs.

## Invoke and retire a service

One resident service is shared only within the same runner registration,
execution user/security context and artifact digest. Startup is single-flight
within that scope. Different immutable artifacts can run side by side; no
service crosses a privilege boundary. Reuse amortizes process and connection
startup. It does not establish a performance claim for individual commands.

The runner launches the executable in SDK service mode and owns all three process
pipes. HTTP/2 uses prior knowledge over stdin/stdout, with the runner as client
and service as server. There is no port listener, TLS, HTTP/1 upgrade or gRPC.
Before admitting calls, the runner checks `GET /v1/info` against the pinned wire
version and operation set. Timeout, missing operations or version mismatch fail
startup. The verified file establishes executable identity; no self-hash is
required in the handshake.

Each invocation owns its cwd, environment, identity, input, output and cancellation.
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
| Service has no remaining owners | Stop admission, request shutdown, drain streams, close stdin and reap the child. A bounded shutdown deadline permits terminating the retiring process. |

For example, two jobs using the same artifact can share one service. Cancelling
one call normally leaves the other running. If its handler refuses to stop within
the grace period, retiring the faulty service also fails the other affected call;
the runner must report that failure rather than claim isolated cancellation.
Changing the startup catalog creates new pinned bindings, not an in-place
replacement of an executable serving an existing context.

## Invocation protocol

Protocol version 1 uses direct HTTP/2 requests. The shared SDK owns process IO;
handlers use logical output writers. Executable stdout contains only protocol
bytes. Process stderr carries service diagnostics, drained independently by the
runner with bounded retention.

| Request | Purpose |
| --- | --- |
| `GET /v1/info` | Return wire version and operation ids before invocations. |
| `POST /v1/invoke` | Run one invocation on one stream with concurrent request/response bodies. |
| `POST /v1/shutdown` | Stop admission and drain before process exit. |

The invocation request begins with a four-byte big-endian JSON byte length,
then bounded JSON containing `operation`, `invocationId`, parsed `args`, `cwd`
and `env`. Each subsequent stdin
chunk has a four-byte big-endian length and at most 64 KiB of binary payload.

The service sends response headers before waiting for input. When the handler
asks for input, an input-pull record authorizes exactly one chunk or EOF. The
caller preserves that chunk across HTTP/2 DATA frames; a short live-stdin read
does not authorize another read. Receive-window capacity is not input demand.
Request END_STREAM means stdin EOF.

Responses contain records with a one-byte kind, four-byte big-endian payload
length and payload. DATA frame boundaries are unrelated to record boundaries.

| Kind | Payload |
| --- | --- |
| 1: stdout | Raw bytes. |
| 2: stderr | Raw bytes. |
| 3: completion | JSON: `exitCode` from 0 to 255, optionally `error: { code, message }`. |
| 4: input pull | Empty; requests one input chunk or EOF. |

A normally handled invocation ends with exactly one completion record. Missing
completion is failure even after HTTP 200; bytes after completion are invalid.
Reject unknown metadata fields, malformed or oversized records and invalid
operation arguments. Enforce limits while decoding, before unbounded allocation.
HTTP status reports rejection before execution, such as invalid metadata or
admission failure. After execution starts, command failure uses completion,
with any partial-operation result carried by command output.

| Resource | Version 1 limit |
| --- | --- |
| Invocation metadata JSON | 256 KiB |
| Response record payload | 64 KiB |
| HTTP/2 header list | 16 KiB |
| Concurrent admitted service streams | 32 |
| Queued output records per invocation | 4 |
| Handshake and invocation metadata timeout | 10 seconds |
| Cooperative cancellation grace | 5 seconds |

These are fixed SDK limits. Flow control and bounded per-stream/aggregate queues
prevent a slow consumer from blocking unrelated calls or growing memory without
bound. Input returns receive capacity while assembling one requested, bounded
chunk. Output reserves capacity before sending DATA. Connection processing must
continue through blocked output to receive resets and disconnects.
`RST_STREAM(CANCEL)` triggers the invocation's cancellation token; the service
lifetime rules above determine whether cleanup succeeds or the process is faulty.

Local forwarding reuses framing, demand and completion rules but has a distinct
raw CLI metadata schema. Its authentication and disconnect behavior remain in
[the local client contract](command-client.md#transport-and-lifetime).

## Contract generation and validation

Design rules above define the contract. Their executable schemas are maintained
once in the owning TypeScript packages; generated Rust must enforce the same
constraints rather than introduce a second manually maintained definition.

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

Rust validates fixed contracts directly: literals, enums, ranges, string/array
constraints, record keys, required/optional/null values and declared operation
uniqueness. Unsupported Zod constructs or refinements fail generation. Structure
types alone are insufficient; fixed contract validation does not round-trip values
through JSON or a separately generated JSON Schema. Runtime command argument
schemas are a distinct contract: declarations supply JSON Schema for dispatcher
validation, and native handlers validate their own arguments before work.

The [build guide](../native-builds.md#validation) identifies release checks.
Acceptance of this design requires the same observable outcomes for successful
calls, blocked IO, malformed input, cancellation and service failure on all six
targets, as well as verified publication and installation. Synthetic transport
benchmarks do not establish file-command performance.
