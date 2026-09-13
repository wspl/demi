# Command declarations and execution

An agent harness supplies one command tree. Groups contain subcommands; leaves
select either an application callback (`rpc`) or a native package operation
(`native`). `packages/shell/src/command.ts` owns this public contract. Command
names, help, argument schemas and bindings are defined once in that tree.

## Declaration

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

`input` contains Zod field schemas. `positionals` names ordered positional fields;
`stdinField` names a text argument read from finite stdin; `restField` receives raw
arguments after `--`. Optional output schemas describe structured JSON output.
A group-only invocation or `--help` renders help without executing a handler or
reading input. A leaf may supply a running hint used while its callback is active.

RPC leaves carry `run(ctx)`. The callback receives parsed arguments, cwd, the
invoking Host, cancellation, byte IO and command storage bound to the invoking
agent node. The application defines callbacks; they are not inherently tied to
the Demi product backend. Native leaves carry no JavaScript implementation.

## Manifests and initialization

`command-loader` serializes declarations into a validated manifest containing
roots and immutable native package descriptors. Each native binding pins its
package descriptor hash and operation. Package IDs and child names must be unique;
unknown bindings, missing operations and inconsistent hashes are rejected.
The manifest contains no source text, import URL or downloadable JavaScript.

`createRemoteShellEnvironmentFactory({ packages, resolveArtifact })` belongs to
`host-remote`. Application assembly injects that factory into `AgentServer` as its
shell environment. Its structural context supplies Host, command registry, shell
options and command storage. The factory validates bindings before starting jobs.
Neither core agent initialization nor model/provider settings select storage
vendors or native implementation packages.

`demiPackage` is the descriptor exported by `@demicodes/demi-package`. Its resident
`demi-commands` executable implements Demi’s native file read/create/edit/patch
operations. Additional packages implement the same independent service contract.
All native Demi implementations belong in this package; standard shell utilities
belong in `native-utils`, and application-state callbacks remain with their owners.

## Dispatch

The backend sends a manifest and each job pins its hash. The job receives command
aliases pointing at its runner, an exact local endpoint and an opaque context.
The client forwards root/argv; the runner authenticates context, parses the command,
validates input and dispatches the selected binding. Native operations execute
beside their files in a resident service. RPC operations travel to the embedding
application over the authenticated runner connection and byte-pipe transport.

The same TypeScript declarations serve SDK embedders through `createLoader` with
injected RPC and native executors. The Rust runner uses generated manifest types
and its own dispatcher, with shared fixtures for argument behavior. The loader
does not spawn processes or resolve object-store credentials.

## Input, output and cancellation

Finite pipe input and live shell input have separate semantics. `stdinField`
consumes finite text into an argument; ordinary callbacks can receive finite bytes
through `stdin` and later interactive writes through `stdinStream`. Input demand
crosses both local and native HTTP/2 streams explicitly. A command that never reads
input does not consume it merely because a receive window is available.

Logical stdout/stderr remain distinct byte streams until the shell tool formats
its bounded combined preview. Normal completion carries an exit status; transport
loss or missing completion is a failure. Cancellation stops the invocation and
releases its input, output, callback resources and running hint. One cancelled
native invocation does not terminate unrelated calls in the shared service.

File algorithms live in `demi-package/src`, not the runner or command loader.
They receive per-call cwd and cancellation, and do not change global process
state. Native package installation, flow-control limits, service retirement and
six-target release rules are defined in [native-runtime.md](native-runtime.md).
