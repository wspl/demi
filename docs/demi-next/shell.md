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
 │  fs+errno · spawn+tee · pty           │    │  fs+errno · spawn · stdio             │
 │  tcp/tls/ws/uds · http                │    │  uds · http                           │
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

## Primitives

Everything JS cannot do itself, and everything that loops over bytes:

| Area | Primitives |
|---|---|
| Module loading | ESM from files and from strings; `import()` |
| Event loop | timers, microtask and job scheduling, async completion for every IO primitive below |
| Filesystem | the `HostFileSystem` method set with **errno fidelity**: open/read/write/stat/readdir/mkdir/rename/unlink/symlink/chmod/utimes/truncate, streaming read and write |
| Processes | spawn with pipes, stdin write, kill by signal name, exit code and signal; **tee**: stdout/stderr written to a file on the target with only a bounded view returned to JS; pty allocation for interactive commands |
| Network | TCP client and listener, TLS client, WebSocket client, HTTP client, Unix domain sockets (client and listener) |
| Bytes | base64, MessagePack encode/decode, UTF-8 encode/decode |
| Process | argv, env, cwd, exit, signals, stdin/stdout/stderr streams |
| Globals | `TextEncoder`, `TextDecoder`, `atob`, `btoa`, `URL`, `crypto.randomUUID`, `crypto.getRandomValues`, `performance.now`, `console`, `AbortController`, `structuredClone` |

The rule that decides what belongs here: **JS on the shell handles control
flow; any per-byte work is a primitive.** QuickJS has no JIT — a pure-JS
base64 loop measured 2.8 MB/s where the native primitive measured 300 MB/s
— so the tee, the codec and the transcoders are Rust, and the runner's JS
never sees a chunk it has to iterate over.

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
