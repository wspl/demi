# Demi Next: The Shell

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (M7) |
| Scope | The QuickJS-based runtime binary that runs the runner and every root command on every execution target |

## Role

The shell is a small Rust binary embedding QuickJS. It exists so that the
code on execution targets — the runner and the root commands — can stay JS,
share the protocol schemas and the loader with the backend unchanged, and
still start in tens of milliseconds inside a freshly booted microVM. It is
the only Rust in the system and carries no product logic: it knows nothing
about the runner protocol, the command tree or the backend.

## The stack

One binary on disk; what runs inside it depends on the name it was invoked
by:

```
   invoked as  demi-runner                       invoked as a root command: demi, myagent, …
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
 │  @demicodes/host-shell        (JS)    │    │  @demicodes/host-shell        (JS)    │
 │  the Host contract over the shell API │    │  the Host contract over the shell API │
 ╞═══════════════════════════════════════╡    ╞═══════════════════════════════════════╡
 │  shell API (private)         (Rust)   │    │  shell API (private)         (Rust)   │
 │  fs+errno · spawn+tee                 │    │  fs+errno · spawn · stdio             │
 │  ws · http · uds                      │    │  uds · http                           │
 │  event loop · timers                  │    │  event loop · timers                  │
 │  msgpack · base64 · utf-8 · globals   │    │  base64 · utf-8 · globals             │
 ├───────────────────────────────────────┤    ├───────────────────────────────────────┤
 │  QuickJS (rquickjs)                   │    │  QuickJS (rquickjs)                   │
 └───────────────────────────────────────┘    └───────────────────────────────────────┘
                 the same static binary, a few MB, JS bundled in
```

Two layers of API meet in the shell, with different audiences:

- The **shell API** — the primitives the binary exposes to JS. Private:
  only `@demicodes/host-shell` and the runner use it.
- The **command ABI** (`commands.md`) — what a `runtime` command module
  sees. Public. A module never touches the shell API; the Host
  implementation between them is the seam.

The two processes meet on the target through two files the runner owns:
`~/.demi/runner.sock`, the UDS a command-mode process uses for `rpc`
commands and manifest misses, and `~/.demi/commands/<hash>/`, the manifest
cache it reads directly. A command-mode process never opens a network
connection and never holds a credential.

## Why a shell, measured

Inside a fresh Firecracker guest under nested virtualization (the realistic
deployment), the first execution of a binary costs time proportional to the
number of executable pages it touches, on top of the runtime's own start-up:

| Runtime | First exec | Second exec | Size |
|---|---|---|---|
| QuickJS shell, hello | 0.12 s | 0.01 s | 1.4 MB |
| Bun, hello | 0.9–1.7 s | 0.10 s | ~100 MB |
| Bun, the runner | 4.95 s | 1.56 s | 103 MB |

The per-page cost is unaffected by page-cache warming or bytecode caching;
only a small binary removes it. Measurements and the decomposition are in
`progress.md`. A general-purpose small runtime (LLRT) would qualify on size
but carries a Node-compatibility surface we do not want, an experimental
label, and gaps (WebSocket, UDS) that would need Rust modules regardless —
the same work with a larger, borrowed API.

## The shell API

The shell runs one bundled ESM module and gives it the few things JS
cannot do for itself. Its API is shaped by that role, not by any existing
runtime:

- **Built-in modules under the `demishell:` scheme**, one per area:
  `demishell:fs`, `demishell:process`, `demishell:net`, `demishell:bytes`,
  `demishell:runtime`. Bundles treat the scheme as external;
  `@demicodes/host-shell` is the only package that imports it, declares
  its types, and exports the typed wrappers the runner and
  `@demicodes/command-loader` use. The shell's module loader resolves
  `demishell:*` **only for the embedded bundle**: a module loaded from a
  file — a `runtime` command module — that imports it fails to load. That
  check is what makes the shell API private and the command ABI the only
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
  `AbortSignal`; host-shell maps it.
- `demishell:runtime` exports `version` and `abi`, two numbers; host-shell
  checks the abi at start.

### Primitives

Every primitive is there because one of the three JS blocks in the stack
needs it; nothing is there for generality.

| Area | Primitives | Needed by |
|---|---|---|
| Module loading | the embedded bundle runs at start; `import()` of an **absolute file path** loads a second module at run time — no npm-style resolution of any kind | `runtime` command modules from `~/.demi/commands/<hash>/` |
| Event loop | `setTimeout`/`clearTimeout`/`setInterval`/`queueMicrotask`; every IO completion below is delivered on the same loop | all |
| Filesystem (`demishell:fs`) | the `HostFileSystem` method set one to one — `readFile`/`writeFile`/`appendFile`, `stat`/`lstat`, `readdir` with types, `mkdir`, `rm`, `cp`, `mv`, `chmod`, `symlink`/`link`/`readlink`/`realpath`, `utimes` — plus `open`/`read`/`write`/`close` for streaming; errno fidelity throughout | host-shell |
| Processes (`demishell:process`) | `spawn({ command, args, cwd, env, stdin, uid?, gid?, killProcessGroup?, tee? })` → handle with stdin/stdout/stderr ids; `kill(id, signal)`; `wait(id)` → `{ code, signal }`. **tee**: with `tee: { stdoutPath, stderrPath, viewLimit }` the full streams are written to those files inside the shell and JS reads only the first `viewLimit` bytes of each, then `null`; `wait` also reports the full byte counts. `uid`/`gid` is how PID 1 runs jobs as the guest user | runner jobs, Host `spawn`, the Claude Code CLI |
| Network (`demishell:net`) | `wsConnect(url, headers)` → send bytes, receive bytes, close; `httpRequest({ method, url, headers, body: bytes \| file id })` → status, headers, body id; `udsConnect(path)`, `udsListen(path)`/`accept`. TLS lives inside these; **no TCP or TLS primitive is exposed** — nothing needs one | the backend socket, the relay, `output_upload`, transfers |
| Bytes (`demishell:bytes`) | MessagePack encode/decode (`Uint8Array` and `Date` as extension types), base64, SHA-256, random bytes | frames, manifest-cache verification, claim codes |
| Own process (`demishell:runtime`) | `argv`, `env` (read-only snapshot), `cwd`/`chdir`, `exit`, `onSignal`, pre-opened stdin/stdout/stderr ids, `pid`, `identity` (`uid`/`gid`/`hostname`/`homeDir`), `version`/`abi` | entry-mode selection, `HostIdentity`, PID 1's `SIGTERM` |
| Globals | standard names only, because libraries look them up as globals: `TextEncoder`, `TextDecoder`, `atob`, `btoa`, `URL`, `URLSearchParams`, `crypto.randomUUID`, `crypto.getRandomValues`, `performance.now`, `console`, `AbortController`, `structuredClone` | zod, the protocol package, the loader |

When the shell is PID 1 it additionally reaps adopted children itself; that
is its only PID 1-specific behaviour. Mounting and network configuration
are done by spawning `mount` and `ip` from the rootfs, not by primitives.

The rule that decides what belongs here: **JS on the shell handles control
flow; any per-byte work is a primitive.** QuickJS has no JIT — a pure-JS
base64 loop measured 2.8 MB/s where the native primitive measured 300 MB/s
— so the tee, the codec and the transcoders are Rust, and the runner's JS
never sees a chunk it has to iterate over.

### Not in the first version

Each has no consumer today and is added only when one appears: pty
(interactive input goes through the job's stdin pipe), HTTP/WebSocket/TLS
servers (the runner is outbound-only; the relay is a UDS), mount and
netlink primitives, fs watching, workers, WebAssembly, compression.

The Host implementation `@demicodes/host-shell` maps the `Host` contract
(`packages/shell/src/host.ts`) onto these primitives, exactly as
`@demicodes/host-local` maps it onto Node.

## Entry modes

One binary, one file on disk, reached through symlinks:

- `demi-runner` — **runner mode**: runs the runner program (`runner.md`).
- any other name — **command mode**: runs the loader with `argv[0]` as the
  root command (`commands.md`). `demi` is the built-in root; a library
  user's `myagent` is another symlink to the same file. The runner
  maintains the symlinks from the manifest's root set.

The mode is selected by `argv[0]`. The JS for both modes is bundled into
the binary; there is no separate JS file to install, no `node_modules` on a
target.

## Packaging

- Static musl builds for Linux x86_64 and aarch64 (the managed-host rootfs
  and user hosts), macOS arm64 and x86_64 builds (user hosts). Windows has
  no bash and is not a target.
- Size budget: a few MB. Every added primitive is weighed against the
  first-execution cost it adds.
- Version reported in the runner `hello`; the backend refuses a shell older
  than the protocol it speaks.

## Packages

- `packages/demi-shell` — the Rust crate: `src/` split by area (loop, fs,
  process, net, bytes, globals, entry). Depends on `rquickjs` and the
  standard library plus rustls; no other runtime dependency.
- `@demicodes/host-shell` — the Host over the shell API (TypeScript, runs
  only on the shell).
- The primitive conformance suite is JS, run on every build target; it is
  the shell's definition of done.
