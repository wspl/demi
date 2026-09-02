# Demi Next: The Command System

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (lands in M8; the CLI's `rpc` path completes in M9) |
| Scope | The command system: root commands, organizing rule, command kinds, the command ABI, the manifest, the loader, tinybash and hostless execution, root commands on a target |

## Root commands

The command system is a mechanism, not a command. A **root command** is a
top-level name the manifest declares — `demi` is the built-in root Demi's
agent ships with, and a library user who builds another agent on Demi
declares their own root (`myagent`) with the same tree types, kinds, ABI,
manifest, loader and target-side entry. Nothing below is specific to
`demi` except its contents.

On a target every root is a name in `PATH` — a symlink to the shell binary
(`shell.md`) — so real bash runs `demi …` and `myagent …` the same way it
runs anything else. In a hostless conversation tinybash accepts exactly
the manifest's roots as first words.

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

## Two execution paths

The same tool call runs one of two ways, decided by whether the
conversation has an execution target (`sessions-and-targets.md`). The model
sees the same commands, help and output shapes on both.

### On a real host (user host or managed host)

Real bash on the target parses the tool call. `demi` is an ordinary program
in `PATH`; everything else is whatever the machine has.

```
 tool call:  demi file edit src/a.ts <<'EOF' … EOF && npm test 2>&1 | tail -20

 backend                          runner on the target                 processes on the target
 ───────                          ────────────────────                 ───────────────────────
 job_start {script, cwd,   ────▶  spawn  bash -c "<script>"     ────▶  bash
            env + conv/shell ids}   │  tee stdout/stderr → artifact       │
                                    │  files under commandArtifactsDir     ├─ demi file edit src/a.ts        (shell, command mode)
                                    │                                      │    read ~/.demi/commands/<hash>/   manifest cache
                                    │                                      │    kind = runtime
                                    │                                      │    → run the module in-process, ctx.fs = real fs
                                    │                                      │    (zero wire bytes)
                                    │                                      │
                                    │                                      ├─ npm test 2>&1 | tail -20       (ordinary processes;
                                    │                                      │    the pipe is an OS pipe)
 ◀── job_output {view ≤ 1 MB} ──────┘                                      │
 ◀── job_exit {code, artifact paths} ◀── bash exits ◀──────────────────────┘

 an rpc command inside the same script, e.g.  demi todo add "run the suite":

                                                                        command mode: kind = rpc
                                  runner  ◀───────── UDS ─────────────  → parsed args + stdin
 ◀── rpc_call {conv id, shell id, ──┘  attributes by the ids in the
     root, command, args, stdin}       job's environment
     backend runs the command
     against conversation state
 ─── rpc_output / rpc_exit ─────▶ runner ──────────── UDS ────────────▶ command mode writes stdout, exits with the code
```

What crosses the wire: the script, the bounded view, the exit, and the
arguments and output of `rpc` commands. File contents and pipeline bytes
never do.

### Hostless (no execution target)

There is no bash and no runner. The backend runs the tool call in tinybash,
which accepts only root commands and runs them in-process.

```
 tool call:  demi file create notes.md <<'EOF' … EOF
             demi todo add "draft the outline"

 backend (one process; nothing leaves it)
 ────────────────────────────────────────
 tinybash ──▶ [ {argv, stdin}, {argv, stdin} ]                words, quotes, heredocs, `;` `&&` `||` newline
        │                                                     anything else → refused (below)
        ▼
 in-process loader  (tinybash's only "executables" are the manifest's roots)
   ├─ demi file create …   kind = runtime  → run the SAME module as on a real host,
   │                                          ctx.fs = store-backed Host
   │                                          → conversations/<id>.sqlite  (host_store)
   └─ demi todo add …      kind = rpc      → the in-process handler, no socket
        │
        ▼
 tool result = stdout / stderr / exit code, exactly as on a real host


 tool call:  npm test
 tinybash ──▶ first word is not a root command
                        ├─ managed hosts configured ──▶ backend provisions a managed host bound to
                        │                                the conversation, writes the hostless files
                        │                                into its home, injects the context block,
                        │                                runs the command on the real-host path above
                        └─ not configured ───────────▶ refused: "this conversation has no machine;
                                                        pair a device and move it there"
```

### Side by side

| | Real host | Hostless |
|---|---|---|
| Who runs the tool call | real bash on the target | tinybash in the backend |
| What can appear in it | anything bash runs | root commands, heredocs, `;` `&&` `\|\|` newline |
| Where a `runtime` module runs | in a command-mode shell process on the target | in the backend process |
| The `ctx.fs` it sees | the target's real filesystem (`host-shell`) | the conversation's store-backed Host (`host-virtual`) |
| Where an `rpc` command runs | in the backend, reached via UDS → runner socket | in the backend, called directly |
| Where files live | on the target | in `conversations/<id>.sqlite` |
| Where full output lives | artifact files on the target | the tool result itself (bounded, no tee needed) |
| Bytes on the wire | script, view, exit, rpc args/output | none |
| Leaving this path | user switches the target in the picker | first non-`demi` command auto-provisions |

## Command kinds

Every leaf is one of two kinds:

- **`rpc`** — the implementation runs in the backend. The command needs
  backend state: `todo` (`CommandStorage`), `agent` (the subagent
  supervisor), `host` (registry, grants, provisioner). An invocation on a
  target travels to the backend as a typed message carrying the parsed
  arguments and stdin; the result streams back as stdout, stderr and exit
  code.
- **`runtime`** — the implementation is an ES module shipped to wherever the
  command is invoked and run there against that place's filesystem. `demi
  file read/create/edit/patch` and future `demi search` are `runtime`. On a
  target the module runs inside the shell in command mode with zero round
  trips; in a
  hostless conversation it runs inside the backend against the
  conversation's store-backed Host.

The rule is mechanical: **a command that touches only the target's
filesystem is `runtime`; a command that touches conversation or platform
state is `rpc`.** There is no third kind and no per-target implementation.

## The command ABI

A `runtime` module has one export:

```ts
export default async function (ctx: CommandContext): Promise<CommandResult>
```

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

A module imports nothing from the runtime: no Node builtins, no Bun or
shell globals, no network. It sees the standard ECMAScript library plus the
Web-platform globals every embedder guarantees (`TextEncoder`,
`TextDecoder`, `URL`, `atob`, `btoa`, `crypto.randomUUID`, `AbortSignal`).
This is what makes one module run identically inside the shell against a
real filesystem, inside the backend against the store-backed Host, and
inside a test with an in-memory Host. It is the public contract a third
party builds on, versioned with the manifest.

Byte-heavy work inside a module goes through `ctx.fs` and the streams,
which every embedder implements natively; a module never loops over bytes
in JS (the shell has no JIT — `shell.md`).

## The manifest

The backend builds a manifest from the assembled tree at start-up and
whenever the tree changes:

```
manifest
  hash                       content hash of everything below
  roots[name]                one tree per root command (`demi`, `myagent`, …):
    tree                     groups and leaves: name, kind, help, input and output schemas (JSON Schema)
  modules[hash]              one bundled ESM file per runtime leaf, referenced from its leaf
```

Each `runtime` leaf is bundled at build time into one self-contained module
(its source plus whatever it imports, tree-shaken); a small command is
simply a small bundle. The manifest is served by the backend to embedders
over their existing connection (the runner socket) and by HTTP for the
standalone case; modules are fetched by hash and cached forever — a new
tree is a new manifest hash, never a mutated module.

## The loader

`@demicodes/command-loader` is a pure-JS library with no runtime
dependency. It is the one place that knows how to run a command:

```
loader = createLoader({ source, host, rpc?, cache? })
loader.dispatch(root, argv, io) → exit code
```

- `source` yields the manifest and modules — from the runner's connection,
  from a directory, from a URL.
- `host` is the Host the `runtime` modules run against.
- `rpc`, when present, carries typed `rpc` invocations; an embedder without
  one serves only `runtime` commands.
- `cache` persists manifests and modules by hash (a directory on a target,
  memory in the backend).

Dispatch selects the root's tree, resolves the path through it, prints help
for a group,
parses and validates the leaf's arguments against its schema, then either
runs the cached module with a `ctx` built from `host`, `io` and the
arguments, or sends the `rpc` message. Help text comes from the tree, so
`demi file --help` is identical on every surface.

Embedders:

| Embedder | Source | Host | rpc |
|---|---|---|---|
| backend, hostless conversation | in-process tree | the conversation's store-backed Host | in-process |
| runner | the backend socket, cached on disk under `~/.demi/commands/<hash>/` | — (the runner does not execute commands; it caches and relays) | the backend socket |
| the shell in command mode on a target | the runner's disk cache; a miss asks the runner over the UDS | `@demicodes/host-shell` over the real filesystem | the runner over the UDS |
| the shell in command mode, standalone (no runner) | a configured directory or URL | the real filesystem | none, or an embedder-supplied transport |
| tests | in-memory | in-memory Host | stub |

A third party who wants Demi's commands in another agent needs the loader,
a Host implementation and a manifest source — no runner, no shell, no
backend.

## Root commands on a target

A root command on a target is the shell binary in command mode
(`shell.md`) running the loader, reached through a symlink named after the
root: `argv[0]` selects the root's tree in the manifest. Real bash spawns it
like any other program; it reads the manifest cache the runner maintains,
runs `runtime` commands in its own process, and forwards `rpc` commands to
the runner over the local UDS. The process holds no credential: the runner
attributes an `rpc` call to a conversation by the ids the backend injected
into the bash environment at spawn time and forwards it on its
authenticated socket (`runner.md`). The runner creates and removes the
symlinks as the manifest's root set changes.

Stdin and stdout are byte-faithful in both kinds: a `runtime` module reads
and writes its process streams; an `rpc` invocation streams stdin to the
backend and stdout/stderr back.

## Hostless execution

A conversation with no execution target (`sessions-and-targets.md`) still
executes root commands. The backend runs the tool call in **tinybash**
(`@demicodes/tinybash`): a deliberately tiny shell — parser plus executor —
whose only executables are the manifest's root commands, dispatched through
an in-process loader whose Host is the conversation's store-backed
filesystem (`@demicodes/host-virtual`). It is the in-process counterpart of
real bash on a target: same tool, same commands, a fraction of the grammar.

tinybash accepts exactly:

- one simple command per statement — words with single quotes, double
  quotes and backslash escapes; `#` comments;
- heredocs: `<<EOF`, `<<'EOF'`, `<<-EOF`, and here-strings `<<<`; `demi
  file create` and `demi file patch` take their content on stdin and the
  agent writes them this way;
- statements joined by newline, `;`, `&&` or `||`, with bash's exit-status
  semantics for the chains; the tool result's exit code is the last
  statement's.

Anything else is refused with a message naming the way out: pipes,
redirections, variables, command and process substitution, globs,
background jobs, and any first word that is not a manifest root. tinybash
does not pretend to be bash, so there is no divergence catalogue — its
grammar is the list above. The tool description in the hostless state says
which commands exist and that any other command starts a machine; the
model reaches for `demi file` rather than `cat`.

The first command whose first word is not a root auto-provisions a managed
host bound to the conversation, and the backend writes the hostless files
into its home (`sessions-and-targets.md`).

## The `demi host` group

Contributed by the backend composition root because it needs the registry,
the grant table and the provisioner:

```
demi host                                        help for the group
demi host list                                   granted hosts: id, kind, online; the current one marked
demi host current                                the current execution target
demi host shell --id <hostId> <shell_content>    run a shell string in that host's real bash; byte-faithful stdio
```

`<shell_content>` is executed by the remote host's `bash -c`, so pipes,
redirections and globs apply remotely; it starts in that host's default
cwd. The backend checks the conversation's grant set before dispatching
(`sessions-and-targets.md`). A cross-host byte transfer such as `demi host
shell --id A "tar cz -C /work ." | tar xz` is brokered by the backend as an
HTTP stream between the two runners (`runner.md`), never over the runner
sockets.

## Packages

- `@demicodes/shell` keeps the `Command` types (tree, input/output specs,
  `CommandContext`, `CommandResult`) and the Host contract; it loses the
  interpreter and the portable command set in M9.
- `@demicodes/coding-agent` declares the `demi` root's agent-facing groups
  with the `kind` on each leaf; the `runtime` leaves' implementations are
  written against the ABI. A library user declares their own root the same
  way, with the same types.
- `@demicodes/command-loader` is new: the loader, the manifest types and
  build.
- `@demicodes/tinybash` is new: the hostless shell — parser and executor
  over a loader, no Host of its own. Usable by any embedder that wants
  hostless execution, not only the backend.
- `@demicodes/backend` assembles the roots, builds and serves the
  manifest, and embeds the loader for hostless conversations.
