# Native IPC client size experiment

Measured on 2026-09-08 using Demi `2bcd0c3a` and its pinned txiki.js
fork `1a4c5b01`. This is an experiment, not a replacement for the current
`demi` command or its relay protocol. Production code is unchanged.

## Question and result

A future `demi` executable could submit raw arguments and invocation context
to the local runner, which would own command parsing and dispatch. Would a
C client using libuv be substantially smaller than a standalone txiki.js
client performing the same transport work?

Yes. The C + libuv probe is approximately 105–113 KiB on the tested Unix
platforms, versus 2.50–2.75 MiB for the txiki.js probe: approximately 96%
smaller. The Windows C + libuv executable cross-build is approximately
141 KiB. These are transport-probe sizes, not estimates verified against a
complete future command client.

| Implementation | macOS ARM64 bytes | Linux ARM64 musl bytes | Windows x64 bytes |
|---|---:|---:|---:|
| Native POSIX C | 52,224 | 15,048 | not implemented |
| C + static libuv | 107,184 | 115,288 | 143,872 |
| Embedded txiki.js | 2,624,944 | 2,879,176 | not built |

All variants use the same byte-forwarding behavior: connect to the supplied
endpoint, send stdin incrementally, half-close the send side at stdin EOF,
and copy received bytes to stdout until the peer closes. The native POSIX
probe uses one sender thread and one receiving thread. The libuv probe uses
`uv_pipe_connect`, asynchronous stdin reads and socket writes, and bounded
receive buffers. The JavaScript probe uses `tjs.connect('pipe', ...)` and
Web Streams. None contains command implementations or the Demi wire codec.

The txiki.js probe contains only this small script, not the full runner
bundle. It uses the existing production runtime build settings, with FFI,
SQLite, WASM, and mimalloc disabled and the public standard library retained.
This makes the comparison about the runtime needed for equivalent transport
work, rather than comparing a tiny C program with all of Demi's application
code. Removing more txiki.js features is outside this experiment.

## Platform behavior

[libuv pipe handles](https://docs.libuv.org/en/v1.x/pipe.html) use Unix domain
sockets on Unix and named pipes on Windows. The current txiki.js pipe
implementation also calls libuv's pipe APIs. A JS engine is not required to
obtain that common transport abstraction.

For example, the transport can connect to `/path/to/runner.sock` on Unix or
`\\.\pipe\demi-<user>-<instance>` on Windows. A production implementation
still needs platform-specific endpoint discovery and access controls:
filesystem permissions on Unix, named-pipe access control on Windows.
The current runner adapter unconditionally applies `chmod` to the socket
path; this is not a complete Windows implementation today.

The macOS C client statically includes libuv and imports only the system
`libSystem` library. Linux C clients are stripped static musl ELF files.
The Windows PE includes libuv and imports Windows system/UCRT DLLs, not a
separately deployed libuv DLL. Windows execution was not verified because
no Windows execution environment was available.

## Verification and limits

All three implementations ran successfully on both macOS ARM64 and a real
Linux ARM64 Lima VM. Each passed:

- Empty input and a small payload containing NUL, non-UTF-8 bytes, and CRLF.
- An exact 3 MiB byte round trip using piped stdin/stdout.
- The same round trip with regular-file stdin/stdout redirection.
- Failure when connecting to a nonexistent endpoint.

The fixture server reads in 7,919-byte chunks, independently of the client
buffer size. Each process has a timeout, and output is compared byte for
byte. The tests do not use models or the Demi backend. macOS C binaries
were stripped and ad-hoc signed. The Windows binary was only cross-built
and inspected as a PE file.

The probes deliberately omit request metadata, framing, stderr and exit
messages, protocol versioning, caller authorization, cancellation control,
terminal behavior, and Windows Unicode argument handling. The libuv probe
uses synchronous stdout writes inside its read callback; this bounds memory
but can block its event loop while the consumer is stalled. A production
client needs asynchronous output handling and cancellation that remains
responsive under backpressure. A peer closing is considered success in this
probe; a real client must require the command's explicit exit response.

These omissions mean the probes are not production clients. They still
demonstrate that adding libuv does not imply adding a multi-megabyte runtime.
The remaining protocol and lifecycle work must be designed and tested before
changing the real command architecture.

## Reproduction

On a macOS ARM64 host with the repository's recursive submodules, Bun,
Apple Clang, CMake, Zig, and Python installed:

```sh
docs/experiments/demi-ipc-size/build.sh
```

`CMAKE` and `ZIG` may be absolute executable paths. Outputs and build logs
belong under `.cache/demi-ipc-size`. The script builds the native macOS
clients, Linux ARM64 clients, Windows x64 libuv client, and runs the macOS
checks. Copy the Linux executables and `verify.py` to Linux and invoke
`python3 verify.py <posix-client> <uv-client> <tjs-client>` for the Linux
checks. The script reuses the ordinary txiki.js build caches; it does not
replace any released runner file.

Measurements used Apple Clang 21, Zig 0.16, MinSizeRel/`-Os`, stripping,
dead-code removal, and ThinLTO for Unix probes and the Windows libuv archive.
The Windows final link uses `zig cc` directly because CMake's whole-archive
link command triggered unresolved CRT symbols with this Zig toolchain.

The result supports using a small C + libuv client for a future command
transport while retaining TypeScript/txiki.js for runner orchestration.
Using native POSIX calls alone saves more bytes but requires a separate
Windows transport implementation; that tradeoff is not necessary to achieve
most of the observed size reduction.
