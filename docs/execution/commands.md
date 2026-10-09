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
`demi agent` group is specified in [Subagents](../agent/subagents.md), and the
`demi shell` group in [The whole output](../agent/runtime.md#the-whole-output) and [Stopping a command](../agent/runtime.md#stopping-a-command).

## Declare a command

For example, `demi file view shot.png` shows the model an image on the execution target.
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

Placed under the `demi file` group, the leaf handles `demi file view shot.png`.
The dispatcher validates `{ "path": "notes.txt" }` against `input` before
invoking `file.read`, and the native service decodes the same value into
`FileRead`, together with the invocation's cwd and IO. The arguments and
results of every `demi.file` operation are types of one contract crate,
`command-package-file-protocol`, that the declarations and the native program both link, so a
declaration and its handler cannot describe different arguments
([Contract crates](../architecture/contracts.md#contract-crates)).

An `rpc` leaf runs a handler in the backend instead.
`demi skills add vercel-labs/agent-skills` declares its arguments the same
way:

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// The repository: owner/name on GitHub, or a Git URL
    repository: String,
    /// Turn on only these skills of the repository
    skill: Vec<String>,
}
```

Its leaf has `kind: "rpc"`, `positionals: ["repository"]`, and an
`output.json` schema derived from the type it prints with `--json`. Its
handler receives the validated arguments as an `AddArgs`, adds the source,
and prints its skills ([Handle an rpc call](#handle-an-rpc-call)).

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
([The check](../agent/permissions.md#the-check)). A leaf whose input field
names a device or a project that the call may bring into the conversation
declares that field as `bringsHost`, as `demi conversation move` declares its
project, and the dispatch adds Manage Devices when it does
([Several categories](../agent/permissions.md#several-categories)). The
runner ignores these fields.

A group contains subcommands and does not execute a handler. A command name
starts with an ASCII letter or digit and continues with letters, digits,
underscores, and hyphens; roots and sibling names must be unique. Registration
also rejects reserved root names, such as shell words and standard tools, and
these declarations:

- a group with no subcommands;
- an input source that names a field the input does not have, overlapping
  input sources, duplicate positional fields, a required positional after
  an optional one unless it is the last positional and directly follows the
  only optional one, an array positional that is not the last, and a
  `positionalOptions` field that is not a positional;
- an option named `help` or `json`;
- a `permission` on a `native` leaf, a `permission` that names a category
  none of the leaf's groups declares, and a category declared twice in the
  command set;
- a `bringsHost` on a `native` leaf, or one that names no string field of the
  input;
- a `stdinField` that is not a string, or a `restField` that is not an array of
  strings;
- a field schema outside the [input subset](#the-input-subset).

A rejection names the command and, when a field is at fault, the field. It
happens when the command set is built, before any call.

## Parse input and render help

One implementation of everything this section describes, the command-tree
library ([Crates](../architecture/crates-and-packages.md#crates)), serves both
the runner and the backend. The runner parses argv and answers `--help` with it;
the backend renders the model's capability index and checks registrations with it.
The library builds a clap command for the selected leaf from its declaration,
and clap parses argv; the `jsonschema` validator over the schema schemars
derived checks what clap cannot express, and validates arguments that arrive
as JSON. Both report a usage error in clap's shape, so it reads the same
wherever it is raised, and no second set of rules exists to drift from the
declaration.

Each input field has one source:

| Declaration | Source and behavior |
| --- | --- |
| `input` | The JSON Schema of the leaf's argument type. It validates the whole input. |
| `positionals` | Ordered fields supplied as positional arguments. They have no named-option form, unless `positionalOptions` names them. The last may be an array, which takes every positional token left, as `demi attachment upload <path>...` does; usage shows it with `...`. One optional positional may stand directly before a required last one, as `demi browser key <tab> [<ref>] <key>` does: two tokens fill the tab and the key, three fill all three. |
| `positionalOptions` | Positionals that may also be given as a named option, as a browser target's `ref` is both `demi browser click t1 e3` and `--ref e3`, since outputs print it as `[ref=e3]` and models write it both ways. Usage shows the positional form. Giving both is a usage error. |
| `stdinField` | A string field populated from finite stdin: a quoted heredoc, a pipe, or input redirection. It has no option or positional form. |
| `stdinRead` | When the `stdinField` is read; absent, always. `{ unless: [options] }` leaves it unread, and stdin with the calling process, when one of the options is given, as `file edit` reads no blocks with `--old`. `{ with: [options] }` reads it only when one of them is given, as `browser find` reads a query body only with `--query`, so a call from a shell whose stdin is an empty pipe is not refused for an empty body. An option counts as given unless its value is `false`. It names only options of the leaf, and only beside a `stdinField`; help shows it on the stdin line: `Stdin body: blocks, not read with --old`, `Stdin body: body, read only with --query`. |
| `restField` | An array receiving raw tokens after `--`. It has no named-option form. |
| Remaining input fields | Named options such as `--path notes.txt`. Their schemas define values, optionality, boolean flags, enums, and repeated array options. |
| `output.json` | A schema enabling validated structured output through `--json`. |
| `media` | The leaf shows media to the model, handing them to its job whatever its stdout is ([Return media](#return-media)). Only `file view` declares it, and the dispatcher fails a call of a leaf without it that returns one. |

For example, a model runs `demi file edit a.ts b.ts` on a leaf that takes
one optional path, or `demi browser read t1 --selector main`:

```text
error: unexpected argument '--selector' found

  tip: a similar argument exists: '--css'

Usage: demi browser read <tab> [--css <selector>] …

For more information, try '--help'.
```

A usage error is clap's: one error at a time, in clap's words, with no
colour, the leaf's usage line and the pointer to `--help`, on stderr, with
exit status 2, as every program built with clap reports it and as models
know. The usage line is the one help renders from the declaration, so the
error, `--help` and the capability index show the same usage. Demi adds its
own hints as clap's `tip:` lines, such as `"demi file edit" reads blocks only
from stdin; remove --content and use a quoted heredoc`, and clap suggests a
near name for a misspelt option or operation. What clap cannot express, a
length, a pattern, a range or an item count, the validator checks once clap
has parsed, and reports in the same shape: `error: "path" is longer than 4096
characters`. Arguments that arrive as JSON, such as an `rpc` call's arguments
at the backend or the arguments of a one-shot user call, are validated as
they arrive and reported the same way: `"7"` for a numeric field is a usage
error there, not a 7.

- **Fields.** A positional field is a positional argument; the last may take
  every token left. A named option is `--name <value>`; a value may begin
  with `-`, and one that begins with `--` is given as `--name=value`. A
  repeated array option takes each occurrence. `restField` takes the tokens
  after `--`; without one, `--` ends options.
- **Booleans** are flags: `--exact` alone is true, `--exact=false` is false.
  A `true` or `false` token right after a boolean flag is refused with the
  tip to write `--exact=false`, rather than taken as the next positional.
- **A body option** such as `--content` is refused with the tip to use stdin.
  A finite stdin body is at most 1 MiB.

The earlier parser listed every failed field in one message and exited 1; it
swallowed the token after a boolean as the boolean's value, so
`demi browser open --show https://example.com` failed as a missing URL, and
it named no usage, so a model read the help after every mistake.

A group-only invocation or `--help` prints help and exits successfully without
running a handler or reading stdin. For example, `demi file view --help` must
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

Every `--help` and the model's capability index come from one renderer over
the declarations ([System prompt](../agent/system-prompt.md#capability-index)).
A group's declaration carries its index entry, at most 600 characters,
refused at registration when longer. The index opens with one paragraph of
defaults that every command follows unless its own help says otherwise: success prints raw text on
stdout; failure writes an error message to stderr and exits non-zero; `--help`
works at any level; usage writes `<placeholders>` for values and `[brackets]`
for optional arguments; values containing spaces are quoted; stdin bodies use a
quoted heredoc, pipe, or input redirection and have no option;
`--name=value` and `--` pass values that begin with `--`; text is read with
the standard tools, and `file view` alone shows the model an image, a video
or a PDF, from a file or a pipe. Each group's entry, its operations' names and the pointer to its
`--help` follow. An empty command set renders no index.

Help displays a complete usage template, value placeholders, required and
optional arguments, enum choices, and repeatable options. A leaf with a
`permission` adds one line: the command needs the user's permission in each
conversation, and without it the command fails and the user is asked. Stdin bodies appear
in their own section and in a quoted heredoc template. `--json` appears only
on commands with a JSON output schema. Usage templates are generated from the
declaration; declarations do not carry separate examples to keep in sync.

For example, the generated structure for a patch is:

```sh
demi file patch <<'EOF'
<patch>
EOF
```

Angle brackets identify placeholders; square brackets identify optional
arguments. The agent substitutes actual values and quotes shell arguments.

### Demi command inputs

| Command | Positional input | Named options | Stdin |
| --- | --- | --- | --- |
| `file view` | path or `-`, repeated, optional | none | one medium, read when no path or `-` is given |
| `file edit` | path, optional | old, new, occurrence or context | SEARCH/REPLACE blocks with their files' paths, read only without `--old` |
| `file patch` | none | none | unified diff |
| `agent spawn` | none | profile, description, JSON output | task brief |
| `agent send`, `resume` | id | JSON output | message |
| `agent abort`, `show` | id, repeated | JSON output | unused |
| `agent list`, `profiles` | none | JSON output | unused |
| `host shell` | one quoted script | host | streamed to the remote program |
| `host list`, `current` | none | none | unused |
| `skills list` | none | JSON output | unused |
| `skills add`, `enable`, `disable` | repository | skill, repeated | unused |
| `skills update`, `remove` | repository | none | unused |

`file edit` takes its change in one of two forms, never both: blocks on
stdin, or `--old` and `--new` for a one-line change, with `--occurrence` or
`--context`, not both, to choose among several matches. With `--old` it
reads no stdin, so `… | while read f; do demi file edit "$f" --old a --new b;
done` leaves the loop's input to the loop, as `git apply` reads stdin only
when it names no file. `host shell` takes the script as one quoted argument so its stdin
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
says why. For example, `demi skills add vercel-labs/agent-skills` in a job reaches the
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
| Relayed pipes | The ids of the pipes relayed for the call's stdin and stdout. |

The port is the handler's side of the call. Each of its operations is one
request and one reply:

| Operation | Meaning |
| --- | --- |
| Write stdout, write stderr | Output. Stdout flows through the call's relayed pipe, so a write waits until the calling process has read enough; stderr goes to the runner as messages ([Deliver IO](#deliver-io-and-release-an-invocation)). |
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

A handler ends with an exit status. A handler's error is a runtime error,
written as GNU tools write theirs: the command's path, the object it concerns
when there is one, and the reason, `demi file edit: /src/app.ts: No such file
or directory`, on stderr, with exit status 1. The dispatcher writes the
command's path once; the handler names the object, and an operating-system
error gives its reason as `strerror` words it, without Rust's `(os error 2)`.
No prefix of Demi's own precedes it, `demi-runner:` or `command_failed:`. A
command that takes several values handles each in order, writes one such line
for each that fails, goes on with the others, and exits 1 when any failed, as
`cat a missing b` does. A call the runner cancels ends with 130. A call whose
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
compares its own stdin with the job's input, as a builtin does, and
forwards the answer ([A job's own input](runner.md#a-jobs-own-input)). It
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

Beside stdout and stderr, a declared command may return media: images,
videos and PDF documents that it hands to its job for the model to see.
[Media the model views](../agent/runtime.md#media-the-model-views) owns the
rule and what the model receives; this section says how a command returns a
medium. `demi file view` is the one command that does, so a model has one way
to show itself a file, and every other command, a plugin's included, writes
the bytes it makes to its stdout for a pipe into `demi file view` or a file.

- **The handler returns, the runner keeps.** A `native` handler returns a
  medium through its output writer, as medium records
  ([Response records and completion](native-runtime.md#response-records-and-completion)).
  The dispatcher in the runner hands every medium to the invocation's job,
  whatever the command's stdout is: a pipe, a file, a command substitution or
  the job's output. The job numbers and keeps it
  ([Pipes and output](runner.md#pipes-and-output)) and writes its line into
  the job's output as it arrives. A medium never becomes stdout bytes.
- **Checks.** A medium from a leaf that does not declare `media`, and a
  medium over 16 MiB, fail the command: each is a defect of its handler,
  which the dispatcher reports rather than repairs. `demi file view` checks
  the bytes' type itself, against the list the job carries, before it returns
  them ([Media the model views](../agent/runtime.md#what-demi-file-view-shows)).
  Only an invocation a job's command makes can return media; a user stream's
  or a package call's writer refuses them.

## File commands

`demi file view`, `edit`, and `patch` run in the native `demi.file` service,
beside the file. Each resolves relative paths against the invocation's cwd
and stops at the invocation's cancellation. Mutations run one at a time in a
service: one mutation's planning and writes finish before the next begins.
Each replaced file is published atomically, from a temporary file beside it
whose name says it is Demi's ([File contents](runner.md#file-contents)), and
an edit or a patch that changes several files restores the files it already
changed when a later write fails. Edit and patch record their writes for
[edit tracking](edit-tracking.md).

### Editing files

`demi file edit` is the way an agent changes the files of its task, made to
do what models otherwise write a Python script for. In this project's own
Claude Code transcripts of two weeks, models changed files 8,061 times with a
Python script against 717 times with the Edit tool beside it: 93% of the
scripts replaced exact text, 63% checked that the text was there, 36%
changed two files or more, 20% replaced a section found between two anchors,
and 84% built or tested in the same call. A script does all of that in one
call, which a one-replacement tool cannot; but `str.replace` changes nothing,
silently, when the text is not there, and the conversation cannot show what
the script wrote. `demi file edit` does the same in one call, fails loudly,
and records every change.

For example, the agent adds a module, registers it, renames a call in a
long function without copying its body, and builds, in one call:

```text
demi file edit <<'EOF' && cargo check
crates/backend/src/conversation/stream.rs
<<<<<<< SEARCH
=======
//! The conversation's stream of frames.

pub(crate) struct Stream;
>>>>>>> REPLACE

crates/backend/src/conversation/mod.rs
<<<<<<< SEARCH
mod socket;
=======
mod socket;
pub(crate) mod stream;
>>>>>>> REPLACE

crates/backend/src/conversation/socket.rs
<<<<<<< SEARCH
pub(crate) async fn serve(socket: Socket) {
.......
    send_frames(&socket).await;
}
=======
pub(crate) async fn serve(socket: Socket) {
.......
    stream::send(&socket).await;
}
>>>>>>> REPLACE
EOF
```

It prints what it did, and each change as the file now reads:

```text
Created crates/backend/src/conversation/stream.rs (3 lines)
Edited crates/backend/src/conversation/mod.rs (+1)
  14  mod socket;
  15  pub(crate) mod stream;
  16  mod state;
Edited crates/backend/src/conversation/socket.rs (+1 −1)
  88      }
  89      stream::send(&socket).await;
  90  }
```

- **Files.** On stdin, outside the blocks, a line names the file the blocks
  after it change, as Aider writes it; blank lines may stand between blocks,
  and nothing else. `demi file edit <path>` with blocks and no path lines
  changes that one file; a path argument together with path lines is
  refused.
- **Stdin, not arguments.** A quoted heredoc passes the text byte for byte,
  so no quote, `$` or backslash needs escaping. Text passed as an argument
  does: a model that edited a function with `--old $'…'` wrote each quote as
  `\x27`, and a command substitution, `"$(cat <<'EOF' … EOF)"`, drops the
  text's last newlines. `--old` and `--new` stay for a one-line change to the
  path argument, `--old beta --new gamma`, with `--occurrence` or
  `--context`, not both, to choose among several matches; with `--old` the
  command reads no stdin. Blocks and `--old` together are refused.
- **Text, not lines.** The markers `<<<<<<< SEARCH`, `=======` and
  `>>>>>>> REPLACE` are whole lines and may carry trailing spaces or tabs. A
  block's SEARCH is the text between the first two, without the line ending
  of its last line, and it matches that text anywhere in the file, part of a
  line included, as Claude Code's Edit matches its `old_string`; its REPLACE,
  the text up to the third, takes its place, with the file's own line
  endings. So a SEARCH of whole lines replaces those lines, and a SEARCH of
  `withinLimit(` inside a line changes only that call. A SEARCH that ends a
  line, replaced by an empty REPLACE, takes its line ending with it, so
  deleting lines leaves no blank one. The SEARCH matches as written:
  whitespace, quotes and indentation included.
- **A new file.** An empty SEARCH creates its file with the REPLACE as its
  content, and fails when the file exists, so an edit never overwrites a
  file it did not read. A file that is not to be created must exist. Writing
  a whole file the agent means to replace is `cat > file <<'EOF'`, which edit
  tracking records as well.
- **A section.** A SEARCH line of seven dots, `.......`, stands for any run
  of lines, none included, as short as the rest of the block allows, and the
  block then matches whole lines: the
  agent names a function by its first and last lines without copying its
  body. Seven dots, the markers' width, because a line of three is Python's
  `...` and a placeholder in many files. The REPLACE holds either no such
  line, and replaces the whole section, or as many as the SEARCH, in order,
  each keeping the lines its SEARCH line stood for, as the example keeps the
  function's body.
- **Exactly once.** A SEARCH must match exactly one place; overlapping
  matches count as one. An error names the file by its full path, the block,
  the rule and that nothing was written, so the agent knows what to fix and
  that no file changed:
  `demi file edit: /work/app/src/limit.ts: block 2: its SEARCH is not in the
  file; a SEARCH must match the file's text exactly, whitespace included, and
  nothing was written. The closest lines are 40-42: …`, or
  `…: block 2: its SEARCH occurs 3 times, at lines 12, 40 and 77; include more
  of the text around it so it occurs once, and nothing was written`. A path
  that does not exist, for a block whose SEARCH is not empty, fails as
  `demi file edit: /work/app/src/limt.ts: No such file or directory`.
- **Together or not at all.** Every block of every file is matched against
  the files as they were before the call, blocks of one file must not
  overlap, and nothing is written until every block matched; then the files
  are written, and a write that fails restores the files written before it.
- **What it prints.** A line for each file, `Created <path> (<n> lines)` or
  `Edited <path> (+<added> −<removed>)`, in the order of stdin, and under an
  edited file each change as the file now reads, numbered as `cat -n`
  numbers, with one line of context on each side, as Claude Code's Edit
  shows its result. Changes whose shown lines would touch or lie within two
  lines of each other print as one piece, as `diff` joins hunks; other
  pieces of a file are separated by a line `--`, as `grep -C` separates its
  groups, so no gap in the numbering goes unmarked. Changes that would
  print more than 60 lines in all print their first 60 and, for each file
  cut, one line saying how to read the rest:
  `[… 140 more lines changed; read them: sed -n 120,260p <path>]`. A blank
  line prints as its number alone. An edit that changes nothing prints
  `Edited <path> (no change)` and writes nothing. The agent sees its result
  without reading the file again.
- **What stdin may hold.** Outside the blocks, a line names a file and must
  be followed by a block; a block needs a file before it, from a path line
  or the argument; a file that a block creates has no other block in the
  call. A path line is the whole line, so stray prose before the blocks
  fails as a file that does not exist, naming the line.
- A text with a line that is exactly a marker, or a line of seven dots,
  cannot be written as a block; `--old` and `--new`, or `demi file patch`,
  write it.

The capability index entry for `demi file` says to use it whenever the
agent changes the files of its task or wants to see an image, a video or a
PDF, and shows the shape of the example above, two files and a build in one
call, and `demi browser screenshot t1 | demi file view`, since a model takes
up a tool by the example it has seen. It says that text is read with `cat`,
`sed -n` or `rg`.

`demi file patch` applies a unified diff from stdin, as `git diff` writes it,
to one or more files. It ignores the line counts of each hunk's header, as
`git apply --recount` does, since a model that writes a diff by hand often
miscounts them: a model's diff with a miscounted header failed with `Patch
hunk line counts do not match header`. It finds each hunk by its context and
removed lines where they match exactly once, or, where they match several
times, at the match nearest the header's line; a tie, or several matches
under a header without numbers, fails and names them. A header without
numbers, `@@`, is accepted, and so is the text git writes after a header's
second `@@`. An empty line in a hunk is an empty context line, as
`git apply` reads it. A `---` and `+++` pair starts another file only when an
`@@` line follows it, so a removed line `-- x` before an added line `++ y`
stays in its hunk. A hunk that matches nowhere fails the patch, naming its
file and hunk, and changes nothing.

`demi file view` shows the model images, videos and PDF documents, from its
paths or its stdin, and declares `media`; it prints nothing to stdout
([Media the model views](../agent/runtime.md#media-the-model-views)). It
has no reading of text: a model reads a file with `cat`, `sed -n` or `rg`,
the standard utilities on every Host
([Standard utilities](runner.md#standard-utilities)), and a text file given
to `demi file view` fails with the line that names `cat`.

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
  attachment is for the user, and the model views a file with
  `demi file view` when it needs to see it.
- **Where it runs.** `demi attachment` is the product's group beside
  `demi host`, handled by the backend: the runner of the invocation's Host
  ([Handle an rpc call](#handle-an-rpc-call)) first hashes the file beside the
  invocation (`fs_hashFile`, with the 25 MiB limit, so a larger file is
  refused before a byte is read); a blob the conversation owner's namespace
  holds already is not read again, and the record names it, with the media
  type the backend reads from the blob's first 64 KiB. Otherwise the runner
  streams the file's bytes to the backend through a pipe
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
| `demi file view a.png` with its stdout a pipe, a file and the job's output | The image is the job's medium each time; the pipe and the file receive nothing |
| `xargs -n 1 demi file view` over two PNG files | Both are media of the job, numbered in order: the alias hands them over as a builtin does |
| A leaf without `media` returns a medium | The command fails, and nothing is kept for it |
