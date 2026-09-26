# Command declarations and execution

An agent supplies one command tree. Groups organize subcommands; leaves select an
application callback (`rpc`) or a native package operation (`native`). The tree
defines command names, help, argument schemas, and bindings for both embedded
shell calls and external command clients.

The `demi browser` command family, including readable output, optional
JSON, image bytes, targeting, and examples, is specified in
[Browser automation](browser.md#command-contract). It uses this command contract
rather than a separate shell or model tool loop. The `demi host expose`
group is specified in [Host expose](expose.md#commands).

## Declare a command

For example, this native leaf reads a file on the execution target:

```ts
const readFile = {
  name: 'read',
  summary: 'Read a file on the execution target',
  kind: 'native',
  binding: { package: 'demi.builtin', operation: 'file.read' },
  input: { path: z.string() },
  positionals: ['path'],
} satisfies Command
```

Placed under the `demi file` groups, the leaf handles `demi file read notes.txt`.
The dispatcher validates `{ path: 'notes.txt' }` before invoking `file.read`.
The native executable receives parsed arguments and the invocation's cwd and IO.

An RPC leaf instead provides an application function:

```ts
const location = {
  name: 'location',
  summary: 'Print the invoking shell directory',
  kind: 'rpc',
  async run(ctx) {
    await ctx.io.stdout(`${ctx.cwd}\n`);
    return { exitCode: 0 };
  },
} satisfies Command
```

The callback receives the invoking Host, parsed input, environment, cancellation,
byte IO, and command storage bound to its agent node. The embedding application
owns this function; an SDK application need not use the Demi backend.

The public types belong to `packages/shell/src/command.ts`. A group contains
subcommands and does not execute a handler. Roots and sibling names must be
unique. Registration rejects reserved root names and invalid command trees.

A leaf written inside a `Command[]` literal is typed by the tree, so its
`parsed.values` carry no field types. Declaring it through `defineCommand`
keeps its own input schemas: `parsed.values.path` is then a `string` the
handler uses directly, and the leaf still belongs in the same tree.

## Parse input and render help

Each input field has one source:

| Declaration | Source and behavior |
| --- | --- |
| `input` | Zod field schemas used to validate values. |
| `positionals` | Ordered fields supplied as positional arguments. |
| `stdinField` | A string field populated from finite stdin, not an argv option. |
| `restField` | An array receiving raw tokens after `--`. |
| Remaining input fields | Named options such as `--path notes.txt`. |
| `output.json` | A schema enabling validated structured output through `--json`. |

Unknown options, unknown fields, missing required values, duplicate scalar
values, and schema failures reject execution, and one rejection names every
field that failed. Without `restField`, `--` ends option parsing.

Conversion belongs to the CLI alone. An argv token is text, so the parser turns
it into the number, boolean, or array element its field declares — each element
of a repeated option separately — and then validates the whole input at once. An
external client sends arguments as decoded JSON, which are validated as they
arrive: `"7"` for a numeric field is a usage error there, not a 7.

An input field may declare `string`, `number` (`.int()` included), `boolean`,
`enum`, or an array of those, optionally wrapped in `.optional()`, with
`.describe()` supplying its help text. Registration rejects anything else,
naming the field. The reason is that a tree reaches an external runner as JSON
Schema, and only the subset means the same thing on both sides: `.nullable()`
returns from that trip as a plain union the argv parser cannot spell,
`.default()` returns as a bare optional whose value nobody fills in, and
`.refine()`, `.transform()` and `.pipe()` vanish, leaving the runner accepting
what the declaring process rejects. A field that needs a value when the caller
omits one, or a rule the subset cannot state, belongs in the leaf's own `run`.

A group-only invocation or `--help` prints help and exits successfully without
running a handler or reading stdin. For example, `demi file read --help` must work
even if its input is an idle terminal. Help comes from the declaration, so adding
an operation does not require a second manually maintained help definition.

## Dispatch the same declaration on each surface

The arrows below show calls, not process containment. Both entry paths converge
before the dispatcher selects the execution destination.

```text
Shell builtin -------- direct call ------> Dispatcher
External client ------ local forwarding -> Dispatcher

Dispatcher -------- native operation ----> Target command service
Dispatcher -------- application RPC -----> Embedding application
```

The execution adapter creates a validated manifest from the command tree and
startup package catalog. Each job pins that manifest and receives a live context.
On a live runner connection, send the manifest before the first job that uses it
and whenever the selected manifest hash changes. Consecutive jobs with the same
hash use the already validated immutable manifest. Switching A → B → A sends
all three selections; existing jobs retain their own pinned manifest. A new
connection sends its first manifest again, even when its hash matches the
previous connection. Ordered delivery installs a manifest before its job; this
avoids revalidating the same command schemas before every shell invocation.
A failed send makes the selection uncertain; the next job sends its manifest
again, even if it was selected before the failed send.

External aliases forward raw argv and context to the runner. The runner validates
the context, resolves the command, parses input, and dispatches the selected leaf.
A native package never receives an unvalidated CLI request.

`command-loader` owns manifest serialization and TypeScript loading. SDK embedders
use `createLoader` with explicit RPC and native executors. The Go runner uses
generated manifest types and its own dispatcher, with shared CLI fixtures to
check argument behavior. The TypeScript loader does not manage processes or
object-store credentials.

Native bindings pin an exact descriptor hash and operation. Missing operations,
inconsistent hashes, and unknown packages prevent a usable manifest.
[Native execution](native-runtime.md#bind-an-exact-package) owns catalog
composition, immutable releases, installation, and service lifetime.

## External command clients

An external program such as a Python script calls a declared root through an alias to
`demi-runner`. The alias basename selects the root. The client forwards raw argv,
cwd, environment, and its live execution context, then streams command IO. It
contains no native command algorithms. Shell builtins and in-process utilities
(`xargs`, `find -exec`, `env`) call the dispatcher directly and do not need this
extra process or connection.

The runner injects the endpoint and the job's opaque context handle into its
jobs. The handle leads the runner to the job's live execution context, and with
it to the [command context](native-runtime.md#command-context). The runner
accepts local connections only from the current account:

| Platform | Local transport |
| --- | --- |
| Linux and macOS | Owner-restricted Unix domain socket |
| Windows | Account-restricted byte-mode named pipe |

Each client opens one HTTP/2 connection and one invocation stream. The runner
accepts every local connection ([Load](runner.md#load)). When it cannot accept
one yet, because its queue of waiting connections is full or it has no open
file to spare, the client waits and connects again for as long as the runner
runs. A refused connection looks the same whether the runner is busy or gone,
so the runner holds a lock on a file beside its socket while it runs; the
system releases it when the runner exits, even by crashing. A refused client
checks the lock and waits while it is held, and fails at once otherwise.
Raw CLI metadata has its own schema; framing, input demand, and completion use
the [native protocol](native-runtime.md#invocation-protocol). The runner authenticates
the context before dispatching. Connection processing continues through blocked
output, and client loss cancels its invocation. Stdin EOF alone is not cancellation.

Aliases use the runner release selected for their backend registration. That
registration's authenticated local management endpoint provides status and drain.
Unix and PowerShell installers verify release metadata, size, and SHA-256 before
publishing the executable, with installation locks protecting upgrades. The
[runner design](runner.md#connection-and-identity) owns registration lifetime;
[builds and releases](../native-builds.md) owns release production and checks.

## Deliver IO and release an invocation

Finite input and interactive input have different lifetimes:

- `stdinField` consumes finite text into its argument. The callback's `stdin`
  is then `null`.
- Without `stdinField`, callbacks receive finite bytes through `stdin` when a
  pipe is present.
- `stdinStream` supplies subsequent interactive writes until the shell job ends.

Input is demand-driven. A command that never requests input must not consume it
because transport capacity is available. Native and forwarded calls use the
[input-demand protocol](native-runtime.md#request-body-and-input-demand).

Stdout and stderr remain separate byte streams. Successful `--json` execution
validates captured stdout before releasing it. Normal completion carries an exit
status; transport loss is not successful command completion.

A declared `runningHint` replaces the generic model-facing running hint while the
command is active. Dispatch releases the hint and invocation resources on success,
failure, and cancellation. Ordinary native cancellation preserves other calls;
a faulty handler that cannot stop follows the
[service failure contract](native-runtime.md#invoke-and-retire-a-service).

## Implementation discrepancy

The Go dispatcher checks help before consuming a command body. The TypeScript
`runRegisteredCommand` entry point currently consumes finite stdin for a leaf with
`stdinField` before checking its parsed help flag. `createLoader` uses that entry
point too. A local fixture calling a `stdinField` leaf with `--help` observed one
stdin read, zero handler calls, and exit code 0.

This is a code deviation from the help contract above. The TypeScript entry point
must check help before reading the body. The discrepancy is recorded here rather
than changing the intended behavior; this documentation checkpoint does not fix
that implementation.
