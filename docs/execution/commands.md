# Command declarations and execution

Each agent node has one command tree. Groups organize subcommands; leaves
select a handler that runs in the backend (`rpc`) or a command package
operation (`native`). The tree defines command names, help, argument schemas,
and bindings for both embedded shell calls and external command clients. The
plugins declare most of it, the product adds `demi host`, and the agent runtime
grafts `demi agent` and `demi shell` per node
([Commands](../architecture/plugins.md#commands)).

The `demi browser` command family, including readable output, optional
JSON, image bytes, targeting, and examples, is specified in
[Browser automation](../browser/browser.md#command-contract). It uses this
command contract rather than a separate shell or model tool loop. The
`demi expose` group is specified in [Host expose](expose.md#commands),
the `demi agent` group in [Subagents](../agent/subagents.md), and the
`demi shell` group in [The whole output](../agent/runtime.md#the-whole-output).

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
  "binding": { "package": "demi.file", "operation": "file.read" },
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
results of every `demi.file` operation are types of one contract crate,
`command-package-file-protocol`, that the declarations and the native program both link, so a
declaration and its handler cannot describe different arguments
([Contract crates](../architecture/contracts.md#contract-crates)).

An `rpc` leaf runs a handler in the backend instead.
`demi expose add 3000` declares its arguments the same way:

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// host:port, or a bare port meaning 127.0.0.1
    address: String,
    /// Host name or device id from demi host list; the primary host by default
    host: Option<String>,
}
```

Its leaf has `kind: "rpc"`, `positionals: ["address"]`, and an `output.json`
schema derived from the type it prints with `--json`. Its handler receives the
validated arguments as an `AddArgs`, creates the expose, and prints its URL
([Handle an rpc call](#handle-an-rpc-call)).

Declarations are separate from handlers. The command set pairs the declaration
tree with one handler per `rpc` leaf, and everything that reads the tree sees
only the declarations: the manifest a runner receives and the help the model
reads. Building a manifest therefore needs no handler, and a runner never holds
one. Registration refuses an `rpc` leaf without a handler.

An `rpc` leaf whose handler acts on Demi itself, beyond the conversation,
names the [permission category](../agent/permissions.md#categories) it needs
as `permission`, such as `skills.manage` for `demi skills add`, and one of its
groups declares that category with its `action` and `description`. The
declaration is all a command does about permissions: the backend's dispatch
checks the conversation's grant before the handler runs
([The check](../agent/permissions.md#the-check)). The runner ignores both
fields.

A group contains subcommands and does not execute a handler. A command name
starts with an ASCII letter or digit and continues with letters, digits,
underscores, and hyphens; roots and sibling names must be unique. Registration
also rejects reserved root names, such as shell words and standard tools, and
these declarations:

- a group with no subcommands;
- an input source that names a field the input does not have, overlapping
  input sources, duplicate positional fields, a required positional after
  an optional one, and an array positional that is not the last;
- an option named `help` or `json`;
- a `permission` on a `native` leaf, a `permission` that names a category
  none of the leaf's groups declares, and a category declared twice in the
  command set;
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
| `positionals` | Ordered fields supplied as positional arguments. They have no named-option form. The last may be an array, which takes every positional token left, as `demi attachment upload <path>...` does; usage shows it with `...`. |
| `stdinField` | A string field populated from finite stdin: a quoted heredoc, a pipe, or input redirection. It has no option or positional form. |
| `restField` | An array receiving raw tokens after `--`. It has no named-option form. |
| Remaining input fields | Named options such as `--path notes.txt`. Their schemas define values, optionality, boolean flags, enums, and repeated array options. |
| `output.json` | A schema enabling validated structured output through `--json`. |
| `media` | The leaf may return media, images and videos sent where its stdout goes ([Return media](#return-media)). Help marks it, and the dispatcher fails a call of a leaf without it that returns one. |

Unknown options, unknown fields, missing required values, duplicate scalar
values, and schema failures reject execution, and one rejection names every
field that failed. It names the field rather than repeating its value, which
may be a whole stdin body: `"count" is not of type "integer"; "path" is a
required property`. An unknown option's rejection also names the command, such
as `Unknown option "--bogus" for "demi expose add"`, since a script may run
several commands. The parser reports a missing option value before it
consumes the next option. `--name=value` supplies an option value that begins
with `--`. Without `restField`, a standalone `--` ends option parsing and the
tokens after it are positionals; with `restField`, they fill that field. A body
option such as `--content` is rejected with a diagnostic directing the caller to
remove it and use stdin. A finite stdin body is at most 1 MiB.

Conversion belongs to the CLI alone. An argv token is text, so the parser turns
it into the number, boolean, or array element its field declares, each element
of a repeated option separately, and then validates the whole input at once. A
token that spells no such value, such as `twelve` for a number, stays text, so
that validation rejects it together with every other failure. Arguments that
arrive as JSON, such as an `rpc` call's arguments at the backend
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
the web app validates count the same unit
([Validation at entry](../architecture/contracts.md#validation-at-entry)).

### Help

The model's command help and every `--help` come from one renderer over the
declarations. The model's help opens with one paragraph of defaults that every
command follows unless its own help says otherwise: success prints raw text on
stdout; failure writes an error message to stderr and exits non-zero; `--help`
works at any level; usage writes `<placeholders>` for values and `[brackets]`
for optional arguments; values containing spaces are quoted; stdin bodies use a
quoted heredoc, pipe, or input redirection and have no option;
`--name=value` and `--` pass values that begin with `--`; and a command
marked as returning media attaches its images and videos to the result when
its stdout is the job's output, and otherwise writes a single one's bytes as
its stdout. Each root's help
follows. An empty command set renders no help.

Help displays a complete usage template, value placeholders, required and
optional arguments, enum choices, and repeatable options. A leaf with a
`permission` adds one line: the command needs the user's permission in each
conversation, and without it the command fails and the user is asked. Stdin bodies appear
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
| `agent spawn` | none | profile, description, JSON output | task brief |
| `agent send`, `resume` | id | JSON output | message |
| `agent abort`, `show` | id | JSON output | unused |
| `agent list`, `profiles` | none | JSON output | unused |
| `host shell` | one quoted script | host | streamed to the remote program |
| `host list`, `current` | none | none | unused |
| `skills list` | none | JSON output | unused |
| `skills add`, `enable`, `disable` | repository | skill, repeated | unused |
| `skills update`, `remove` | repository | none | unused |

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
A command package never receives an unvalidated CLI request. An `rpc` leaf
reaches the backend as an `rpc_call` that names only its job: the backend checks
the call against its live record of that job
([Bind jobs to their caller](sessions-and-targets.md#bind-jobs-to-their-caller)),
validates the arguments again, checks the conversation's grant of the leaf's
[permission category](../agent/permissions.md#the-check), if it names one, and
dispatches the call in process to the invoking node's command set. A call
without the grant ends with exit status 1 and reaches no handler.

Native bindings pin an exact descriptor hash and operation. Missing operations,
inconsistent hashes, and unknown packages prevent a usable manifest.
[Native execution](native-runtime.md#bind-an-exact-package) owns catalog
composition, immutable releases, installation, and service lifetime.

## Handle an rpc call

An `rpc` handler receives its call as data and acts only through messages;
[The TypeScript boundary](../architecture/contracts.md#the-typescript-boundary)
says why. For example, `demi expose add 3000` in a job reaches the
backend as an `rpc_call`. The backend builds an invocation from the call and its
record of the job, and gives the handler that invocation and a port. The
handler of a plugin's leaf forwards both to its plugin as a command request,
whose port offers these operations among the plugin's others
([The contract](../architecture/plugins.md#the-contract)).

The invocation carries:

| Field | Meaning |
| --- | --- |
| Path and argv | The leaf's path from its root, and the raw argv. |
| Arguments | The input, validated against the leaf's schema. A `stdinField` body is already among them. |
| `json` | Whether the caller passed `--json`. |
| Host | The Host the job runs on: the conversation's primary Host, or an attached Host for a job `demi host shell` started there ([Attached hosts](sessions-and-targets.md#attached-hosts)). A handler that reads or writes the invoker's files does so on this Host. |
| cwd and environment | The invoking shell's directory and environment. |
| Command context | The [command context](native-runtime.md#command-context) from the backend's record of the job: conversation, caller, and locale. |
| Caller | The agent node the job runs for, from the same record; none for a job no agent started. A job the handler starts on another Host carries it on. |
| Stdin | Whether the calling process has a pipe on its stdin. |
| Stdout | Where the calling process's stdout goes: `job`, the job's output, or `elsewhere` ([Where a command's stdout goes](runner.md#where-a-commands-stdout-goes)). |
| Relayed pipes | The ids of the pipes relayed for the call's stdin and stdout. |

The port is the handler's side of the call. Each of its operations is one
request and one reply:

| Operation | Meaning |
| --- | --- |
| Write stdout, write stderr | Output. Stdout flows through the call's relayed pipe, so a write waits until the calling process has read enough; stderr goes to the runner as messages ([Deliver IO](#deliver-io-and-release-an-invocation)). |
| Return a medium | One image or video, named as a blob of the conversation owner's namespace that the handler put first ([Return media](#return-media)). |
| Read stdin | The next chunk of a finite stdin, on demand. |
| Read live stdin | The next interactive write to the job, until the job ends. |
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

A handler ends with an exit status. A handler error writes `<root>: <message>`
to stderr and exits 1. A call the runner cancels ends with 130. A call whose
calling process closed its stdout, as `head` does once it has read its lines,
ends with 141 and nothing on stderr, as a program that writes to a closed pipe
ends in a shell; so `demi shell output 17 --raw | head -n 20` prints its
lines and no error. A call stopped for another reason, such as its job
exiting, a relayed pipe failing, or the backend shutting down, ends with 1 and
its cause on stderr. When several causes stop a call, the first is the one
reported.

## External command clients

An external program such as `xargs` calls a declared root through an alias to
`demi-runner`. The alias basename selects the root. The client forwards raw argv,
cwd, environment, and its live execution context, then streams command IO. It
compares its own stdin and stdout with the job's, as a builtin does, and
forwards the answers
([Where a command's stdout goes](runner.md#where-a-commands-stdout-goes)). It
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

## Return media

Beside stdout and stderr, a declared command may return media: images and
videos, which [Media a command returns](../agent/runtime.md#media-a-command-returns)
sends where the command's stdout goes. That section owns the rule and what
the model receives; this one says how a command returns a medium. For
example, `demi browser screenshot t1` returns its PNG. Run on its own, its
stdout is the job's output, so the runner keeps the PNG with the job and the
job's result attaches it; with `> shot.png`, the runner writes the PNG's
bytes into the file.

- **The handler returns, the dispatcher routes.** A `native` handler returns
  a medium through its output writer, as medium records
  ([Response records and completion](native-runtime.md#response-records-and-completion));
  an `rpc` handler through its port, naming a blob it put, which the backend
  streams to the runner as `rpc_medium { callId, after, size, pipe }`. No
  handler writes a medium to stdout itself: the dispatcher in the runner
  routes every medium by the invocation's `stdout`, so the rule is the same
  for every command, a builtin or an alias, `native` or `rpc`.
- **Stdout is the job's output (`job`).** The dispatcher hands the medium to
  the job, which numbers and keeps it
  ([Pipes and output](runner.md#pipes-and-output)), and writes the medium's
  line into the command's stdout where the handler returned it: after the
  stdout records before it, for a `native` call, and after the first `after`
  bytes of the call's stdout, for an `rpc` call, whose stdout and media arrive
  on different paths. Under `--json`, the lines follow the JSON value once the
  dispatcher has released it, so the value stays one JSON value; a command
  that returned media may print no value, and its lines are then its
  stdout.
- **Stdout goes elsewhere (`elsewhere`).** The dispatcher holds the medium in
  the job's directory and, when the command completes with status 0, writes
  its bytes as the command's stdout. A second medium, or stdout bytes beside
  the medium, the `--json` value included, fail the command with status 1,
  and the held medium is not written: a second medium with the message
  [Where a medium goes](../agent/runtime.md#where-a-medium-goes) gives, and
  stdout beside it with
  `<command>: a medium must be all of its stdout when its stdout is not the job's output`.
  Stdout a command wrote before its medium has passed already; a handler
  that follows the next point writes none. A command that fails writes none
  of its medium, as it releases no captured `--json` output.
- **What the handler knows.** The invocation's `stdout` tells the handler
  where its stdout goes, so a command that prints text about its medium,
  such as a screenshot's size, prints it only when its stdout is the job's
  output. It needs to know nothing else: the numbers, the lines and the
  bounds are the runner's.
- **Checks.** A medium from a leaf that does not declare `media`, a medium
  whose bytes are not an image or video type of the model-media table
  ([Accepted attachment types](../providers/models.md#accepted-attachment-types)),
  and a medium over 16 MiB fail the command: each is a defect of its handler,
  which the dispatcher reports rather than repairs. Only an invocation a job's
  command makes can return media; a user stream's or a package call's writer
  refuses them.

## File commands

`demi file read`, `create`, `edit`, and `patch` run in the native `demi.file`
service, beside the file. Each resolves relative paths against the invocation's
cwd and stops at the invocation's cancellation. Mutations run one at a time in a
service: one mutation's planning and writes finish before the next begins. Each
replaced file is published atomically, from a temporary file beside it whose
name says it is Demi's ([File contents](runner.md#file-contents)), and a patch
that changes several files restores the files it already changed when a later
write fails. Create, edit, and patch record their writes for
[edit tracking](edit-tracking.md).

`demi file read` prints a file's bytes, and declares `media`: a file of at
most 16 MiB whose bytes are an image or video type of the model-media table it
returns as a medium instead ([Return media](#return-media)). So
`demi file read shot.png` shows the image to the model, several such reads in
one script show it each image in order, and `demi file read shot.png > copy.png`
still copies the bytes.

## Attachment commands

`demi attachment upload <path>...` gives the user files of the Host as
attachments of the conversation, which the agent's messages then show. For
example, the agent took a screenshot of a sign-in page and recorded a video
of the flow:

```text
$ demi attachment upload out/login.png demo.mp4
a3  out/login.png  image/png  421888 bytes
a4  demo.mp4       video/mp4  8598311 bytes
```

Its reply then shows both, the image as an image and the video playing in
place ([Files named in messages](../product/file-previews.md#files-named-in-messages)):

```markdown
The sign-in page works again:

![The fixed sign-in page](attachment:a3)

![The flow from sign-in to the dashboard](attachment:a4)
```

- **A copy, kept.** An attachment is the file's bytes as they were when the
  command ran, stored as a blob in the conversation owner's namespace, with a
  record in the conversation: its number, the file's name, its media type,
  which the backend reads from the bytes as it does for the user's uploads
  ([Uploads and media](../product/web-api.md#uploads-and-media)), its size and
  its blob. Nothing changes or removes it afterward
  ([Retention](../backend/storage.md#retention)), so a message shows it
  whatever becomes of the file, the Host or the conversation: a file
  rewritten or deleted, a device offline, a Cloud stopped or reset, an
  archive, a target change.
- **Numbers.** Attachments are `a1`, `a2`, … in the conversation, from its
  `attachment` sequence, never given twice
  ([Identifiers the model sees](../agent/runtime.md#identifiers-the-model-sees)).
  A Fork's destination keeps its source's attachments, since its history
  names them.
- **Files.** A path resolves against the invocation's cwd. A path that is not
  a regular file, or a file over 25 MiB, the user's own upload limit, fails
  with a line on stderr that names the path and why; the command goes on with
  the next path and exits with status 1, and the files it stored keep their
  numbers and lines. A size is a count of bytes, as a medium's line gives it.
  `--json` prints `{ attachments: [{ id, path, mediaType,
  size }] }`.
- **No medium.** The command prints the numbers and returns no medium: the
  attachment is for the user, and the model reads a file with `demi file read`
  when it needs to see it.
- **Where it runs.** `demi attachment` is the product's group beside
  `demi host`, handled by the backend: the runner of the invocation's Host
  ([Handle an rpc call](#handle-an-rpc-call)) reads the file beside the
  invocation and streams its bytes to the backend through a pipe
  ([File contents](runner.md#file-contents)), and the backend stores the blob
  and then the record.

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
| A `native` and an `rpc` command of one job, their stdout the job's, each print a line, return an image and print another line | The job's media 1 and 2, in the order they reached the runner; each medium's line lies between its command's two lines |
| `xargs -n 1 demi file read` over two PNG files, its stdout the job's | Both are returned as media of the job: the alias makes the same comparison as a builtin |
| `demi file read shot.png \| wc -c` | The file's size; the job has no medium |
| `demi browser screenshot t1 --json --output x.png \| jq .path` | The path: a leaf that declares `media` but returns none writes its stdout as any command does |
| A leaf without `media` returns a medium, and a handler returns a medium that is not an image or video | The command fails, and nothing is kept or written for it |
| A command whose stdout goes elsewhere returns a medium and then prints a line | Status 1, nothing on stdout, the message on stderr |
| A command whose stdout goes elsewhere returns two media | Status 1, nothing on stdout, the message naming both ways out on stderr |
