# Claude Code

The Claude Code provider talks to the vendor through the vendor's CLI. Demi
installs and updates its own copy of that CLI on the machines that run it. It
never uses a `claude` found on `PATH`, a copy the machine's owner installed, or
a system package: what runs is a version Demi chose and verified.

The CLI is started with no tools of its own, no session persistence and no
device configuration, so it is a channel to the vendor and nothing else. It
does not touch the workspace, which is why **where it runs is a choice**, not a
consequence of where the conversation's files are. The model's tools still run
where the conversation's tools run: the CLI hands each tool call back to Demi,
and the agent executes it.

For example, a conversation on a paired laptop uses Claude Code. Its request
wakes the user's Cloud and starts the CLI there with the account's token. The
CLI streams the model's answer back as stream-json. When the model calls
`shell_exec`, the CLI asks Demi for the result over the SDK MCP channel and
waits. The run ends with that tool call, and the agent runs the command on the
laptop. The next run gives the waiting CLI the result, and the model continues
in the same process.

This document specializes the
[provider contract](providers.md#provider-contract) for a provider that runs a
process.

## Execution boundary

The Claude Code provider is a backend component: like every runtime, its
runtime lives on the user's shard. It starts the CLI through the Host process
interface on the user's Cloud and exchanges stream-json on the process's
standard input and output. Which CLI that is, and which machine, are defined
below: Demi's own verified copy, on the user's Cloud, whatever the
conversation's execution target is.

The provider reads its account's setup token from the vault and sets it as
`CLAUDE_CODE_OAUTH_TOKEN` in the spawn environment. The runner and the CLI
therefore receive this credential: the user's Cloud is trusted with that
account's token, which on a shared instance means every user
([Scope](providers.md#scope)). When the token cannot be read, the request fails
with the authentication error; the CLI is never started without a token.

The CLI sends its inference traffic directly to the vendor. Demi adds no
inference proxy and no remote-inference call. The quota probe is a backend
operation that needs no CLI and no machine
([Vendor quota](usage-and-quota.md#vendor-quota)), and a setup token needs no
refresh. The runner keeps no credential of its own: the token exists on the
Cloud only in the CLI process's environment.

The provider replays the transcript itself and turns the CLI's session
persistence off. A new process, after a Cloud stop or reset or a change of
account, model or thinking setting, starts from the backend's transcript, so
nothing depends on an earlier CLI session.

The provider uses the same Host process interface as every other Host
operation ([Host operations](../execution/runner.md#host-operations)), not a
copy of its own. It relies on three of its properties: an environment entry can
unset an inherited variable; standard output and standard error are byte
streams that end when the process exits, and waiting reports the exit code, the
signal or the failure to start; dropping the process handle asks the Host to
kill the process without waiting.

## Which version

The version Demi wants is the vendor's newest: the `latest` pointer of the
official distribution (`https://downloads.claude.ai/claude-code-releases`).
Nothing stores a wanted version. The backend reads the pointer and that
version's `manifest.json`, which gives each platform's byte size and SHA-256,
and keeps the answer for six hours; **Check for updates** reads it again at
once. It reads only the official host: a redirect is refused. Concurrent readers
share one read, and a version's manifest, which never changes once published,
is read once.

Demi always follows the official newest release; provider entries store no CLI
version and expose no version-selection or update-disable setting. The provider
names the newest version it was tested with; a newer one in use is worth a note
in settings and a line in a failure report, never a refusal.

What is **installed** is a fact of each machine and is asked of that machine
when someone wants to know. The backend keeps no table of it. A machine that is
offline or asleep has no answer, and showing a version is never a reason to
wake one.

A newer version does not interrupt work. A machine that already has a usable
version keeps answering with it while the newer one installs beside it; the
next CLI process after the install is the newer one. Only a machine with no
usable version waits for an install. Installing a version removes the others:
a process already running one keeps its executable where the system allows a
file in use to be removed, and where it does not, the removal is left for a
later install.

## The package

`demi.claude` is a native package of its own
([Bind an exact package](../execution/native-runtime.md#bind-an-exact-package)),
not part of `demi.builtin`: the runner knows nothing about Claude Code, and
removing the package and the backend's use of it leaves the rest of Demi as it
was. It has two operations:

| Operation | Input | Answer |
|---|---|---|
| `claude.ensure` | A release record: `{ version, platforms }`, each platform's official `url`, byte `size` and `sha256` | `{ version, path }`: the executable's absolute path |
| `claude.status` | — | `{ platform, installed }`: this machine's platform key and the versions it has, newest first |

The backend and the package decode these records with the same Rust types
([Contract crates](../architecture/contracts.md#contract-crates)). The backend
reaches the operations through a
[service stream](../execution/runner.md#service-streams), which carries no
arguments: the record is the stream's input, read to its end, and the answer is
one JSON document on its output, since the output is all a service stream
brings back: `{ ok: true, … }`, or `{ ok: false, code, message }` with one of
`invalid_release`, `unsupported_platform`, `download_failed`,
`verification_failed`, `install_failed`. A failed invocation's nonzero exit
does not discard this document: the backend validates it and reports its code,
message and requested version. An output with no document is a service that
failed. The command context names the conversation being served, or the
provider entry for work that belongs to none.

The backend builds the release record from the manifest; the package does not
read pointers or choose versions. It picks the platform from the machine, not
from the runner's own target: a runner built for musl runs on glibc machines,
and the CLI's `linux-*` and `linux-*-musl` builds are not interchangeable. A
platform the release does not carry is `unsupported_platform`, never another
platform's build.

`claude.ensure` installs under the Host user's `.demi/claude/<version>/`; a
Cloud image may carry a version under `/opt/demi/claude/<version>/`, which is
used when it is the wanted one. It downloads to a temporary file, enforces the
size, verifies the digest, and only then publishes the executable atomically,
with the same verified download and publication every Demi installer uses.
Callers asking for the same version share one download. A file that fails
verification is removed and the install fails; there is no fallback to another
version, location or executable.

The backend starts the CLI through the ordinary Host process interface with the
path `claude.ensure` answered, the account's token, a configuration home of
Demi's, and the CLI's own updater turned off. Demi is the only thing that
changes the version.

## Where it runs

The CLI runs on Cloud, always: the Cloud of the conversation's user for a
request, the Cloud of the acting user for work that belongs to no conversation
(installing after an account is added, **Test connection**). Every deployment
has Cloud ([Managed hosts](../cloud/managed-hosts.md)), Demi owns it, and it is
one kind of machine, so the CLI has one install to keep and one platform to be
right on. A conversation whose files and commands are on a paired device still
infers through the user's Cloud; its tools keep running on its own device, and
the account's token never reaches a paired device. Usage and plan need no CLI
and no machine at all: the backend asks the vendor itself
([Vendor quota](usage-and-quota.md#vendor-quota)).

A **placement** decides the machine. It is one rule in one place: what needs a
provider's process says what the work is and takes the Host, the run directory
and the command context it is given, and does not know why it got that one.

| Work | Placement |
|---|---|
| A conversation's request | The Cloud of the conversation's user |
| Work that belongs to no conversation | The acting user's Cloud |

A placement answers with candidates in order and the work takes the first that
is available; each list has one entry, and a different policy is a different
list. The placement reaches the Cloud through machine access, which wakes a
stopped Cloud and takes no conversation's file gate
([Host operations](../execution/sessions-and-targets.md#host-operations)). The
placement is also what makes a conversation
[use the Cloud as `provider`](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device),
which is all the Cloud's lifecycle knows of this: the conversation's turn keeps
the Cloud awake, and a reset holds the conversation and lets it go on
afterwards. Nothing in the lifecycle names Claude Code.

The process is **retained** between turns and is not activity
([Activity](../execution/resource-lifecycle.md#activity)): a Cloud that goes
idle stops with the process in it, and the next request wakes the Cloud, starts
the CLI again and replays the transcript. It runs in `~/.demi/claude/run` on the
Cloud, with `~/.demi/claude/config` as its configuration home, not in the
conversation's directory, which on a paired device is a path the Cloud does not
have; the CLI reads and writes nothing there that a conversation owns.

### How a runtime gets its process

A process provider cannot build its runtime from the provider alone: the
process needs a machine, and only the backend chooses one. `Provider::runtime`
of the Claude Code provider therefore refuses (`ProcessHostRequired`), and the
runtime is built over a placement instead:

```text
backend, on the user's shard                 provider-claude-code
  claude-code family -- same arguments --->  provider + placement = runtime
                                                  |
  placement.start(spawn) <------------------------+  a run needs a new process
    machine access: wake or admit the Cloud
    demi.claude: claude.status, claude.ensure when needed
    make ~/.demi/claude/run and ~/.demi/claude/config
    spawn(executable, run dir, config dir) -> the provider's spawn request
    Host process interface: start it, then release the Cloud's admission
  --- shell's Process ---------------------------->  the run drives it
```

`provider-claude-code` defines the placement contract: `start` starts a new CLI
process on the machine the placement chooses and answers shell's `Process`. The
provider builds the spawn request itself, from the executable's path and the
run and configuration directories the placement names on that machine, so the
CLI's arguments, environment and token stay the provider's. The backend
implements the placement by the rules above: it holds the Cloud's admission
only while the process starts, since the process is retained, and a failure to
wake the Cloud, to install the CLI or to start it fails the request with its
reason ([What the user sees](#what-the-user-sees)).

The backend's provider family builds the runtime from the same arguments it
builds the provider from, plus the placement, so no code downcasts a provider
to its concrete type. The conversation's runtime and **Test connection** build
a provider's runtime this way whenever the provider's capabilities say that it
needs a process.

On a shared instance each user's requests run on their own Cloud with the
master's account, so the master's token reaches every user's Cloud
([Scope](providers.md#scope)).

## Requests over stream-json

A runtime keeps at most one CLI process, started for one session, model and
thinking setting.

**Starting.** A new process gets:

- no tools of its own, no session persistence, no slash commands and no
  permission prompts;
- the request's model, system prompt and thinking effort;
- stream-json input and output with partial messages, so reasoning and text
  stream as they are generated;
- an environment with the account's token, Demi's configuration home, the CLI's
  updater and its automatic compaction turned off (Demi compacts the
  conversation itself), an MCP tool-output limit of one million tokens so the
  CLI does not cut a result Demi sends, and `CLAUDECODE` unset so the CLI never
  treats itself as running inside another Claude Code session.

When the request offers tools, the provider first sends the CLI an `initialize`
control request that declares one SDK MCP server, and waits for its success.
It then writes the history as the first messages: earlier tool calls and their
results as assistant text, user messages with only the user's real input, and
no earlier reasoning.

**Continuing.** A kept process receives only what the transcript gained since
its last request: the new user messages, steers included. The provider closes
it and starts a new one when the request's model or thinking setting differs,
or when the transcript does not continue what the process saw: it holds fewer
user messages than the process received, or its first user message changed, as
after an edit, a fork or compaction.

**Reading.** Each output line is decoded by its `type` tag
([Reading vendor input](providers.md#reading-vendor-input)). Partial messages
become reasoning and text events. The model's `tool_use` blocks become tool
calls, named without the `mcp__<server>__` prefix the CLI gives tools it
reaches over MCP. The `result` line ends the request: the run ends with a
response carrying the usage of the CLI's last API call (the result's own usage
when it lists no calls), or with an error when the CLI reports one, shown as
the CLI worded it. A line that fails to decode, or a `tool_use` block without an
ID or a name, fails the run with that line as the failure record. Lines that
report rate limits feed the account's quota
([Vendor quota](usage-and-quota.md#vendor-quota)).

## The SDK MCP channel

The CLI can call tools that the process driving it serves over MCP, the Model
Context Protocol, carried inside stream-json control messages instead of a
separate connection. Demi serves each request's tools this way. A request from
the CLI arrives on its standard output as a control request that wraps one MCP
message; Demi's answer goes back on standard input as a control response that
wraps the MCP reply.

Demi's side of the channel is an MCP server built with rmcp, the official Rust
MCP SDK, because the protocol's types, the `initialize` handshake and version
negotiation are the library's job. The run drives it: the run passes each MCP
message it reads from the CLI to the server through an in-memory transport and
writes the server's replies back to the CLI. The server uses rmcp's local mode,
because its state lives on the user's shard, one thread. rmcp handles each
request on its own task, so `ping` and `tools/list` are answered while a
`tools/call` waits for its result.

| MCP request | Demi's answer |
|---|---|
| `tools/list` | The current request's tools: name, description and input schema |
| `tools/call` | The result of that tool call, once the agent has it ([Tool-call batches](#tool-call-batches)) |
| The handshake, `ping`, anything else | rmcp's own answer; a method Demi does not serve is an error |

A `tools/call` names the model's original tool-use ID in its `_meta` field, as
`claudecode/toolUseId`, and Demi matches calls and results by that ID; a call
without one gets a new unique ID. A result goes back as MCP content: text,
images as base64 with their media type, and a video as text naming its media
type. A failed tool sets the result's error flag.

## Tool-call batches

Claude Code can emit several `tool_use` blocks in one assistant message, but
its SDK MCP channel exposes only the first `tools/call`: a later call may stay
blocked until the earlier one has its result. If Demi handled each call as it
arrived, the agent would see one tool call per run and mistake the delayed
calls for separate model turns.

The provider therefore collects the assistant message's original `tool_use`
blocks until `message_stop` and yields the whole batch, with the original
tool-use IDs, before answering any call. The run ends there and keeps the
process; the CLI waits on its open call. A `tools/call` that arrives before the
model's message was streamed is yielded alone, as a batch of one.

The next run starts by delivering the batch's results. Every call of the held
batch must have a result; a missing one fails the run and closes the process.
The provider answers the calls already open, stores the other results, and
writes any user messages the transcript gained, such as a steer. It then
continues reading the CLI's output: each later `tools/call` of the batch is
answered at once from its stored result, and the model goes on in the same
process.

This keeps one CLI process and one MCP session; there is no proxy process and
no second MCP transport.

## Process lifetime

The process is kept between turns ([Where it runs](#where-it-runs) says what
that means for the Cloud). A run takes the process while it runs and hands it
back to the runtime only where the conversation can continue in it; every other
exit ends it.

| Situation | What happens | The process |
|---|---|---|
| The `result` line | The run ends with the response, or with the CLI's error | Kept |
| `message_stop` after tool uses | The batch is yielded and the run ends | Kept |
| The CLI's output ends | The run fails if stored results were never asked for, or if the process exited with a nonzero status and the run was not cancelled; the message is the tail of standard error, or "Claude Code exited with code N" | Gone |
| A malformed line or tool use | The run fails with the line as its record | Closed |
| The run is cancelled | The run ends without an event | Closed |
| The turn drops the run's stream | — | Killed by the Host |
| The runtime is closed: the session is disposed, or the backend builds the session a new runtime | — | Closed |

Closing sends SIGTERM, waits up to five seconds for the process to exit, then
sends SIGKILL and waits for it.

A runtime stands for one account. Selecting another account builds a new
provider, the session's next request builds a new runtime from it and closes
the previous one, and the new process starts with the newly active account's
token ([An account is the unit](providers.md#an-account-is-the-unit)).

**Diagnostics.** The provider drains the CLI's standard error continuously and
keeps its last 64 KiB, which become the failure message when the process exits
with a nonzero status and no result. Each run has a `tracing` span naming the
provider, the session and the request. The raw stream-json exchange is written
only to the `tracing` target `demi::provider::claude_code::wire`, at trace
level, which is off by default. Turned on, it records every line in both
directions, prompts and tool output included; the line that records a spawn
leaves out the environment, so the token is never written.

## What the user sees

Installing is not an event of the conversation. A request whose Cloud has no
usable CLI installs one and then proceeds, under the same **Requesting** the
request would show anyway; an update installs with nothing shown at all.

A failure is always shown, with its reason and the version: the release could
not be read, the download failed or did not match its digest, the platform has
no build, the disk is full, the executable did not start. In a conversation it
is that request's error, and the next request tries again. For account work it
is the result of the action that asked, and an install that failed after an
account was added does not undo the account: the settings page shows the
account as added and the CLI as not installed, with **Install** to try again.
Backend shutdown cancels an install in progress; the next request, or
**Install**, starts it again.

Settings show, for a Claude Code entry, the vendor's newest version and the
version the user's Cloud has when it can be asked now. Updates are automatic;
**Check for updates** refreshes the release information, and **Install** retries
a failed installation. The routes are listed in
[Web API](../product/web-api.md#model-configuration-and-provider-inspection).

## Acceptance

- A scripted CLI that emits two tool uses in one message has both reach the
  agent in one run before any MCP answer, and its second `tools/call`, sent
  only after the first was answered, is answered at once from the stored
  result.
- A second request continues in the same process and writes only the new user
  message; after the first message is edited, a new process starts and replays
  the transcript.
- Cancelling a run closes the process and ends the run without an event; a
  process that ignores SIGTERM is killed after five seconds.
- After another account is selected, the next request starts a new process
  with that account's token.
- A conversation on a paired device infers through the user's Cloud, and the
  account's token never reaches the paired device.
- A real CLI, driven by hand during acceptance, completes a two-tool batch and
  its continuation, and its transcript shows every tool call arriving as an SDK
  MCP `tools/call`.
