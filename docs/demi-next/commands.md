# Demi Next: The Command System

| | |
|---|---|
| Date | 2026-09-08 |
| Status | Target architecture contract; acceptance tracked in `progress.md` |
| Scope | The command system: root commands, organizing rule, command kinds, the command ABI, the manifest, the loader, root commands on a target |

## Root commands

The command system is a mechanism, not a command. A **root command** is a
top-level name the manifest declares — `demi` is the built-in root Demi's
agent ships with, and a library user who builds another agent on Demi
declares their own root (`scout`) with the same tree types, kinds, ABI,
manifest, loader and target-side entry. Nothing below is specific to
`demi` except its contents.

The accepted target is a native C + libuv command client, separate from
`demi-runner`. It sends raw invocations to the runner, which owns the loader
and dispatch. [Native command client and IPC](command-client.md) defines
that boundary, discovery and access control. The command trees, kinds,
manifest and module ABI below remain shared.

## Organizing rule

Demi's agent works entirely through shell commands. **Every Demi-specific
capability lives under the `demi` root, and every `demi` subcommand is a
noun domain group** (`file`, `todo`, `agent`, `host`, …); anything outside
a root is an ordinary shell command run by the target's real bash. **Group
nodes navigate; leaf nodes execute**: a `Command` with `subcommands` has no
`run`; invoking a group bare prints its help, and the dispatcher returns
help rather than "requires a subcommand" when argv is exhausted on a group.
`demi agent spawn [--profile] [--description] [prompt]` is the spawn leaf;
`agent` itself is a group. A user-defined root follows the same rule for
its own groups.

The trees are **defined once, in the backend**: `@demicodes/coding-agent`
declares the `demi` root's agent-facing groups, the backend composition
root contributes the groups that need backend state (`host`), an embedding
library user adds their roots, and the backend builds one manifest from
all of them. No other process holds a command definition.

## Execution flow

Every tool script runs through the selected device's runner and real bash
(`sessions-and-targets.md`). Cloud is obtained or woken before execution when
needed. The command manifest is shared across paired devices and Cloud.

### On a paired device or Cloud

Real bash on the target parses the tool call. `demi` is an ordinary program
in `PATH`; everything else is whatever the machine has.

```
 tool call:  demi file edit src/a.ts <<'EOF' … EOF && npm test 2>&1 | tail -20

 backend                          runner on the target                 processes on the target
 ───────                          ────────────────────                 ───────────────────────
 job_start {script, cwd,   ────▶  spawn  bash -c "<script>"     ────▶  bash
            env + conv/shell ids}   │  tee stdout/stderr → output         │
                                    │  files under commandOutputDir        ├─ demi file edit src/a.ts        (native C client)
                                    │                                      │    read ${DEMI_HOME}/commands/<hash>/   manifest cache
                                    │                                      │    kind = runtime
                                    │                                      │    → run the module in-process, ctx.fs = real fs
                                    │                                      │    (zero wire bytes)
                                    │                                      │
                                    │                                      ├─ npm test 2>&1 | tail -20       (ordinary processes;
                                    │                                      │    the pipe is an OS pipe)
 ◀── job_output {the model's view} ─┘                                      │
 ◀── job_exit {code, cwd, output paths, tails} ◀── bash exits ◀────────────┘

 an rpc command inside the same script, e.g.  demi todo add "run the suite":

                                                                        runner dispatch: kind = rpc
                                  runner  ◀───────── UDS ─────────────  → parsed args; the pipe as frames
 ◀── rpc_call {conv id, shell id, ──┘  attributes by the ids in the
     root, command, args, stdin?}      job's environment
 ─── rpc_pipes {stdin, stdout} ─▶ runner: PUT the pipe, GET the stdout    HTTP streams brokered by the backend,
     backend runs the command                                             never bytes on the socket
     against conversation state
 ─── rpc_output / rpc_exit ─────▶ runner ──────────── UDS ────────────▶ native client writes stdout, exits with the code
```

What crosses the runner socket: the script, the model's view of the
output (the first bytes while running, the last bytes at exit —
`runner.md`), the exit, and the arguments, stderr and exit code of `rpc`
commands. An `rpc` command's stdin and stdout are pipes — HTTP streams
brokered by the backend (`runner.md` § Pipes). File contents and pipeline
bytes never ride the socket.

## Command kinds

An executable leaf may declare `runningHint`, model-facing guidance shown
while that invocation is active. The manifest preserves it, and the loader
reports it only after help and argument validation have selected an actual
execution. Each invocation clears its hint on completion or cancellation;
concurrent pipelines keep independent hints. The runner reports the lifecycle over its
relay (`runner.md`). `demi agent spawn` and `resume` use this to explain
steering, abort and completion wakeups instead of suggesting polling.

Every leaf is one of two kinds:

- **`rpc`** — the implementation runs in the backend. The command needs
  backend state: `todo` (`CommandStorage`), `agent` (the subagent
  supervisor), `host` (registry, attached hosts, provisioner). An invocation on a
  target travels to the backend as a typed message carrying the parsed
  arguments; its stdin and stdout are pipes brokered over HTTP
  (`runner.md` § Pipes), its stderr and exit code come back on the
  socket. The handler reads `ctx.stdin` as an `AsyncIterable<Uint8Array>`;
  a leaf that declares `stdinField` has the loader collect the pipe into
  that argument before parsing.
- **`runtime`** — the implementation is an ES module shipped to wherever the
  command is invoked and run there against that place's filesystem. `demi
  file read/create/edit/patch` and future `demi search` are `runtime`. On a
  target the module runs inside an isolated txiki.js runner worker with zero round
  trips.

The rule is mechanical: **a command that touches only the target's
filesystem is `runtime`; a command that touches conversation or platform
state is `rpc`.** There is no third kind and no per-target implementation.

## The command ABI

A leaf declares its kind next to its schema. An `rpc` leaf carries `run`,
the backend handler; a `runtime` leaf carries `module`, the **text** of an
ES module with one export:

```ts
export default async function (ctx: CommandContext): Promise<CommandResult>
```

`CommandResult` is `{ exitCode }`; everything a command has to say goes
through its streams, so the result is the same object on every surface.

Arguments reach the module as written. A module resolves paths through
`ctx.fs` with `ctx.cwd`, so messages retain the caller's spelling. The command
input schema validates arguments; filesystem access follows the selected Host's
policy. No separate path-admission metadata is needed.

`ctx` is the whole world the module sees:

| Field | Meaning |
|---|---|
| `args` | the parsed arguments, already validated against the command's zod input schema |
| `fs` | the `HostFileSystem` facet of the Host the command runs against |
| `cwd` | the invoking shell's working directory as a path string |
| `env` | the invoking shell's exported environment |
| `stdin` | the command's stdin as a byte stream |
| `stdout`, `stderr` | writers |
| `signal` | an `AbortSignal` for cancellation |

A module is one self-contained file: what it uses beyond its `ctx` is the
standard ECMAScript library plus the Web-platform globals every embedder
guarantees (`TextEncoder`, `TextDecoder`, `URL`, `atob`, `btoa`,
`crypto.randomUUID`, `AbortSignal`). Nothing bundles the module, so an
import of a value fails where the module loads, the same in every
embedder; type imports are erased by the transpiler. This is what makes
one module run identically inside txiki.js against a real filesystem,
and inside a test with an injected Host. It is the public contract a third
party builds on, versioned with the manifest.

Byte-heavy work inside a module goes through `ctx.fs` and the streams,
which every embedder implements natively; a module never loops over bytes
in JS (txiki.js has no JIT — `txiki.md`).

### How a tree carries a module

A module is one self-contained file named `<leaf>.command.ts`,
type-checked like any other source; the helpers it needs live in it. The
tree that declares the leaf imports it as text and hands the text to
`runtimeModule`:

```ts
import readModule from './read.command.ts' with { type: 'text' }

{ name: 'read', kind: 'runtime', module: runtimeModule(readModule), input: { path: z.string() }, positionals: ['path'] }
```

Bun honors the `text` import attribute natively (development, tests, the
backend). tsdown does not — rolldown resolves the file as a module — so
every package that declares runtime leaves builds with the
`commandModulesAsText` plugin from `@demicodes/command-loader/build`, which serves
`*.command.ts` files as text by name; the built package carries the module
text inline. TypeScript types the import as the module, not as a string,
so `runtimeModule` is the one declared conversion, and it checks at
runtime that it received text: a build that forgot the plugin fails when
the tree is constructed, not when a manifest ships a function.

## The manifest

The backend builds a manifest from the assembled tree at start-up and
whenever the tree changes:

```
manifest
  hash                       content hash of everything below
  roots[name]                one tree per root command (`demi`, `scout`, …):
    tree                     groups and leaves: name, kind, help, input and output schemas (JSON Schema),
                             and for a runtime leaf the hash of its module
  modules[hash]              one self-contained ESM file per runtime leaf
```

The build takes each `runtime` leaf's module text, transpiles it to
JavaScript, and stores it under the hash of the result. The build is the
loader's `buildManifest`, with the transpiler injected by the composition
root (Bun's, in the backend); the loader itself never transpiles. The manifest is served
by the backend to embedders over their existing connection (the runner
socket) and by HTTP for the standalone case; modules are fetched by hash
and cached forever — a new tree is a new manifest hash, never a mutated
module.

## The loader

`@demicodes/command-loader` is a pure-JS library with no runtime
dependency. It is the one place that knows how to run a command:

```
loader = await createLoader({ source, host, rpc? })
loader.dispatch(root, argv, io) → exit code          io: DispatchIO (stdin stream, stdout, stderr, cwd, env, signal)
loader.roots                                          the manifest as command trees
```

- `source` yields the manifest — held in memory (`inMemorySource`), read
  from a directory (`directorySource`: `manifest.json` beside
  `modules/<hash>.mjs`, the layout `writeManifestDirectory` produces),
  received over the runner's connection, fetched from a URL. A source that
  keeps modules as files names them (`modulePath`); the directory source
  is also the cache on a target.
- `host` is the Host the `runtime` modules run against.
- `rpc`, when present, carries typed `rpc` invocations (`RpcInvocation`:
  root, path, parsed args, `--json`, the pipe as a stream, cwd, env, plus
  the stdio and post-start stdin to relay); an embedder without one serves
  only `runtime` commands, and an `rpc` leaf reports the missing
  transport. `inProcessRpc(roots, { storage, host })` supports library
  embeddings and tests whose handlers are in the same process.

Dispatch selects the root's tree, resolves the path through it, prints help
for a group, parses and validates the leaf's arguments against its schema,
then either runs the module with a `ctx` built from `host`, `io` and the
arguments, or sends the `rpc` message. A module is imported from a `blob:`
URL of its text (library embeddings and tests) or from the source's module file
(txiki.js imports only files), so the same bytes run everywhere. Help text comes from the tree,
so `demi file --help` is identical on every target.

Embedders:

| Embedder | Source | Host | rpc |
|---|---|---|---|
| runner | the backend socket, cached and pinned per job under `${DEMI_HOME}/commands/<hash>/` | machine layer; runtime leaves execute in isolated workers | the authenticated backend socket and HTTP pipes |
| tests | in-memory | in-memory Host | stub |

A third party who wants Demi's commands in another agent needs the loader,
a Host implementation and a manifest source — no runner, no txiki.js, no
backend.

## Root commands on a target

A root command is the standalone C + libuv client (`command-client.md`).
Real bash invokes it through the root alias in the job's pinned manifest
bin directory. Its basename identifies the root; raw argv, current cwd/env
and live execution context travel to the job's exact runner endpoint.
The client loads no manifest and performs no command parsing.

The runner validates that context, selects its manifest, and dispatches
through `command-loader`. Runtime modules use isolated workers; backend
handlers use authenticated RPC with HTTP pipes for stdin/stdout. The
runner returns byte-faithful output and final status over local IPC.
The client has no credential and cannot select another backend by changing
session IDs. Ordinary terminal invocation without a live context fails.

The runner snapshots a manifest for each job/spawn and injects its bin PATH,
`DEMI_RUNNER_ENDPOINT` and `DEMI_CONTEXT_ID`. `${DEMI_HOME}/commands/current`
selects the manifest for new contexts; existing contexts retain theirs.
`DEMI_HOME` defaults to `~/.demi/instances/<backend-hash>` on paired devices
and `/run/demi` in managed guests.

## The `demi host` group

Contributed by the backend composition root because it needs the registry,
the attached-host table and the provisioner:

```
demi host                                         help for the group
demi host list                                    the main host, marked, and every attached host: name, id, online, directory
demi host current                                 the main host
demi host shell --host <name|id> <shell_content>  run a shell string in that host's real bash; byte-faithful stdio
```

`<shell_content>` is one positional argument executed by that host's
`bash -c`, so pipes, redirections and globs apply remotely. It starts in
the host's recorded working directory — for the main host the
conversation's directory there, for an attached host the directory its
last `shell --host` ended in (`sessions-and-targets.md` § Attached hosts)
— and the directory it ends in is recorded for the next. The caller's
stdin and stdout are attached to the remote job's, both streaming while
it runs — pipes brokered by the backend between the two runners over
HTTP (`runner.md` § Pipes), never over the runner sockets — so `demi host
shell --host ci "tar c -C /work ." | tar x` lands a working tree byte for
byte, and `tar c . | demi host shell --host ci "tar x -C /work"` does the
same the other way; stderr and the exit code pass through as they happen.
`--host` takes the name `demi host list` prints or the device id. The
reachable set is the main host plus the attached hosts, checked in one
place.

## Packages

- `@demicodes/shell` keeps the `Command` types (tree, input/output specs,
  `CommandContext`, `CommandResult`, `runtimeModule`), the Host
  contract and the `ShellEnvironment` contract. It carries no engine.
- `@demicodes/coding-agent` declares the `demi` root's agent-facing groups
  with the `kind` on each leaf; the `runtime` leaves are `*.command.ts`
  files written against the ABI. A library user declares their own root
  the same way, with the same types.
- `@demicodes/command-loader` owns: the manifest types, the manifest
  build (`buildManifest`, the transpiler injected), the loader,
  and the `commandModulesAsText` build plugin under
  `@demicodes/command-loader/build`.
- `@demicodes/host-remote` owns `RemoteHost` and `RemoteShellEnvironment`:
  the backend's machine interface and shell job view.
- `@demicodes/backend` assembles roots, builds and serves the manifest and
  authenticates RPC invocations against live jobs. Its shell-environment
  factory supplies `RemoteShellEnvironment` for the resolved device. It does
  not execute shell scripts. `shell_write` reaches the job's live stdin,
  including a foreground `demi agent spawn` command.
