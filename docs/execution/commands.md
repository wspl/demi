# Command declarations and execution

An agent supplies one command tree. Groups organize subcommands; leaves select a
handler that runs in the backend (`rpc`) or a native package operation
(`native`). The tree defines command names, help, argument schemas, and bindings
for both embedded shell calls and external command clients.

The `demi browser` command family, including readable output, optional
JSON, image bytes, targeting, and examples, is specified in
[Browser automation](../browser/browser.md#command-contract). It uses this
command contract rather than a separate shell or model tool loop. The
`demi host expose` group is specified in [Host expose](expose.md#commands), and
the `demi agent` group in [Subagents](../agent/subagents.md).

## Declare a command

For example, `demi file read notes.txt` reads a file on the execution target.
Its arguments are one Rust type:

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileRead {
    /// File path to read
    pub path: String,
}
```

The leaf declares its name, summary, input sources, and binding around the JSON
Schema that schemars derives from this type. Because the schema is derived, the
schema a runner enforces and the type a handler decodes are one definition. A
declaration is data, and it serializes as the manifest node a runner receives;
the manifest adds only the hash of the release descriptor each native binding
pins ([Dispatch](#dispatch-the-same-declaration-on-each-surface)):

```json
{
  "name": "read",
  "summary": "Read a file on the execution target",
  "kind": "native",
  "binding": { "package": "demi.builtin", "operation": "file.read" },
  "input": {
    "title": "FileRead",
    "type": "object",
    "properties": {
      "path": { "description": "File path to read", "type": "string" }
    },
    "required": ["path"],
    "additionalProperties": false
  },
  "positionals": ["path"]
}
```

Placed under the `demi file` group, the leaf handles `demi file read notes.txt`.
The dispatcher validates `{ "path": "notes.txt" }` against `input` before
invoking `file.read`, and the native service decodes the same value into
`FileRead`, together with the invocation's cwd and IO. The arguments and
results of every `demi.builtin` operation are types of one contract crate that
the declarations and the native programs both link, so a declaration and its
handler cannot describe different arguments
([Contract crates](../architecture/contracts.md#contract-crates)).

An `rpc` leaf runs a handler in the backend instead.
`demi todo add "Write tests"` declares its arguments the same way:

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// Todo text
    text: String,
}
```

Its leaf has `kind: "rpc"`, `positionals: ["text"]`, and an `output.json`
schema derived from the type it prints with `--json`. Its handler receives the
validated arguments as an `AddArgs`, adds the todo to the invoking agent node's
command storage, and prints the new todo ([Handle an rpc call](#handle-an-rpc-call)).

Declarations are separate from handlers. The command set pairs the declaration
tree with one handler per `rpc` leaf, and everything that reads the tree sees
only the declarations: the manifest a runner receives and the help the model
reads. Building a manifest therefore needs no handler, and a runner never holds
one. Registration refuses an `rpc` leaf without a handler.

A group contains subcommands and does not execute a handler. A command name
starts with an ASCII letter or digit and continues with letters, digits,
underscores, and hyphens; roots and sibling names must be unique. Registration
also rejects reserved root names, such as shell words and standard tools, and
these declarations:

- a group with no subcommands;
- an input source that names a field the input does not have, overlapping
  input sources, duplicate positional fields, and a required positional after
  an optional one;
- an option named `help` or `json`;
- a `stdinField` that is not a string, or a `restField` that is not an array of
  strings;
- a field schema outside the [input subset](#the-input-subset).

A rejection names the command and, when a field is at fault, the field. It
happens when the command set is built, before any call.

## Parse input and render help

One implementation of everything this section describes, the command-tree
library ([Crates](../architecture/crates-and-packages.md#crates)), serves both
the runner and the backend. The runner parses argv and answers `--help` with it;
the backend renders the model's command help and checks registrations with it.
Both validate arguments with the `jsonschema` validator over the schema
schemars derived, so a usage error reads the same wherever it is raised, and no
second set of rules exists to drift from the declaration.

Each input field has one source:

| Declaration | Source and behavior |
| --- | --- |
| `input` | The JSON Schema of the leaf's argument type. It validates the whole input. |
| `positionals` | Ordered fields supplied as positional arguments. They have no named-option form. |
| `stdinField` | A string field populated from finite stdin: a quoted heredoc, a pipe, or input redirection. It has no option or positional form. |
| `restField` | An array receiving raw tokens after `--`. It has no named-option form. |
| Remaining input fields | Named options such as `--path notes.txt`. Their schemas define values, optionality, boolean flags, enums, and repeated array options. |
| `output.json` | A schema enabling validated structured output through `--json`. |

Unknown options, unknown fields, missing required values, duplicate scalar
values, and schema failures reject execution, and one rejection names every
field that failed. The parser reports a missing option value before it
consumes the next option. `--name=value` supplies an option value that begins
with `--`. Without `restField`, a standalone `--` ends option parsing and the
tokens after it are positionals; with `restField`, they fill that field. A body
option such as `--content` is rejected with a diagnostic directing the caller to
remove it and use stdin. A finite stdin body is at most 1 MiB.

Conversion belongs to the CLI alone. An argv token is text, so the parser turns
it into the number, boolean, or array element its field declares, each element
of a repeated option separately, and then validates the whole input at once.
Arguments that arrive as JSON, such as an `rpc` call's arguments at the backend
or the arguments of a one-shot user call, are validated as they arrive: `"7"`
for a numeric field is a usage error there, not a 7.

A group-only invocation or `--help` prints help and exits successfully without
running a handler or reading stdin. For example, `demi file read --help` must
work even if its input is an idle terminal. Help comes from the declaration, so
adding an operation does not require a second manually maintained help
definition.

### The input subset

An input field is a string, a number, an integer, a boolean, a string enum, or
an array of one of those, and any field may be optional. A string may carry
length and pattern bounds, a number or integer a range, and an array bounds on
its item count. A field's doc comment becomes its description in the schema
and in help.

In the derived JSON Schema, the leaf's input is an object that allows no other
properties, and a property uses only `type`, `enum`, `items`, `description`,
`format` for numbers, `minLength`, `maxLength`, `pattern`, `minimum`,
`maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `minItems`, and `maxItems`.
An optional field is an `Option` in the argument type: its schema leaves it out
of `required` and never allows `null`. Registration rejects anything else and
names the field: `default`, `null` in a type, `oneOf`, `anyOf` or `allOf`,
`$ref`, nested objects, and arrays of arrays.

The reason is that the argument type reaches the runner as the JSON Schema
schemars derives from it, and the runner enforces exactly that schema: a rule
the schema cannot state is a rule the runner cannot check. Bounds survive the
trip, so the runner enforces them as the backend does. A default does not:
validation never fills in a missing value, so a field that needs a value when
the caller omits one takes it in the handler. A transform, such as trimming or
normalizing, has no schema form either and also belongs to the handler. `null`
has no argv spelling, and nested objects and unions have no argv syntax the
parser could fill. The argument type decodes with serde's derived rules only,
so every value the schema accepts decodes; a handler that cannot decode an
accepted value has a declaration bug, not a usage error.

A length bound counts Unicode scalar values, the rule JSON Schema gives
`minLength` and `maxLength`, at both ends. For example, `𝄞` is one scalar value
and two UTF-16 units, so it passes `maxLength: 1` as a command argument. Strings
the browser validates count the same unit
([Validation at entry](../architecture/contracts.md#validation-at-entry)).

### Help

The model's command help and every `--help` come from one renderer over the
declarations. The model's help opens with one paragraph of defaults that every
command follows unless its own help says otherwise: success prints raw text on
stdout; failure writes an error message to stderr and exits non-zero; `--help`
works at any level; usage writes `<placeholders>` for values and `[brackets]`
for optional arguments; values containing spaces are quoted; stdin bodies use a
quoted heredoc, pipe, or input redirection and have no option; and
`--name=value` and `--` pass values that begin with `--`. Each root's help
follows. An empty command set renders no help.

Help displays a complete usage template, value placeholders, required and
optional arguments, enum choices, and repeatable options. Stdin bodies appear
in their own section and in a quoted heredoc template. `--json` appears only
on commands with a JSON output schema. Usage templates are generated from the
declaration; declarations do not carry separate examples to keep in sync.

For example, the generated structure for file creation is:

```sh
demi file create <path> <<'EOF'
<content>
EOF
```

Angle brackets identify placeholders; square brackets identify optional
arguments. The agent substitutes actual values and quotes shell arguments.

### Demi command inputs

| Command | Positional input | Named options | Stdin |
| --- | --- | --- | --- |
| `file read` | path | none | unused |
| `file create` | path | none | file content |
| `file edit` | path | old, new, occurrence, context | unused |
| `file patch` | none | none | unified diff |
| `todo add` | text | JSON output | unused |
| `todo update` | id | text, status, JSON output | unused |
| `todo done` | id | JSON output | unused |
| `todo list` | none | JSON output | unused |
| `agent spawn` | none | profile, description, no-subagents, JSON output | task brief |
| `agent send`, `resume` | id | JSON output | message |
| `agent abort`, `show` | id | JSON output | unused |
| `agent list` | none | JSON output | unused |
| `host shell` | one quoted script | host | streamed to the remote program |
| `host list`, `current` | none | none | unused |

`file edit` has two separate text operands; its old and new values are quoted
option arguments. Use `file patch` for a multiline change supplied as one
heredoc. `host shell` takes the script as one quoted argument so its stdin
remains available for the remote program's data. These commands do not claim
that stdin supplies their script or edit operands.

## Dispatch the same declaration on each surface

The arrows below show calls, not process containment. Both entry paths converge
before the dispatcher selects the execution destination.

```text
Brush builtin -------- direct call ------> Dispatcher (runner)
External client ------ local forwarding -> Dispatcher (runner)

Dispatcher -------- native operation ----> Command service on the same Host
Dispatcher -------- rpc_call ------------> Backend: the node's command set
```

The backend's execution adapter builds a manifest from the command set's
declarations and the startup package catalog. Each native binding gains the
hash of the release descriptor it pins, and the manifest carries those
descriptors and its own hash, the SHA-256 of its canonical JSON. Each job pins
that manifest and receives a live context. On a live runner connection, send the
manifest before the first job that uses it and whenever the selected manifest
hash changes. Consecutive jobs with the same hash use the already validated
immutable manifest. Switching A → B → A sends all three selections; existing
jobs retain their own pinned manifest. A new connection sends its first manifest
again, even when its hash matches the previous connection. Ordered delivery
installs a manifest before its job; this avoids revalidating the same command
schemas before every shell invocation. The runner keeps one installed manifest,
so a manifest and the job after it travel together: jobs that start at once on
one connection take turns. A failed send makes the selection
uncertain; the next job sends its manifest again, even if it was selected before
the failed send.

The manifest's envelope belongs to the runner wire's contract crate and its
nodes are the declarations themselves, so the adapter that builds a manifest
and the runner that checks it share one definition and one hash. Before
installing a manifest, the runner verifies that every descriptor matches its
hash, every native binding names an operation of its pinned descriptor, the
tree is at most 32 levels deep, and the manifest matches its own hash.

External aliases forward raw argv and context to the runner. The runner validates
the context, resolves the command, parses input, and dispatches the selected leaf.
A native package never receives an unvalidated CLI request. An `rpc` leaf
reaches the backend as an `rpc_call` that names only its job: the backend checks
the call against its live record of that job
([Bind jobs to their caller](sessions-and-targets.md#bind-jobs-to-their-caller)),
validates the arguments again, and dispatches the call in process to the
invoking node's command set.

Native bindings pin an exact descriptor hash and operation. Missing operations,
inconsistent hashes, and unknown packages prevent a usable manifest.
[Native execution](native-runtime.md#bind-an-exact-package) owns catalog
composition, immutable releases, installation, and service lifetime.

## Handle an rpc call

An `rpc` handler receives its call as data and acts only through messages;
[The TypeScript boundary](../architecture/contracts.md#the-typescript-boundary)
says why. For example, `demi todo add "Write tests"` in a job reaches the
backend as an `rpc_call`. The backend builds an invocation from the call and its
record of the job, and gives the handler that invocation and a port.

The invocation carries:

| Field | Meaning |
| --- | --- |
| Path and argv | The leaf's path from its root, and the raw argv. |
| Arguments | The input, validated against the leaf's schema. A `stdinField` body is already among them. |
| `json` | Whether the caller passed `--json`. |
| cwd and environment | The invoking shell's directory and environment. |
| Command context | The [command context](native-runtime.md#command-context) from the backend's record of the job: conversation, caller, and locale. |
| Storage binding | Whose command storage the job reaches: its agent node and that node's generation, from the same record. A job the handler starts on another Host carries it on. |
| Stdin | Whether the calling process has a pipe on its stdin. |
| Relayed pipes | The ids of the pipes relayed for the call's stdin and stdout. |

The port is the handler's side of the call. Each of its operations is one
request and one reply:

| Operation | Meaning |
| --- | --- |
| Write stdout, write stderr | Output. Stdout flows through the call's relayed pipe, so a write waits until the calling process has read enough; stderr goes to the runner as messages ([Deliver IO](#deliver-io-and-release-an-invocation)). |
| Read stdin | The next chunk of a finite stdin, on demand. |
| Read live stdin | The next interactive write to the job, until the job ends. |
| Storage | Read, list, and conditional write of the invoking agent node's command storage. |
| Cancellation | Whether, and when, the call was cancelled. |

A handler resolves what it acts on from the invocation's context: the
`demi agent` handlers find their conversation's tree and node there, and
`demi host shell` finds its conversation. The port offers no Host operations. A
backend handler that must reach a Host goes through the conversation's host
access with the conversation the context names
([Host operations](sessions-and-targets.md#host-operations)). The relayed pipe
ids let it hand the call's stdin and stdout to a job it starts on another
device, so those bytes flow between the two devices through the backend's pipes
without passing through the handler.

Command storage changes by versioned compare-and-set: a write names the
revision it read and fails when another write came first, and the handler then
reads again, so two `demi todo add` calls at once keep both todos.
[Command state history](../agent/command-state-history.md#mutation-api-and-concurrency)
defines the storage messages, their revisions, and the versions they create.

A handler ends with an exit status. A handler error writes `<root>: <message>`
to stderr and exits 1. A call the runner cancels ends with 130. A call stopped
for another reason, such as its job exiting, a relayed pipe failing, or the
backend shutting down, ends with 1 and its cause on stderr. When several causes
stop a call, the first is the one reported.

## External command clients

An external program such as `xargs` calls a declared root through an alias to
`demi-runner`. The alias basename selects the root. The client forwards raw argv,
cwd, environment, and its live execution context, then streams command IO. It
contains no native command algorithms. Brush builtins call the dispatcher directly
and do not need this extra process or connection.

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
Raw CLI metadata has its own type; framing, input demand, and completion use
the [native protocol](native-runtime.md#invocation-protocol). The runner authenticates
the context before dispatching. Connection processing continues through blocked
output, and client loss cancels its invocation. Stdin EOF alone is not cancellation.

Aliases use the runner release selected for their backend registration. That
registration's authenticated local management endpoint provides status and drain.
Unix and PowerShell installers verify release metadata, size, and SHA-256 before
publishing the executable, with installation locks protecting upgrades. The
[runner design](runner.md#connection-and-identity) owns registration lifetime;
[builds and releases](../delivery/builds-and-releases.md) owns release production
and checks.

## Deliver IO and release an invocation

Finite input and interactive input have different lifetimes:

- `stdinField` consumes finite text into its argument. The handler then has no
  stdin to read.
- Without `stdinField`, a handler reads finite bytes from stdin when a pipe is
  present.
- Live stdin supplies subsequent interactive writes until the shell job ends.

Input is demand-driven. A command that never requests input must not consume it
because transport capacity is available. Native and forwarded calls use the
[input-demand protocol](native-runtime.md#request-body-and-input-demand). An
`rpc` handler asks for each live chunk, and a chunk it did not ask for, or one
over 64 KiB, stops the call.

Stdout and stderr remain separate byte streams. With `--json`, the dispatcher
captures stdout, up to 1 MiB, and on success validates it against the leaf's
output schema before releasing it. Output that is too large, is not JSON, or
does not match fails the command instead, with nothing on stdout, and a command
that fails releases no captured output. Normal completion carries an exit
status; an `rpc` call's exit status follows the drain of its stdout, so the
calling process has read everything before the call exits. Transport loss is not
successful command completion.

The runner holds at most eight of an `rpc` call's messages from the backend
that it has not handled yet, such as stderr the calling process has not read;
one more cancels the call rather than holding up the connection.

A declared `runningHint` replaces the generic model-facing running hint while the
command is active. Dispatch releases the hint and invocation resources on success,
failure, and cancellation. Ordinary native cancellation preserves other calls;
a faulty handler that cannot stop follows the
[service failure contract](native-runtime.md#invoke-and-retire-a-service).

## File commands

`demi file read`, `create`, `edit`, and `patch` run in the native `demi.builtin`
service, beside the file. Each resolves relative paths against the invocation's
cwd and stops at the invocation's cancellation. Mutations run one at a time in a
service: one mutation's planning and writes finish before the next begins. Each
replaced file is published atomically, from a temporary file beside it, and a
patch that changes several files restores the files it already changed when a
later write fails. Create, edit, and patch record their writes for
[edit tracking](edit-tracking.md).

## Acceptance

Verify with a real runner and without calling a model:

| Situation | Required result |
| --- | --- |
| `--help` on a `stdinField` leaf whose stdin is an idle terminal | Help prints and the command exits 0; stdin is not read and no handler runs |
| A declaration with a `default`, a nullable field, or a nested object | Registration fails and names the field |
| `𝄞` for a string argument with `maxLength: 1` | Accepted by the runner and by the backend |
| `"7"` for an integer field in an `rpc` call's JSON arguments | A usage error; the handler does not run |
| `--json` output that does not match the output schema | The command fails with nothing on stdout |
| An `rpc` call cancelled while its handler runs | Exit status 130, the cause on stderr |
