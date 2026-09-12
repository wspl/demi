# Native runner and command services

Status: accepted architecture; implementation acceptance is tracked separately.

## Confirmed direction

The runner does not execute downloaded JavaScript. Agent-defined commands have
two execution destinations: a callback to the agent's embedding application, or
a native command implementation on the execution target. The callback need not
belong to the Demi backend product; programming SDK users supply their own.

All Demi builtin command implementations belong to one `demiPackage`, including
commands such as `demi file patch`. SDK developers can supply additional independent
implementation packages. These packages must be distributable without rebuilding the
runner. Compile-time registration into the runner does not meet that requirement.
SDK developers can build their own implementation executable. A package may
implement several commands rather than requiring an executable per command.

The backend distributes declarations and native artifacts. Artifact selection
must identify OS, architecture and relevant binary compatibility requirements;
the runner caches exact content-addressed versions. A command declaration binds
to an implementation artifact and an operation within it. Protocol validation,
installation and execution are runner responsibilities; file-editing algorithms
remain in the implementation package. The implementation package contract is specified below.

Rust shell and standard utility builtins remain a separate concern from these
downloadable agent-specific command implementations. The runner can still ship
as a single executable while downloading additional command executables.

Auditing remains out of scope. This direction removes the need for downloaded-JS
workers; it does not remove cancellation and cleanup requirements. Removing the
JS engine altogether also requires porting the current TypeScript runner itself.
The LLRT report records earlier feasibility evidence, not a runtime selection.

## Resident services and HTTP/2

A native implementation executable runs as a resident command service and handles
multiple concurrent invocations. The runner reuses the service and its HTTP/2
connection. Do not spawn a process for each command or introduce a worker-process
pool as the normal dispatch mechanism. Separate implementation packages may have
separate resident services; a package can serve multiple command operations.

HTTP/2 is the selected communication protocol. Map each invocation to a separate
HTTP/2 stream, with incremental request input and response output. Use parent-owned process stdin/stdout pipes as the HTTP/2 transport. The runner
is the HTTP/2 client and the resident executable is the HTTP/2 server. Use direct
HTTP/2 requests, not gRPC. The application wire contract is specified below.

The implementation SDK must provide the following execution guarantees:

- Dispatch concurrent invocations within the resident process. CPU-bound or
  blocking work must not block connection processing or cancellation delivery.
- Keep cwd, environment, input, output and cancellation per invocation. Do not
  mutate process-global cwd or environment to execute a command.
- Carry logical stdout, stderr and completion distinctly. Preserve binary payloads
  without base64 or JSON byte arrays. Keep service diagnostics separate.
- Use bounded queues and HTTP/2 flow control. A slow invocation must not block
  unrelated invocations or grow memory without a bound. Continue processing
  connection/control frames while individual consumers are stalled.
- Propagate stream cancellation to the native handler and release its resources.
  HTTP/2 stream reset alone is not proof that computation stopped. Do not kill the
  shared service to perform ordinary cancellation of one invocation.
- Report failures of a shared service to all affected invocations. Do not blindly
  retry commands with side effects after an ambiguous process failure.
- Define service shutdown and version replacement so active invocations are
  accounted for and process/connection resources are released.

The design objective is to amortize startup and connection costs, avoid repeated
process creation and keep steady-state calls inexpensive. Performance has not
been measured. Acceptance needs benchmarks for small-call latency, concurrent
throughput, large binary streams, slow consumers, cancellation and memory usage.

File operations run beside their files. Applying a patch does not require sending
the entire original file through the runner. The native service owns the patch
implementation; the runner owns distribution, connection and invocation routing.

## Required distribution targets

Every released native command implementation package must provide all six
artifacts below for the same package version and protocol contract. The runner
release must cover the same target matrix.

| Operating system | Architecture |
| --- | --- |
| macOS | arm64 |
| macOS | x64 |
| Linux | arm64 |
| Linux | x64 |
| Windows | arm64 |
| Windows | x64 |

These are six native executable artifacts, not source-only support or a promise
that a compiler might support the target. Each artifact must identify its target
and content hash. The backend selects the artifact for the target runner; a
missing target prevents publishing a complete release. Emulation is not a
substitute for supplying the native artifact.

Release validation must distinguish successful cross-compilation from execution
on the target. The same protocol and command conformance cases must be exercised
across all six targets. Target modern operating systems; do not introduce a
legacy OS support matrix. The selected Linux baseline is statically linked
musl: x86_64-unknown-linux-musl and aarch64-unknown-linux-musl. Inspect final
artifacts for unexpected shared-library dependencies; selecting a musl target
alone does not prove a self-contained executable. Native dependencies must
support that build. musl is a distribution choice, not a performance guarantee.
Windows runtime linkage and the target execution infrastructure still need
implementation choices. Cross-platform release support is required, not yet verified.

## Build and distribution architecture

The selected build environment is Linux for all six release artifacts:

| Target | Toolchain |
| --- | --- |
| Linux arm64/x64 | cargo-zigbuild, static musl |
| macOS arm64/x64 | cargo-zigbuild, Apple SDK |
| Windows arm64/x64 | cargo-xwin, LLVM, Microsoft SDK/CRT, MSVC targets |

Windows target triples are x86_64-pc-windows-msvc and
aarch64-pc-windows-msvc. Linux target triples are x86_64-unknown-linux-musl and
aarch64-unknown-linux-musl. macOS targets are x86_64-apple-darwin and
aarch64-apple-darwin. Build tools, SDKs and dependencies must be pinned for
reproducible releases. Target execution tests are separate from cross-compilation.
The toolchain direction is agreed; successful six-target project builds have not
been demonstrated.

```text
Implementation developer
        |
        v
Linux build environment
  +-- cargo-zigbuild + musl ------> Linux   arm64 / x64
  +-- cargo-zigbuild + Apple SDK -> macOS   arm64 / x64
  +-- cargo-xwin + Windows SDK --> Windows arm64 / x64
        |
        v
Versioned implementation package (6 binaries + metadata/hashes)
        |
        v
Backend distribution ----> Runner selects/caches target artifact
```

## Execution architecture

```text
Agent host (Demi product or programming SDK application)
  |-- command declarations + callback handlers
  |-- shell execution requests
  |-- implementation package references
  |
  | existing remote execution connection (protocol changes TBD)
  v
Native runner on execution target -- one runner executable
  |-- job lifecycle, input/output, cancellation
  |-- embedded brush shell
  |     |-- shell/core utilities -> builtins
  |     |-- git/python/node -----> system executables
  |     `-- declared command ---> command dispatcher
  |                                |-- callback -> agent host
  |                                `-- native binding
  |-- artifact cache and resident-service lifecycle
  |                                      |
  |                              reused HTTP/2 connection
  |                              concurrent invocation streams
  |                                      v
  |                    Independent resident command executable
  |                      |-- implementation SDK / dispatcher
  |                      |-- read / create / edit / patch
  |                      `-- local files and native capabilities
  |
  `-- other implementation packages use the same service contract
```

The execution diagram describes responsibility and process boundaries. HTTP/2
runs over stdio; service-instance sharing is scoped as specified below.
The runner package must not depend on a concrete file-command implementation.
Service libraries are compiled into independently distributed executables, not
linked into the runner. The agent-side declaration chooses a callback or an
artifact binding; neither sends JavaScript implementation text.

## Local command client

Verified current implementation: `packages/command-client` builds the `demi`
executable from C and libuv. Each process forwards one command to the runner relay,
including command arguments, cwd, environment and execution context. Its custom
local framing uses separate data and watch connections. The data reader pauses
while writing command output; the watch connection remains readable to detect
disconnection and cancel even when output is blocked. Input is read on explicit
runner demand. The client contains no builtin command implementations.

Implement the forwarding client in Rust alongside
the runner and share command transport contracts. Use HTTP/2 over a local duplex
transport: Unix domain sockets on Linux/macOS and byte-mode named pipes on Windows.
Keep command stdin/stdout/stderr available for normal pipelines and redirection;
the forwarding client's local connection is separate from those descriptors.
The resident implementation package continues to use HTTP/2 over its own stdio.

Each external client invocation opens one connection and one invocation stream.
The runner authenticates the execution context, resolves the declared command and
validates arguments before dispatching to an agent-host callback or a native
package operation. The local CLI endpoint carries root/argv/context; the native
service endpoint carries an already-resolved operation. Share framing, bounded
stream adapters and completion/cancellation handling without conflating these
request schemas or forwarding unvalidated CLI requests directly to a package.

HTTP/2 is useful here for protocol reuse, flow control and cancellation, not for
substantial multiplexing within one short-lived CLI process. It adds handshake
and library costs, so a speed or size improvement is not established. The
connection driver must continue processing resets and disconnects while output
consumers block; bounded output queues must not block that driver. Preserve
stdin-on-demand behavior: HTTP/2 receive-window capacity alone is not permission
to consume command stdin. Specify explicit input demand in the local endpoint
contract before implementation, including commands which never read stdin.

Local client loss cancels its invocation; cancellation propagates through the
runner to the selected destination. stdin EOF is independent of cancellation.
Retain execution-context validation and local endpoint access controls. Validate
blocked output, idle terminal input, disconnects and cancellation on all platforms.

The embedded shell can dispatch recognized declared commands directly inside the
runner through the same dispatcher. Preserve the external forwarding executable
for calls from scripts or external programs. Whether its code ships as a small
separate binary or a mode of the runner executable remains an implementation
decision; neither form contains `demiPackage` implementations.

## Initialization and responsibility boundaries

Verified current code: AgentServerOptions.shellEnvironment receives a
ShellEnvironmentFactory. Its context includes the invoking Host, CommandRegistry,
root/agent session identity and command storage. AgentHarness.commands declares
the command tree. The Next backend currently builds a manifest and supplies
RemoteShellEnvironment from its assembly. These are the appropriate boundaries;
implementation packages do not belong in model/provider configuration or env vars.

Target public composition API:

```ts
const shellEnvironment = createRemoteShellEnvironmentFactory({
  packages: [demiPackage, customPackage],
  resolveArtifact,
});

const server = new AgentServer({
  agent,
  providers,
  shellEnvironment,
  store,
});
```

`demiPackage` provides all Demi builtin command implementations in one resident
executable per target platform. It is not limited to file commands. These Demi
commands are distinct from the shell and standard utility builtins embedded in
the runner. `customPackage` illustrates an optional SDK developer implementation.

The execution adapter owns the package catalog and artifact resolution. The
factory validates declared native bindings against that catalog before making a
command manifest usable. A local programming embedder can resolve artifacts to
already-installed executables; a remote embedder supplies downloadable locations.
Core agent initialization stays independent of S3, OSS and cloud credentials.
`createRemoteShellEnvironmentFactory` belongs to the host-remote execution adapter
and uses structural context types rather than importing agent implementation code.
The same catalog may serve multiple node/Host environments; each invocation still
carries its own execution context and authorized command manifest.

`AgentHarness.commands` continues to own command names, help, argument schemas
and callback handlers. A native leaf refers to an immutable package and operation:

```ts
{
  name: 'patch',
  kind: 'native',
  binding: { package: demiPackage.id, operation: 'file.patch' },
  // Existing help, input/output schemas, positionals and stdinField go here.
}
```

Within one factory catalog, each package id resolves to exactly one immutable
package descriptor. Duplicate ids fail initialization; there is no latest-version
lookup during execution. The finalized command manifest records the descriptor
hash, so the wire binding is exact even when another catalog uses another version.
Callbacks remain agent-host functions; their source code is not distributed.
New native operations require no change to the generic runner.

## Implementation package and exact binding

An implementation release contains a descriptor and exactly six executables.
The descriptor has these fields:

- `id`: stable namespaced package identity, for example `demi.builtin` for
  `demiPackage`.
- `version`: human-readable release version; immutable within the publisher.
- `protocolVersion`: supported command-service wire major version.
- `operations`: unique operation ids implemented by this package.
- `targets`: exactly the six selected target triples, each with SHA-256 and size.

The descriptor hash is computed from a canonical encoding of those fields. Local
file paths, object-store keys and download URLs are transport metadata and do not
participate in package identity. Descriptor types and boundary validators come
from one explicit schema; Rust and TypeScript bindings use that contract.

The factory checks each declared operation exists and each package is complete.
The runner independently validates received descriptors and binding references.
The native handler validates input at its boundary as well. Command input schema
changes that affect native behavior require a matching package release; no silent
argument conversion or version fallback is permitted.

A release validator rejects missing targets, duplicate operation ids, unsupported
wire versions and a reused id/version with different content. The distribution
record pins the complete descriptor hash. Different immutable versions coexist
in the cache; an artifact URL never determines which version a command means.

## Next-only object-store publication and distribution

Only the Next backend implements package synchronization to S3/OSS. It is a
backend infrastructure module, proposed as `backend/src/command-packages/`, with
publication and artifact-location responsibilities. It does not contain command
algorithms. S3 and OSS use their own adapters behind a small object-store
interface; do not assume OSS is identical to every S3 API.

Next startup receives configured local release bundles and object-store
configuration. Before enabling a command catalog it:

1. Validates the descriptor and computes/checks hashes and sizes for all six files.
2. Synchronizes missing executables under immutable content-addressed object keys,
   e.g. `native-commands/blobs/<sha256>`. Uses bounded upload concurrency and
   collapses duplicate uploads in one process. Object metadata/checksums identify
   existing content; multipart ETags are not treated as SHA-256.
3. Publishes the descriptor only after every artifact is available. Its immutable
   key is `native-commands/packages/<descriptor-hash>.json`.
4. Makes the catalog available to the shell-environment factory and runner
   manifest path. A failed publication never exposes a partial release as ready.

Repeated startup reuses existing objects. Upload completion does not authorize a
mutable latest pointer to change an active invocation. Interrupted uploads may
leave unreferenced objects, but no incomplete descriptor becomes active. A local
source bundle with changed bytes is a different release and cannot overwrite the
meaning of an existing id/version.

For downloads, the backend resolver returns `{url, expiresAt?}` for an exact
artifact digest after validating the runner's active package reference. Use an
HTTPS object-store/CDN URL if that deployment publishes artifacts publicly; use
a presigned GET URL for a private bucket. Both are direct downloads from object
storage, not a backend byte proxy. Default private-store integration uses signed
URLs and leaves bucket access policy unchanged.

A URL is issued when an uncached artifact is needed. It is not baked permanently
into an immutable manifest or used as a cache key. If it expires, the runner asks
for a fresh URL for the same digest. Arbitrary download failures are not treated
as expiration; size/hash mismatches fail installation. Storage credentials stay
in the backend. The common execution SDK only knows an artifact resolver, so a
programming SDK user can supply local files, their own HTTPS hosting or another
resolver without using Next or object storage.

```text
Next configuration: release bundles + object-store settings
                |
                v
Next package publisher -- uploads --> S3 / OSS
                |
                +--> immutable package catalog
                              |
Agent shell-environment factory / manifest
                              |
                              v
Runner -- resolve artifact --> Next (metadata / signed URL only)
   |
   +--------- direct HTTPS GET ----------------> S3 / OSS
   |
   +--> validate hash + atomic cache installation
   +--> launch resident executable
   +--> HTTP/2 over stdio
```

## Runner installation, handshake and sharing

The runner selects the exact target tuple and fetches only that executable. One
in-flight download per digest is shared by concurrent requests. Downloads go to a
temporary file, enforce the declared byte size, verify SHA-256 and atomically
publish into the cache after success. Failure/cancellation releases the response
and removes the temporary file. One caller cancelling must not cancel a download
still needed by another caller. Native executable permissions are applied where
required. Do not execute partial or mismatched artifacts.

A resident instance is scoped to one runner registration, execution user/security
context and artifact digest. Sessions with that same scope share it; no service
is shared across a privilege boundary. All requests carry their own cwd, env and
invocation identity rather than changing process-global values. Separate immutable
artifacts may run as separate resident services; calls do not create processes.

Launch the executable with a fixed SDK service-mode argument. The runner owns
stdin, stdout and stderr pipe endpoints. Establish HTTP/2 using prior knowledge;
there is no port listener, HTTP/1 upgrade or TLS inside these parent-owned pipes.
Before accepting invocations, `GET /v1/info` returns the wire version and operation
ids. The runner validates them against the pinned descriptor. Handshake timeout,
missing operations or version disagreement fail service startup explicitly.
Verified executable bytes already establish the artifact identity; the service
need not embed its own executable hash, which would be circular.

Spawn is single-flight within the instance scope. The service stays resident for
reuse until runner shutdown, explicit retirement or failure. Each active command
is bound to its manifest/package snapshot. When a catalog is replaced, new calls
use the new snapshot, old calls keep their binding, and an old service retires
after no live manifest references or calls need it. Close through an application
shutdown request, drain streams, close stdin to deliver EOF, and reap the child;
a bounded shutdown deadline permits terminating the whole retiring service.

## Direct HTTP/2 command protocol

One HTTP/2 stream represents one command invocation:

- `GET /v1/info`: startup negotiation described above.
- `POST /v1/invoke`: one invocation; request/response bodies can stream concurrently.
- `POST /v1/shutdown`: stop admitting invocations and drain before process exit.

The invocation body starts with one bounded length-prefixed JSON metadata record:
operation id, invocation id, parsed arguments, cwd and env. The remainder is raw
stdin bytes. Request END_STREAM means stdin EOF, not cancellation. The server
sends response headers early rather than buffering stdin to completion.

The response body is a sequence of application records: a one-byte kind plus a
four-byte big-endian payload length, followed by bytes. Kinds distinguish stdout,
stderr and a final completion object. Output payloads are raw bytes; completion
is bounded JSON containing exit status and any structured command error. Exactly
one completion record ends a normally handled invocation. Missing completion
means an interrupted/failed invocation, even if HTTP response headers were 200.
HTTP/2 DATA frame boundaries are not application record boundaries.

Use HTTP status for rejection before execution (invalid metadata, unknown binding,
unsupported protocol, admission failure). Command failure after execution starts
is represented by the completion record, including any partial-operation result.
Define record/metadata size limits in the shared schema and SDK constants before
implementation; enforce them while decoding, not after buffering arbitrary input.

The runner and service drive their HTTP/2 connections continuously. Window updates
follow consumption into bounded queues, with per-stream and aggregate limits.
The runtime separates blocking command work from connection processing.
RST_STREAM(CANCEL) cancels that invocation's SDK token; native handlers must stop
and release resources cooperatively. Cancelling one invocation does not close the
shared connection. Service failure fails all affected calls and does not replay
side effects automatically. A cancellation deadline violation is a handler/service
fault; it must not be falsely reported as successful cancellation.

Executable stdout contains only HTTP/2 bytes. Service diagnostics go to process
stderr, which the runner drains independently with bounded retention. Invocation
stdout/stderr are the logical output records, not writes to process stdio.
The SDK owns the protocol IO. Command implementations must use SDK output
writers and must not print to process stdout; arbitrary native code can still
violate that contract, so malformed protocol output fails the connection.

## Transport feasibility and validation gates

Rust h2 accepts an AsyncRead + AsyncWrite transport. The runner can combine child
stdout for reading and child stdin for writing; the service uses the corresponding
stdin/stdout ends. This makes HTTP/2 over stdio feasible without a socket server.
It is a protocol/library feasibility result, not yet a six-platform executable test.

Tokio's generic stdin uses a blocking read that cannot be cancelled. The service
SDK must deliberately handle pipe EOF and reader-thread/resource shutdown rather
than assume dropping a future closes an OS read. Use platform-appropriate pipe
adapters behind the SDK; verify behavior on both Windows architectures and Unix.
No per-invocation thread/process allocation should be required by the transport.

Before implementation acceptance, test concurrent calls, interleaved binary
output, slow readers, stdin EOF, cancellation during CPU and IO work, parent death,
service crashes, handshake mismatch, old/new release coexistence, URL expiration,
download interruption, hash mismatch and all six native release targets. Benchmark
warm invocation latency, throughput and bounded memory. No real model calls are
needed for these checks.

## Shared Rust service contract

`packages/command-protocol` owns strict serde wire types and incremental response
framing. `packages/command-service` owns HTTP/2 service dispatch over injected
asynchronous duplex IO. Neither crate contains Demi builtin command algorithms.

Protocol version 1 uses these limits:

| Resource | Limit |
| --- | --- |
| Invocation metadata JSON | 256 KiB |
| Response record payload | 64 KiB |
| HTTP/2 header list | 16 KiB |
| Admitted concurrent service streams | 32 |
| Queued output records per invocation | 4 |
| Handshake and invocation metadata timeout | 10 seconds |
| Cooperative cancellation grace | 5 seconds |

Response kinds are 1 for stdout, 2 for stderr, and 3 for completion. Completion
contains `exitCode` (0 through 255) and an optional `error` with `code` and
`message`. Unknown metadata fields, malformed records, oversized declared payloads,
missing completion and bytes after completion fail validation. Handlers validate
their own operation argument schema before performing work.

Input returns HTTP/2 capacity as the handler advances through its stream. Output
uses bounded queues and reserves HTTP/2 capacity before sending DATA. Connection
processing continues while output waits for capacity. A cancellation deadline
violation produces a fatal service error requiring process retirement; aborting
an async task does not prove that non-cooperative native work has stopped.

## Remaining implementation choices

- TypeScript bindings generated from the native package descriptor contract.
- Public configuration of service limits.
- Concrete S3/OSS SDK adapters and configuration fields.
- Platform-specific cancellable stdio implementation and Windows runtime linkage.

The implementation must satisfy the validation gates before the Rust runtime
becomes the production runner. Partial compilation does not establish acceptance.

## Sources

- [h2 transport bounds](https://docs.rs/h2/latest/h2/client/fn.handshake.html)
- [Tokio stdin lifecycle](https://docs.rs/tokio/latest/tokio/io/fn.stdin.html)
- [S3 direct signed downloads](https://docs.aws.amazon.com/AmazonS3/latest/userguide/using-presigned-url.html)

- [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild)
- [cargo-xwin](https://github.com/rust-cross/cargo-xwin)

- [HTTP/2 streams and flow control](https://www.rfc-editor.org/rfc/rfc9113.html)
- Existing implementation: `packages/shell/src/command.ts`,
  `packages/command-loader/src/manifest/`, and `packages/runner/src/relay/`.
