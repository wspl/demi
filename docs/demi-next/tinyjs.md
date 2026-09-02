# Demi Next: tinyjs

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (M7) |
| Scope | The QuickJS-based runtime binary that runs the runner and every root command on every execution target |

## Role

tinyjs is a small Rust binary embedding QuickJS. It exists so that the
code on execution targets — the runner and the root commands — can stay JS,
share the protocol schemas and the loader with the backend unchanged, and
still start in tens of milliseconds inside a freshly booted microVM. It is
the only Rust in the system and carries no product logic: it knows nothing
about the runner protocol, the command tree or the backend.

## The stack

One binary on disk; what runs inside it depends on the name it was invoked
by:

```
   invoked as  demi-runner                       invoked as a root command: demi, scout, …
   (PID 1 on a managed host, a service           (spawned by real bash, once per root-command
    on a user host; one process, long-lived)      invocation in a tool call; short-lived)

 ┌───────────────────────────────────────┐    ┌───────────────────────────────────────┐
 │  runtime command module               │    │  runtime command module               │
 │  export default (ctx) => …            │    │  export default (ctx) => …            │
 │  — not run here: the runner only      │    │  — runs HERE against the real fs      │
 │    caches modules for command mode    │    │                                       │
 ├───────────────────────────────────────┤    ├── command ABI (public) ───────────────┤
 │  @demicodes/runner            (JS)    │    │  @demicodes/command-loader    (JS)    │
 │  socket · handshake · Host RPC        │    │  argv[0] = root → its tree → dispatch │
 │  job table · tee · UDS relay          │    │  runtime → run module with ctx        │
 │  manifest cache                       │    │  rpc → UDS → runner → backend         │
 ├───────────────────────────────────────┤    ├───────────────────────────────────────┤
 │  @demicodes/host-runner       (JS)    │    │  @demicodes/host-runner       (JS)    │
 │  the Host contract over the tinyjs API│    │  the Host contract over the tinyjs API│
 ╞═══════════════════════════════════════╡    ╞═══════════════════════════════════════╡
 │  tinyjs API (private)        (Rust)   │    │  tinyjs API (private)        (Rust)   │
 │  fs+errno · spawn+tee                 │    │  fs+errno · spawn · stdio             │
 │  ws · http · uds                      │    │  uds · http                           │
 │  event loop · timers                  │    │  event loop · timers                  │
 │  msgpack · base64 · utf-8 · globals   │    │  base64 · utf-8 · globals             │
 ├───────────────────────────────────────┤    ├───────────────────────────────────────┤
 │  QuickJS (rquickjs)                   │    │  QuickJS (rquickjs)                   │
 └───────────────────────────────────────┘    └───────────────────────────────────────┘
                 the same static binary, a few MB, JS bundled in
```

Two layers of API meet in tinyjs, with different audiences:

- The **tinyjs API** — the primitives the binary exposes to JS. Private:
  only `@demicodes/host-runner` and the runner use it.
- The **command ABI** (`commands.md`) — what a `runtime` command module
  sees. Public. A module never touches the tinyjs API; the Host
  implementation between them is the seam.

The two processes meet on the target through two files the runner owns:
`~/.demi/runner.sock`, the UDS a command-mode process uses for `rpc`
commands and manifest misses, and `~/.demi/commands/<hash>/`, the manifest
cache it reads directly. A command-mode process never opens a network
connection and never holds a credential.

## Why tinyjs, measured

Inside a fresh Firecracker guest under nested virtualization (the realistic
deployment), the first execution of a binary costs time proportional to the
number of executable pages it touches, on top of the runtime's own start-up:

| Runtime | First exec | Second exec | Size |
|---|---|---|---|
| tinyjs, hello | 0.12 s | 0.01 s | 1.4 MB |
| Bun, hello | 0.9–1.7 s | 0.10 s | ~100 MB |
| Bun, the runner | 4.95 s | 1.56 s | 103 MB |

The per-page cost is unaffected by page-cache warming or bytecode caching;
only a small binary removes it. Measurements and the decomposition are in
`progress.md`. A general-purpose small runtime (LLRT) would qualify on size
but carries a Node-compatibility surface we do not want, an experimental
label, and gaps (WebSocket, UDS) that would need Rust modules regardless —
the same work with a larger, borrowed API.

## The tinyjs API

tinyjs runs one bundled ESM module and gives it the few things JS
cannot do for itself. Its API is shaped by that role, not by any existing
runtime:

- **Built-in modules under the `tinyjs:` scheme**, one per area:
  `tinyjs:fs`, `tinyjs:process`, `tinyjs:net`, `tinyjs:bytes`,
  `tinyjs:runtime`. Bundles treat the scheme as external;
  `@demicodes/host-runner` is the only package that imports it, declares
  its types, and exports the typed wrappers the runner and
  `@demicodes/command-loader` use. The tinyjs module loader resolves
  `tinyjs:*` **only for the embedded bundle**: a module loaded from a
  file — a `runtime` command module — that imports it fails to load. That
  check is what makes the tinyjs API private and the command ABI the only
  thing a command module can see.
- **Every IO call returns a `Promise`** and takes and returns plain data.
  Bytes are `Uint8Array`; strings are used only for paths, names and
  signal names.
- **Handles are integers** with an explicit `close(id)` — files,
  processes, sockets and listeners alike. Nothing is released by a GC
  finalizer, so a leak is countable in tests.
- **Reads are pull-model**: `read(id, max)` resolves to bytes or `null` at
  end of stream. There are no WHATWG streams and no event callbacks;
  backpressure is implicit and JS never buffers.
- **Errors are `ShellError { code, errno, syscall, path? }`** with Node's
  string codes (`ENOENT`, `EACCES`, …), so `errorCode` in
  `@demicodes/utils` and the existing Host error mapping apply unchanged.
- **Cancellation is `close` or `kill`.** The primitives do not know
  `AbortSignal`; host-runner maps it.
- `tinyjs:runtime` exports `version` and `abi`, two numbers; host-runner
  checks the abi at start. It also exports `openHandles()`, the count of
  open handles, which is how tests assert nothing leaked.

### Primitives

Every primitive is there because one of the three JS blocks in the stack
needs it; nothing is there for generality. Paths are absolute (host-runner
resolves against the cwd first). Handles (`fd`, process ids, sockets,
listeners) are integers with an explicit `close`.

**`tinyjs:fs`** — the `HostFileSystem` method set one to one, with
errno fidelity throughout:

```ts
readFile(path): Promise<Uint8Array>            // fstat first, one allocation of the exact size
writeFile(path, data, { mode?, append? }): Promise<void>
stat(path) / lstat(path): Promise<Stat>        // { kind, mode, size, mtimeMs, atimeMs, uid, gid, ino, dev, nlink }
readdir(path): Promise<{ name, kind }[]>       // kind: file | dir | symlink | other
mkdir(path, { recursive?, mode? })  rmdir(path)  unlink(path)  rename(from, to)
symlink(target, path)  link(from, to)  readlink(path)  realpath(path)
chmod(path, mode)  utimes(path, atimeMs, mtimeMs)  truncate(path, size)
open(path, flags, mode?): Promise<fd>          // streaming
read(fd, max): Promise<Uint8Array | null>      // null at end of stream
write(fd, data): Promise<void>                 // resolves once the bytes are in the kernel buffer
close(fd)
```

`rm -r`, `cp` and `mv` across devices are composed in host-runner from
these.

**`tinyjs:process`** — runner jobs, Host `spawn`, the Claude Code CLI:

```ts
spawn({
  command, args, cwd, env,                     // env is the complete table; nothing is inherited
  stdin: "pipe" | "null",
  uid?, gid?, processGroup?: boolean,          // uid/gid: how PID 1 runs jobs as the guest user
  tee?: { stdoutPath, stderrPath, viewLimit }
}): Promise<{ pid, stdin: fd | null, stdout: fd, stderr: fd }>   // stdin is null for stdin: "null"
wait(pid): Promise<{ code: number | null, signal?: string, stdoutBytes?, stderrBytes? }>
kill(pid, signal: string, { group?: boolean })                  // a child of this process, not yet reaped
```

`kill` signals only a process this one spawned and has not reaped yet;
any other pid — including `0`, `-1` and the process's own — fails with
`ESRCH` before reaching the kernel, because as PID 1 in a guest a stray pid
would take the machine down.

With `tee`, the full streams are written to the two files inside
tinyjs; the `stdout`/`stderr` fds yield only the first `viewLimit` bytes
and then end, and `wait` reports the full byte counts. Spawn failures map
to the `HostSpawnError` kinds through errno: `ENOENT`, `EACCES`,
`ENOTDIR`, `EISDIR`.

**`tinyjs:net`** — the backend socket, the relay, uploads and
transfers. TLS lives inside these; no TCP or TLS primitive is exposed:

```ts
wsConnect(url, { headers? }): Promise<ws>
wsSend(ws, data): Promise<void>                // resolves once written: that is the backpressure
wsRecv(ws): Promise<Uint8Array | null>
wsClose(ws, code?)
udsConnect(path): Promise<fd>                  // then read/write/close from tinyjs:fs
udsListen(path, { mode }): Promise<listener>   // chmod applied before the first accept
accept(listener): Promise<fd>
close(listener)
httpRequest({ method, url, headers, body?: Uint8Array | { file: path } })
  : Promise<{ status, headers, body: fd }>     // request body streams from the file; response body streams to the reader
```

`wsConnect` and `httpRequest` honour the proxy environment variables and
speak `CONNECT`, because user hosts sit behind corporate proxies.

**`tinyjs:bytes`** — frames, manifest-cache verification, claim codes:

```ts
msgpackEncode(value): Uint8Array               // Uint8Array and Date as extension types
msgpackDecode(bytes): value
base64Encode(bytes): string   base64Decode(text): Uint8Array
sha256(bytes): Uint8Array     randomBytes(n): Uint8Array
```

**`tinyjs:runtime`** — entry-mode selection, `HostIdentity`, PID 1:

```ts
argv: string[]                                 // argv[0] is the invoked name
env: Readonly<Record<string, string>>
cwd(): string   chdir(path)   exit(code)
onSignal(name, handler)                        // SIGTERM, SIGINT, SIGHUP
stdin: fd   stdout: fd   stderr: fd   pid
identity: { uid, gid, hostname, homeDir }
version: number   abi: number
```

**Standard globals**, because libraries look them up by these names:
`setTimeout`/`clearTimeout`/`setInterval`/`clearInterval`/`queueMicrotask`,
`TextEncoder`/`TextDecoder`, `atob`/`btoa`, `URL`/`URLSearchParams`,
`console`, `performance.now`, `crypto.randomUUID`/`crypto.getRandomValues`,
`AbortController`/`AbortSignal`, `structuredClone`. The transcoders, the
timers and the random source are Rust; the classes and `console` are a
small prelude of embedded JS over them.

**Module loading**: the embedded bundle runs at start under the
`/embedded/` namespace; `import()` accepts only absolute file paths, reads
the file and declares it — no npm-style resolution of any kind. `tinyjs:*`
and `/embedded/*` resolve only when the importer is under `/embedded/`: a
module loaded from a file can reach neither the primitives nor the
bundle's own modules (which QuickJS would otherwise hand back from its
module cache). `import.meta.url` is the module's `file:` URL
(`file:///embedded/main.mjs` for the bundle). What `/embedded/` contains is
decided at start (see "Entry modes"): the packed bundle, or the directory
of the entry file given on the command line.

When tinyjs is PID 1 it additionally reaps adopted children itself; that
is its only PID 1-specific behaviour. Mounting and network configuration
are done by spawning `mount` and `ip` from the rootfs, not by primitives.

The rule that decides what belongs here: **JS on tinyjs handles control
flow; any per-byte work is a primitive.** QuickJS has no JIT — a pure-JS
base64 loop measured 2.8 MB/s where the native primitive measured 300 MB/s
— so the tee, the codec and the transcoders are Rust, and the runner's JS
never sees a chunk it has to iterate over. A second reason is the guest
itself: under nested virtualization the first touch of fresh memory is
expensive (50 MB allocated and filled measured 0.9 s on first touch, 0.16 s
on the second), so every large buffer is allocated once, at its known
size, in Rust — never grown incrementally and never copied into the JS
heap.

### Not in the first version

Each has no consumer today and is added only when one appears: pty
(interactive input goes through the job's stdin pipe), HTTP/WebSocket/TLS
servers (the runner is outbound-only; the relay is a UDS), mount and
netlink primitives, fs watching, workers, WebAssembly, compression.

### Protocols come from crates

tinyjs does not implement a wire protocol itself. What the crate owns is
the glue — URLs to connections, connections to integer handles, errors to
`ShellError`, proxy discovery — and the primitives whose semantics we
depend on (errno fidelity, pull-model reads, backpressure, one allocation
per buffer). The protocol work is delegated:

- `tokio-tungstenite` — WebSocket handshake and framing.
- `hyper` (`client`, `http1`) with `hyper-util` — HTTP/1.1, and the
  `CONNECT` tunnel and proxy-environment matcher for user hosts behind
  corporate proxies.
- `rustls` on `ring` through `tokio-rustls`; `rustls-platform-verifier` on
  user hosts, `webpki-roots` in the guest build.
- `rmpv` for MessagePack, `base64`, `sha2`, `getrandom`.
- `rquickjs` for the interpreter and `tokio` (current-thread runtime; the
  context is single-threaded, so none of async Rust's `Send` friction
  applies).

LLRT's module crates were evaluated and rejected (`progress.md`): what they
provided correctly was the trivial part, while the primitives with
semantics we depend on fell short.

The Host implementation `@demicodes/host-runner` maps the `Host` contract
(`packages/shell/src/host.ts`) onto these primitives. It is the Host every
user host and managed host serves through its runner; the backend's end of
the same connection is `RemoteHost` in `@demicodes/runner-protocol`
(`runner.md`).

## Entry modes

tinyjs is a prebuilt runtime that carries no Demi JS of its own. A
deliverable is made the way `bun build --compile` and Node's single
executable applications make one: the bundle is **packed** onto a copy of
the bare binary. The packing is done by **`tinyjsc`**, a second binary
built from the same crate and released together with tinyjs; tinyjs itself
carries no packing code and parses no packing arguments — it only reads
the section at start.

**Packed binary.** `tinyjsc` compiles the bundle to QuickJS bytecode with
the interpreter of its own release and places it in a section of the bare
executable through `libsui`, the injector behind `deno compile`: on Mach-O
a new segment with the load commands adjusted and an ad-hoc signature
applied, on ELF a `PT_NOTE` grafted after the image. At start tinyjs asks
`libsui::find_section` for it in its own mapped image and loads the
bytecode without parsing; nothing is read from disk and no source is
scanned. Bytecode is always used, there is no source option. Bytecode is
tied to the interpreter build, so the section starts with a header naming
the release (the crate version) and the `abi` it was compiled by, and every
tinyjs binary carries a release marker in its bytes: `tinyjsc` reads the
marker and refuses a binary of another release or one already packed, and
tinyjs refuses at start a section from another release. The packed file is
one file on disk, reached through symlinks:

- `demi-runner` — **runner mode**: runs the runner program (`runner.md`).
- any other name — **command mode**: runs the loader with `argv[0]` as the
  root command (`commands.md`). `demi` is the built-in root; a library
  user's `scout` is another symlink to the same file. The runner
  maintains the symlinks from the manifest's root set.

The mode is selected by `argv[0]` inside the bundle; Rust parses no
arguments. There is no separate JS file to install and no `node_modules`
on a target. Packing is a separate tool because packing is always cross:
one machine packs every target, and the runtime carries no code it never
runs on a target.

```
tinyjsc <bundle.mjs> --bin <bare tinyjs of the target platform> --out <file>
```

Compiling the bundle resolves its import graph with the real loader
against a payload that has no modules beside the entry, so a bundle is one
file: an import of anything but `tinyjs:*` fails at pack time rather than
on a target. The bare binary is any platform's build of the same release, so one machine
packs every target (all four targets are 64-bit little-endian, so the
bytecode is the same). The result passes `codesign --verify --strict` on
macOS; a distributed build replaces the ad-hoc signature with the Developer
ID at release time. macOS binaries are linked with `-headerpad` so the
injected load command has room, and `tinyjsc` refuses a binary without it
rather than overwrite the start of `__text`. A packed binary parses no
arguments of its own: everything goes to the bundle. Two other mechanisms
were measured and rejected (`progress.md`): appending after the Mach-O
executes but fails strict validation, and `postject`/LIEF mis-relocates a
Rust binary's TLS sections.

**Bare binary.** Without a payload, `tinyjs <entry.mjs> [args…]` runs the
entry file as the embedded bundle: the entry's directory is `/embedded/`,
so the entry and the files it imports relatively see `tinyjs:*`, while
anything reached through an absolute `import()` still does not. This is
the development and test entry; `argv[0]` is the entry path. With neither
a payload nor an entry, tinyjs prints its usage and exits with 2.

## Packaging

- Static musl builds for Linux x86_64 and aarch64 (the managed-host rootfs
  and user hosts), macOS arm64 and x86_64 builds (user hosts). Windows has
  no bash and is not a target.
- **Size**: 2.3 MB on macOS arm64, 2.6 MB Linux aarch64 musl, 3.0 MB
  Linux x86_64 musl, with every primitive in (QuickJS, rustls with ring,
  hyper, tungstenite, tokio). Every added primitive is
  weighed against the first-execution cost it adds, not against a byte
  budget.
- **The startup path touches no network code.** First-execution cost in a
  guest is paid per page touched, not per byte on disk (a 5.5 MB spike
  binary started only 70 ms slower than a 1.45 MB one because its TLS and
  HTTP code never ran). The TLS configuration, root store and connectors
  are built on the first `wsConnect`/`httpRequest`; a command-mode process
  never builds them. The conformance suite asserts that command-mode
  `hello` first-exec stays in the same class as the bare interpreter.
- Build profile: `opt-level = "z"` for the crate with `opt-level = 3`
  kept for `rquickjs-sys` (the interpreter is all of JS performance; the
  rest is IO glue); fat LTO, one codegen unit, `panic = "abort"`, stripped.
  QuickJS is built without the bignum extension and dump facilities.
  `tokio` carries only the features used. No UPX: decompressing the whole
  binary on every start is exactly the fresh-page cost the guest punishes.
- Root certificates: the guest build (`guest-roots` feature) embeds
  `webpki-roots`; user-host builds use the platform verifier (system
  keychain and certificate store), which is what makes corporate MITM
  proxies work. The conformance run can point `TINYJS_CONFORMANCE_CA` at
  its own test CA instead.
- Version reported in the runner `hello`; the backend refuses a tinyjs older
  than the protocol it speaks.
- `tinyjsc` is built for the machines that pack (developer machines and
  CI), never shipped to a target; a release is the bare `tinyjs` per
  target plus one `tinyjsc`, all from one crate version.

## Packages

- `packages/tinyjs` — the Rust crate, two binaries: `tinyjs`, the runtime,
  and `tinyjsc`, the packer. `src/` is split by area (event loop, loader,
  handles, fs, process, net with connect/tls/ws/http/uds, bytes, runtime,
  globals); the packer's injection and the release check live only in
  `tinyjsc`. Dependencies are those listed under "Protocols come from
  crates".
- `@demicodes/host-runner` — the Host over the tinyjs API (TypeScript, runs
  only on tinyjs, inside the runner). Its `/testing` entry finds the
  binaries and bundles an entry for tinyjs (Bun, one ESM file, `tinyjs:*`
  external), which is how the Host conformance suite of
  `@demicodes/shell/testing` runs on tinyjs from `cargo`-free Bun tests.
- The bundle's entry is `packages/runner/src/tinyjs/entry.ts`: the name
  tinyjs was invoked by selects runner mode or the root command.
- The primitive conformance suite is JS (`packages/tinyjs/conformance`),
  run on the bare binary as `tinyjs conformance/main.mjs` and driven by
  `cargo test`, which provisions ports, a test CA and the Bun stub server.
  It runs on every build target and is the definition of done for tinyjs.
