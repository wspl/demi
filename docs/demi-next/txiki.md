# Demi Next: txiki.js

The runner uses a fork of [txiki.js](https://github.com/wspl/txiki.js), pinned
as the recursive submodule `vendor/txiki.js`. The fork starts from upstream
v26.6.0. QuickJS-ng executes JavaScript; libuv supplies filesystem, process
and socket I/O; libwebsockets supplies HTTP and WebSocket connections.
There is no separate Demi runtime package or Rust packer.

## Responsibilities

`vendor/txiki.js` owns native APIs, the event loop, Web APIs, and linking an
embedded entry module. `packages/runner/runtime` builds the runtime and the
single-file application. `packages/runner/src/machine` adapts those APIs to
Demi's Host, process, job and transport contracts. The backend and runner
use the same MessagePack implementation from `runner-protocol/msgpack`.

Runtime command modules receive the Host and command context from
`command-loader`. This keeps command implementations portable; it is not
a sandbox. Code on the target has the permissions of the target user,
including access to the runtime's globals.

```text
backend (Bun)                  target machine
runner-protocol ─ WebSocket ── runner (TypeScript bundle)
                               ├─ machine/ → txiki.js APIs → files/processes
                               ├─ JobTable → bash → commands
                               └─ local IPC ← native C client
                                   └─ command-loader → isolated worker / backend RPC
```

## Runtime surface

The runner uses `tjs` for files, child processes, identity and signals;
`ReadableStream` and `WritableStream` for byte streams; `fetch` for HTTP;
`WebSocketStream` for the backend; and pipe sockets for the local Unix
relay. The fork's declarations in `types/src` are the authoritative API
contract, referenced by `machine/txiki.d.ts`.

The fork adds the capabilities needed by both devices and managed guests:

- `tjs.spawn(args, { detached: true })` starts a separate session/process
  group on Unix. The child remains waitable. `tjs.kill(-pid, signal)`
  signals that group.
- `tjs.fstat(fd)` inspects an inherited descriptor without taking ownership.
  The native client uses libuv fstat to compare fd 0 with the
  descriptor duplicated by the job prelude. Matching numbers identify the
  job's live stdin; redirection supplies a finite command input instead.
- `tjs.dropPrivileges(uid, gid)` clears supplementary groups, then sets gid
  and uid. PID 1 calls it after mounting the guest filesystem. Failure is
  fatal to startup: an earlier credential operation may already have
  succeeded. It is not a reversible identity-switching API.
- Linux PID 1 reaps adopted children after libuv processes its own child
  notifications. The reaper skips registered libuv process handles, so
  their `wait()` results remain available.
- Native stream writers distinguish a completed synchronous write from a
  queued write, and resolve queued writes on the native completion callback.
  This applies to child stdin and process stdout/stderr. File-backed writes
  also complete partial writes before resolving.

The standard library includes Fetch, URL, Web Streams, WebSocket, encoding
and Web Crypto implementations. Optional FFI, WebAssembly, SQLite and
mimalloc are disabled in Demi builds. Disabling these features does not
remove the standard stream or network APIs used by the runner.

## Jobs and streams

`machine/jobs.ts` drains child stdout and stderr into full log files. Each
stream retains at most `viewLimit` bytes for the wire preview. A requested
full stdout copy uses a standard stream with backpressure: a slow upload
slows the job's output; cancelling that copy lets file logging continue.
The job's completion joins process exit and both log drains. Process-group
termination can therefore be escalated while descendants still hold the
output pipes, even after the group leader has exited.

File writes account for partial writes. Tail reads allocate only the
requested tail length. HTTP uploads pull from the producer through a
ReadableStream with `duplex: 'half'`; responses are consumed as streams.
Fetch and WebSocketStream pause native socket reads when the consumer
queue fills and resume them on demand. An early final response or network
failure cancels the upload producer, including a pending read. The runner
does not buffer a whole job output or HTTP request body before sending it.

## Entry modes and single-file packaging

The fork's CMake build accepts `TJS_APP_ENTRY`, a bundled ES module, and
`TJS_BYTECODE_COMPILER`, the path to a host `tjsc` built from the same pinned
source. `tjsc` emits a C bytecode array, and CMake links that array with the
runtime and native CLI entry into `tjs-app`.

The embedded runtime evaluates this array directly. It does not scan its
executable, parse runtime CLI options, create `~/.tjs`, or need `/proc` to
locate the entry. Normal ELF and Mach-O linking supplies the executable
layout; the macOS build signs the final file after stripping. The build copies the result to `demi-runner`.
The compiler and source bundle are build inputs, not files installed on
the target.

txiki.js packages only `demi-runner`; `demi` is an independent C + libuv
executable. `src/entry.ts` starts the runner or its installation management
commands (`status`, `drain`). PID 1 first performs managed boot and drops to
the guest account. Root command invocation never enters this executable.
`runtime/bundle.ts` embeds the command-worker JS source in the runner bundle;
each runtime leaf receives an isolated worker, with message-based streams
and an interruptible QuickJS loop for cancellation.

`FileHandle.lock()` provides the nonblocking exclusive installation lease,
released by close or process exit. Worker termination sets an atomic flag
checked by QuickJS before joining its thread.

The bare development interpreter uses `tjs run entry.mjs`. It is useful
for isolated conformance tests; it is not the installed command launcher.

## Building

Initialize pinned dependencies before building:

```sh
git submodule update --init --recursive
bun install
```

Native builds need CMake and a C/C++ compiler. Linux static cross builds
also need Zig. `CMAKE` and `ZIG` can name specific executables. The build
cache is `.cache/txiki`; generated binaries are never committed.

```sh
bun packages/runner/runtime/bundle.ts packages/runner/src/entry.ts /tmp/entry.mjs
bun packages/runner/runtime/build.ts /tmp/entry.mjs /tmp/demi-runner
bash packages/guest-image/runner/build.sh aarch64
bash packages/guest-image/runner/build.sh x86_64
# On macOS, also link an Intel application:
bun packages/runner/runtime/build.ts /tmp/entry.mjs /tmp/demi-runner-intel x86_64-macos
```

The guest image pipeline consumes this same static executable as
`/demi-runner`. It remains PID 1 throughout the guest lifetime. Kernel and
rootfs creation are described in `packages/guest-image/README.md`.

When changing fork JavaScript, regenerate the checked-in bytecode using
the fork's `make js` workflow before rebuilding. Commit and push the fork
first, then update and commit the Demi submodule pointer. Both the native
compiler and target runtime must use that pinned QuickJS revision.

## Application sizes

Measured paired release artifacts on 2026-09-08 (MinSizeRel, stripped,
optional features disabled). The native client includes statically linked
libuv; the runner includes JS dispatch and the worker source.

| Target | Native client bytes | Runner bytes | Verification |
|---|---:|---:|---|
| macOS ARM64 | 125,456 | 3,265,232 | integration and installer tests |
| macOS Intel | 93,232 | 3,364,656 | cross build; startup under Rosetta |
| Linux ARM64 musl | 129,056 | 3,510,272 | real KVM, direct and jailer |
| Linux x64 musl | 122,664 | 3,595,136 | static cross build |

Sizes vary with compiler and bundle. The Windows C client cross-build is
available separately; a Windows runner release is not yet published.

## Network configuration

HTTP and WebSocket connections use the fork's proxy environment handling.
TLS uses the embedded CA bundle by default. An embedded application can
set `TJS_CA_BUNDLE` or `SSL_CERT_FILE` to a PEM bundle before starting;
`TJS_CA_BUNDLE` takes precedence. The embedded entry has no persistent
cookie jar. These defaults belong to the runtime, not to backend routing.

## Verification

The runner suite executes native command dispatch, Host conformance, binary wire
frames, process cancellation, live control, runtime/RPC hints and a 3 MiB
bidirectional job pipe against actual txiki.js processes. Tests under the
fork cover the added native primitives. The real Firecracker test uses a
scripted provider and verifies file/job identity, persistent disks and
external reset; it never calls a real model. Current checkpoint results
are recorded in `progress.md`.

The size experiment under `docs/experiments/txiki-size` records a separate
upstream baseline. Its measurements are historical evidence, not sizes
for the forked runner application.
