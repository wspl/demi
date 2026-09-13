# Native runner and command services

Status: design contract. The open decisions under "Contract generation and
validation" require discussion before implementation.

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
workers; it does not remove cancellation and cleanup requirements. The runner itself is implemented in Rust and embeds no JS engine.
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
process creation and keep steady-state calls inexpensive.
`packages/command-service/examples/benchmark.rs` exercises the transport;
file-command performance requires separate measurements.

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
Windows uses the MSVC targets with static CRT linkage. Target execution is validated separately from cross-compilation.

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
The build entry point is `scripts/native/build.ts`; `scripts/native/Dockerfile` pins the Linux toolchain.

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
  | authenticated MessagePack WebSocket and HTTP byte pipes
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

`runner/src/command_client.rs` implements forwarding as a mode of the same
runner executable. Command aliases select their root by basename. Unix domain
sockets serve Linux/macOS; byte-mode named pipes serve Windows. The local endpoint
is restricted to the current account and each invocation must provide its live
execution context. Normal stdin/stdout/stderr remain available for pipelines.

Each client opens one HTTP/2 connection and one invocation stream. The runner
validates the raw root/argv/context request, resolves the pinned declaration and
constructs a validated native operation or application callback. Local CLI metadata
and native invocation metadata remain distinct schemas; their bounded framing,
input demand and completion rules are shared.

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

The embedded shell resolves declared roots to these aliases. The executable
contains forwarding and dispatch code; native Demi algorithms live only in the
independently distributed `demi-commands` service.

## Initialization and responsibility boundaries

`AgentServerOptions.shellEnvironment` receives a
ShellEnvironmentFactory. Its context includes the invoking Host, CommandRegistry,
root/agent session identity and command storage. AgentHarness.commands declares
the command tree. The backend supplies the remote shell-environment factory from its assembly. These are the appropriate boundaries;
implementation packages do not belong in model/provider configuration or env vars.

Public composition API:

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

The backend supplies `runtimePackageCatalog` from its deployed releases at
runtime. Command declarations do not import a compiled-in release descriptor.

The Demi builtin package provides all Demi command implementations in one resident
executable per target platform. It is not limited to file commands. These Demi
commands are distinct from the shell and standard utility builtins embedded in
the runner. The runtime catalog may also contain SDK developer implementations.

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
and callback handlers. A native leaf declares a logical package id and operation;
the execution adapter resolves the exact release from its runtime catalog:

```ts
{
  name: 'patch',
  kind: 'native',
  binding: { package: 'demi.builtin', operation: 'file.patch' },
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
operation id, invocation id, parsed arguments, cwd and env. Each following stdin
chunk has a four-byte big-endian length followed by at most 64 KiB of binary
payload. An input-pull response record authorizes exactly one chunk or EOF. The
SDK sends this record when the handler requests its next input chunk. The caller
preserves that boundary across HTTP/2 DATA frames, so a short live-stdin read does
not authorize additional reads. Request END_STREAM means stdin EOF, not
cancellation. The server sends response headers before waiting for input.

The response body is a sequence of application records: a one-byte kind plus a
four-byte big-endian payload length, followed by bytes. Kinds distinguish stdout,
stderr, input pull and a final completion object. Output payloads are raw bytes; completion
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

Response kinds are 1 for stdout, 2 for stderr, 3 for completion, and 4 for input
pull. Input pull has an empty payload. Completion
contains `exitCode` (0 through 255) and an optional `error` with `code` and
`message`. Unknown metadata fields, malformed records, oversized declared payloads,
missing completion and bytes after completion fail validation. Handlers validate
their own operation argument schema before performing work.

Input returns HTTP/2 capacity while assembling one bounded, requested chunk.
No additional stdin read is authorized until the handler requests another chunk. Output
uses bounded queues and reserves HTTP/2 capacity before sending DATA. Connection
processing continues while output waits for capacity. A cancellation deadline
violation produces a fatal service error requiring process retirement; aborting
an async task does not prove that non-cooperative native work has stopped.

## Contract generation and validation

This section covers the fixed messages between the TypeScript backend and Rust
runner, the command manifest structure, and native package descriptor structure.
Runtime command argument definitions are a separate concern.

Each consuming crate's `build.rs` owns Rust contract generation as part of the
normal Cargo build. A developer does not run a separate generation command first.
The build script may invoke a JS/TS generator; its implementation language does
not change Cargo's ownership of this build step.

Generated Rust types and validation code are written to that crate's `OUT_DIR`
and included by the crate with `include!`. Generated files are build artifacts:
they are not written into the source tree or committed to the repository.

Rust validates received values directly with Rust code or native validation
tools. Generated structure types alone do not establish that value constraints
have been checked. Fixed contract validation does not use a separately generated
JSON Schema document or convert decoded values to JSON for schema validation.

```text
Shared contract definitions (source format remains open)
    |
    v
Consuming crate's build.rs
    |  may invoke a JS/TS generator
    v
OUT_DIR: Rust types and validation code
    |
    v
include! -> compiled crate -> input validation
```

### Open decisions

- The authoritative definition format for backend/runner messages, manifest
  structure and package descriptors. The build rules above do not select Zod,
  Rust declarations or another format as that source.
- The Rust validation mechanism and how generation represents constraints such
  as ranges, lengths, allowed values and relationships between fields. No
  validation library or custom validation framework is selected here.
- The representation and validation of command-specific arguments supplied at
  runtime by the server. These are not fixed protocol types that can all be
  compiled into the runner; their design requires a separate discussion.

Implementation of contract generation and validation depends on resolving these
open decisions.

## Configuration boundaries

Protocol limits are fixed SDK constants. Object-store adapters belong to
backend deployment assembly. The service SDK supplies cancellable platform stdio;
Windows runner releases use static CRT linkage.

## Backend deployment configuration

`backend/src/runner/artifacts/config.ts` reads the JSON file named by
`DEMI_NATIVE_CONFIG`. The backend publishes the configured releases before it
accepts application requests. Release directories contain `descriptor.json` and
one executable under each target triple. Windows executable names end in `.exe`.
Relative release directories resolve against the configuration file's directory.

```json
{
  "prefix": "native",
  "releases": [
    { "directory": "./demi-package", "executable": "demi-commands" }
  ],
  "store": {
    "provider": "s3",
    "bucket": "demi-native",
    "region": "us-east-1"
  }
}
```

The S3 adapter uses the AWS SDK credential provider chain. An OSS store specifies
`"provider": "oss"`, its bucket and region, and reads
`ALIBABA_CLOUD_ACCESS_KEY_ID`, `ALIBABA_CLOUD_ACCESS_KEY_SECRET` and optional
`ALIBABA_CLOUD_SECURITY_TOKEN`. Both adapters accept an optional HTTPS endpoint;
S3 also accepts `forcePathStyle`. Storage credentials stay in this module and
never enter a runner message.

`publish.ts` verifies the SHA-256 and byte count of all six target files before
uploading any object. It writes immutable blobs, the content-addressed descriptor,
and then the immutable package/version descriptor. S3 verifies a SHA-256 upload
checksum; OSS verifies an MD5 upload checksum while retaining the descriptor's
SHA-256 as object metadata. Conditional writes reject conflicting objects.
The resolver returns a signed HTTPS GET URL with a five-minute expiry and checks
that the requested digest and size belong to the published catalog.

`createBackend` receives `nativeCommands: { packages, resolveArtifact }` explicitly.
The command factory binds each job to its own manifest. Cross-host `demi host
shell` jobs receive the calling session's catalog through `host-command.ts`.
Test and local development fixtures supply an explicitly local resolver and a
host-only test descriptor; those descriptors are never publication inputs.

The runner's Rust `mode` module owns one backend connection, its jobs and command
contexts. `dispatch` validates the pinned declaration, handles help without
reading stdin, and routes the invocation to `rpc` or `native`. `rpc` owns the
running-hint guard; dropping an invocation clears the hint and cancels its
callback. `native` shares one verified service per artifact digest and retires
services after the current declaration and live jobs release their references.
The `demi-runner` executable also implements the external command-client mode:
job aliases name that executable, and their basename selects the declared root.

### Shell job lifetime

`runner/src/shell.rs` creates a fresh brush login shell for each job. It loads
login profiles, then restores the runner-owned context, command alias precedence
and requested cwd before executing the script. The job waits for
its asynchronous shell tasks before reporting completion, and preserves the
foreground script's exit status. For example, `(sleep 2; echo done) & echo started`
emits `started` immediately, remains a running job during the sleep, then emits
`done` and exits. A tool timeout returns that running job's handle; `shell_status`
and `shell_abort` continue to control it. Background tasks belong to this job and
do not become detached services. Cancellation and connection loss terminate the
job process and its descendants.

Brush's internal asynchronous tasks do not expose operating-system process IDs
through `$!`. Tests that kill an external command client identify its actual PID
from that child process; ordinary shell job cancellation uses the runner's job
handle. Unix process groups and Windows Job Objects provide process-tree cleanup.

Raw `Host.process.spawn` calls use only their supplied environment. A caller can
set `inheritEnv: true` to extend the device process environment; an explicitly
undefined value removes an inherited variable. The backend's
`llm/session-providers.ts` selects inheritance when launching provider CLIs, so
program lookup uses the device's `PATH` and credentials remain explicit overlays.
The backend never copies its own environment to the device.
