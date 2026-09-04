# Demi Next: The Runner

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Implemented on tinyjs (M9 step 3): connection, Host RPC, jobs and the tee, the relay, the manifest cache; the M1, M4 and M6 suites run on it |
| Scope | The program on every execution target: identity and connection, Host RPC, the job table, the tee, the local relay, the wire rules |

## Role

The runner turns a machine into an **execution target**. It is not an agent
process: it contains no LLM logic and runs no AgentSession. It is a remote
implementation of Demi's `Host` contract (`packages/shell/src/host.ts`) —
this machine's filesystem and process execution over Demi's own protocol —
plus the job table that runs the agent's commands in real bash, the tee that
keeps their full output on this machine, and the relay through which root
commands (`commands.md`) reach the backend. Even the Claude Code CLI reaches the runner
as an ordinary remote `spawn`; the runner has no claude-specific code.

It is JS running on tinyjs (`tinyjs.md`), so its protocol schemas are the
same zod objects the backend uses. Design principle: no speculative
constraints — no workspace restrictions, no local policy layer, no
configuration beyond what the connection needs.

## Responsibilities

1. **Device identity and connection.** One outbound WebSocket to the
   backend's public URL, exponential backoff on reconnect, no inbound ports.
   A user host pairs through the claim flow below; a managed host starts
   with a pre-issued token read from the kernel command line
   (`managed-hosts.md`). Device online status in the web UI is this socket's
   state.
2. **Serving the Host contract.** Filesystem operations and process spawns
   from the backend, scoped per request to a working directory the
   conversation names. Any existing directory is a valid workspace. Binary
   resolution is a device fact: a spawn naming no `PATH`/`HOME` resolves
   against the device's own.
3. **Running commands.** A tool call becomes one job: real `bash -c` on this
   machine with the conversation's cwd and env, the conversation and shell
   ids in the environment, the bounded output view streamed to the backend,
   the full output teed to an output file here.
4. **Relaying root commands.** A Unix domain socket for command-mode tinyjs
   processes: manifest cache misses, and `rpc` command invocations
   forwarded to the backend attributed to the invoking conversation, their
   stdin and stdout carried as pipes — HTTP streams brokered by the
   backend, never bytes on the runner socket (§ Pipes). The runner also
   maintains the root-command symlinks in `PATH` from the manifest.

Non-responsibilities: user authentication; credentials of any kind (the
Claude Code CLI it spawns receives its token as process env from the
backend-side provider, nothing is persisted here); transcript or checkpoint
storage; command implementations (`runtime` modules run in command-mode
processes, `rpc` commands in the backend); provider logic.

## Process shape and local state

The tinyjs binary in runner mode: `demi-runner run [--backend <url>]`. First
start prints the claim token and waits; later starts authenticate with the
persisted device token. On a managed host the runner is PID 1 and performs
init duties (`managed-hosts.md` § Lifecycle): the same binary, told by its
pid; the token comes off the kernel command line and stays in memory, the
state directory is `/var/lib/demi` on the ephemeral upper, every job and
spawn runs as the guest user, and the hello reports that user's identity.

```
~/.demi/                 (`DEMI_HOME` names another place; every job and command-mode process inherits it)
  runner.json            backend URL, device id
  runner-token           device token (0600)
  runner.sock            the local relay (0600)
  commands/<hash>/       manifest cache: manifest.json + modules/<hash>.mjs, by manifest hash
  commands/current       → the cached manifest command mode reads
  bin/<root>             → the packed binary, one per root in the manifest; first in every job's PATH
  output/<jobId>/        stdout.txt, stderr.txt: a job's full output, written by the tee
  store/                 Host.store of the machine's Host
```

`demi-runner run [--backend <url>]` reads `DEMI_RUNNER_NAME` for the
device name (default the hostname) and `DEMI_RUNNER_RECONNECT_MS` for the
first reconnect delay (tests shorten it).

Dependency footprint: `@demicodes/runner-protocol`, `@demicodes/shell`
(the Host contract and the command system), `@demicodes/command-loader`
(cache and relay only), `@demicodes/utils`. No agent, coding-agent or
provider packages. The package's directories are its modules
(`package-boundaries.md`): `machine/` — this machine as the runner sees
it, the `Host` contract over tinyjs's primitives plus the links and the
tee, the only code that imports `tinyjs:*`; `serve/` — the runner's end
of the protocol, `HostRpcServer` and `JobTable`; `relay/` — the UDS relay;
and the two entry modes with their shared state at the root.

Nothing in the backend executes commands in-process; every real machine
is reached through a runner.

Three packages carry a machine's Host, one per place: the wire itself is
`@demicodes/runner-protocol` (schemas, messages, codec — both ends depend
on it, it depends on neither); the backend's end is `@demicodes/host-remote`
(`RemoteHost`, a `Host` that forwards each call over the socket, and
`RemoteShellEnvironment`, the shell of a real host over jobs) — one of the
two Hosts the backend injects into the agent, beside `@demicodes/host-virtual`;
the machine's end is the runner's `machine/` layer, which performs what
`RemoteHost` was asked. The agent never holds the machine layer; a
machine is always reached through `host-remote`.

## Connection model

One outbound WebSocket. Frames are binary **MessagePack** (`Uint8Array` as
bin, `Date` as the timestamp extension, `undefined` as nil), so bytes and
times are native wire types; a text frame is malformed and closes the
socket. The schemas are `zod` in `@demicodes/runner-protocol`, shared
verbatim by both ends; the codec is the carrier's — `@msgpack/msgpack` on
the backend, `tinyjs:bytes` on the runner — handed to `createRunnerWire`,
and the two are held to the same bytes by a test that round-trips frames
through both.

Pairing, end to end — two user steps (run the command, paste the code):

```
① First start (unclaimed)
   runner ──hello { deviceToken: absent, runner: {name, platform, version, identity} }──▶ backend
   runner ◀──claim_pending { claimToken }──────────────────────────────────────────────── backend
   prints the pairing code (128-bit, Crockford base32, grouped); socket stays open

② The user pastes the code on the devices page
   browser ──POST /api/devices/claim──▶ backend binds device to user, mints deviceToken
   runner ◀──claimed { deviceToken }── backend           runner persists it; device shows online

③ Every later start
   runner ──hello { deviceToken, runner }──▶ backend
   runner ◀──hello_ok { deviceId }──────── backend       from here: Host RPC, jobs, ping/pong
```

Failure branch: a refused hello answers `hello_error { code, reason }` and
closes the socket. `unknown_device`, `revoked` and `unsupported_protocol`
cannot change without operator action: the runner stops and prompts to
pair again. **A token holds at most one live connection**: a second
`hello` carrying a token already connected is refused with
`already_connected` and logged by the backend; that one outcome the runner
retries with its backoff, because the live connection may be a half-open
socket the backend has not timed out yet, and the retry succeeds once the
old socket is gone.

### Messages

Handshake and liveness:

| Direction | Message | Purpose |
|---|---|---|
| r → b | `hello { deviceToken?, runner: { name, platform, version, identity, managed? } }` | authenticate; `identity` because `HostIdentity` is read synchronously at shell creation; `managed` marks a guest booted as init — without a token it is refused, never paired (`managed-hosts.md` § Joining) |
| b → r | `hello_ok { deviceId }` / `claim_pending { claimToken }` / `claimed { deviceToken }` / `hello_error { code, reason }` | outcomes; `code` is `unsupported_protocol`, `unknown_device`, `already_connected`, `revoked` or `internal` |
| b → r | `ping` | liveness, backend-driven interval |
| r → b | `pong { jobs }` | liveness plus the count of running jobs, which the idle rule reads (`managed-hosts.md`) |
| b → r | `sync { id }` | before a hibernate: flush the home to disk |
| r → b | `sync_done { id, untouched }` | flushed; `untouched` when nothing wrote to the home since boot, so the save can skip the upload (`managed-hosts.md` § Home persistence) |
| r → b | `home_grow { bytes }` | the home nears its cap: grow its image to this total size |
| b → r | `home_grown { bytes }` | the image is that large; the runner grows the filesystem into it |

Host RPC — the wire form of the `Host` contract's `fs` and `process`
facets (`Host.store` never crosses this protocol):

| Direction | Message | Purpose |
|---|---|---|
| b → r | one message per fs operation, `fs_<method> { id, …params }` — `fs_stat { id, path, cwd? }`, `fs_writeFile { id, path, data, cwd?, createParents? }`, … — a union over the `HostFileSystem` method set, each with its parameters named | typed per op, so argument shapes are checked at decode |
| r → b | `fs_ok { id, op, result }` / `fs_error { id, code?, message }` | the result typed by `op` (bytes, a stat, names or dirents, a path, `null`), or an error carrying the errno code when there is one |
| b → r | `spawn { spawnId, command, args, cwd, env, stdin? }` | start a raw process (the Claude Code CLI, the directory browser) |
| r → b | `spawn_output { spawnId, stream, bytes }` / `spawn_exit { spawnId, code, signal? }` | streamed stdio, exit |
| b → r | `spawn_stdin { spawnId, bytes }` / `spawn_stdin_end` / `spawn_kill { spawnId, signal }` | input and termination |

Jobs — the agent's commands:

| Direction | Message | Purpose |
|---|---|---|
| b → r | `job_start { jobId, script, cwd, env, background, stdin?, stdout? }` | run `bash -c script`; `env` carries the conversation and shell ids; `stdin` / `stdout` are pipes the job's fd 0 / fd 1 are attached to when another process's ends are on the far side (§ Pipes) |
| r → b | `job_output { jobId, stream, bytes }` | live output while the job runs, up to the view budget per stream, then silence |
| r → b | `job_exit { jobId, code, signal?, cwd, output: { stdoutPath, stderrPath, stdoutBytes, stderrBytes, stdoutTail, stderrTail } }` | exit, the working directory the script ended in, where the full output lives on this machine, and the last bytes of each stream |
| b → r | `job_stdin { jobId, bytes }` / `job_kill { jobId, signal }` | interactive input (`shell_write`), termination |
| r → b | `job_list` reply and `pong.jobs` | the job table is the runner's; the backend reads it |

Relay and outputs:

| Direction | Message | Purpose |
|---|---|---|
| r → b | `rpc_call { callId, conversationId, shellId, root, command, args, cwd, env, stdin: boolean }` | an `rpc` command of some root invoked on this target; `stdin` says whether the process has a pipe on fd 0 |
| b → r | `rpc_pipes { callId, stdin?, stdout }` | the call's pipe ends, sent before anything else for the call: the runner `PUT`s the process's pipe into `stdin` and `GET`s `stdout` into the process (§ Pipes) |
| r → b | `rpc_stdin { callId, bytes }` / `rpc_stdin_end { callId }` | the live stdin the command is steered with (`shell_write`); never the pipe |
| b → r | `rpc_output { callId, bytes }` / `rpc_exit { callId, exitCode }` | the stderr view and the exit code back to the command-mode process; `rpc_exit` follows the stdout pipe's drain |
| b → r | `manifest { manifest }` on connect and on change | the command manifest the runner caches for the CLI |
| r → b | `pipe_done { pipeId, ok, error? }` | this runner's end of a pipe closed: its HTTP exchange completed, or why it did not |

The `fs_*` op set mirrors `HostFileSystem` one to one and is one table in
the protocol package (`fsOps`: parameters and result schema per method),
from which the request union, the reply union and the TS types derive.

## Jobs and the tee

The job table is the runner's. A job is one `bash -c` process with the
shell's cwd and env; a background job (`… &`) is the same with its handle
kept after the tool call returns. The runner owns process groups, reaps
children, and reports the count of running jobs in every `pong`.

**What carries from one job to the next** is the working directory and
nothing else. The runner wraps the script so that an `EXIT` trap records
`pwd` when bash ends — an explicit `exit` inside the script included — and
`job_exit.cwd` carries it back; the backend's shell state takes it for the
next `job_start`. Environment variables, functions, aliases and shell
options do not carry: a fresh process per job is the norm across coding
agents (Claude Code, Codex, Gemini, Aider), and only Claude Code carries
the directory, which is the one thing a model reaches for.

A job's environment is what the backend named — the shell's exports,
`DEMI_SESSION_ID` and `DEMI_SHELL_ID` — with the device's `PATH` and `HOME`
filled in when absent, `bin/` first in `PATH`, and three entries of the
runner's own: `DEMI_HOME`, `DEMI_JOB_CWD_FILE` (where the `EXIT` trap
writes `pwd`) and `DEMI_JOB_STDIN_FD` (the descriptor the prelude
duplicated the job's stdin onto with `exec 199<&0`, so a command-mode
process can tell the job's live stdin from a redirection by `fdNode`,
`tinyjs.md`).

**The view is the model's view.** What crosses the wire, what the backend
records, and what the browser shows are one and the same: the bytes the
model sees. The **tee** is a tinyjs primitive (`tinyjs.md`): each job's
stdout and stderr are written in full to output files under the target's
`commandOutputDir`. While the job runs, `job_output` streams the first
bytes of each stream up to the view budget (`JOB_VIEW_BYTES`, a protocol
constant of the model-window class, 32 KB), so a running command can be
polled; at exit `job_exit` carries the last bytes of each stream, read
from the output file, so the model's window is the true tail and not the
tail of a head-limited view. The output path in `job_exit` is what the
model sees as "full output at …"; anything beyond the view it reads with
ordinary commands (`grep`, `sed -n`, `tail`) on the target, where the file
is. Nothing fetches full output back to the backend, and nothing shows the
browser more than the model saw.

`cmd1 | cmd2` is an OS pipe on the target. Zero bytes of it cross the wire.

The word *artifact* is not used for these files: in agent products it
names the agent's deliverables, and Demi keeps it free for that.

## The local relay

`~/.demi/runner.sock` (mode 0600) accepts connections from command-mode
tinyjs processes. Its frames are MessagePack behind a 32-bit big-endian
length, one request per connection: `manifest` (answered from the cache;
a process asks when `commands/current` is missing) or `rpc { agentSessionId,
shellId, root, path, argv, args, json, cwd, env, stdin }` — the parsed
invocation and whether fd 0 is a pipe — followed by the pipe itself as
`pipe { bytes }` frames ending in `pipe_end`, and by `stdin { bytes }`
frames for the live stdin and `stdin_end`; back come `output { stream,
bytes }` frames and `exit { exitCode }`, or `error { message }`. The
runner forwards `rpc` on its authenticated socket as `rpc_call` /
`rpc_stdin` / `rpc_stdin_end`, streams the `pipe` frames into the `PUT`
that the backend's `rpc_pipes` names and the `GET` body back as `output`
frames, and relays `rpc_output` / `rpc_exit`. The UDS is one ordered
connection per request, so the pipe rides it as the byte stream it is;
the runner holds no more of it than an HTTP body in flight. The backend
runs the leaf against the tree of the conversation `agentSessionId` names.
The command-mode process never holds a credential. Attribution is by the
ids the backend put into the job's environment; a process on the same
machine that forges them can only reach the conversations already
executing here, which it could already read and modify.

## Pipes

A **pipe** is a finite byte stream with two ends. Every `rpc` command's
stdin and its stdout is one, and so is a job's stdin or stdout when `demi
host shell --id` attaches it to a process on another host. A pipe whose
ends are in different processes is carried as one HTTP exchange brokered
by the backend — the producing end `PUT`s, the consuming end `GET`s, the
backend pipes one body into the other and holds nothing — never as a
byte stream on a runner socket; the socket carries only the control
frames that name the ends. A pipe both of whose ends are in the backend
is one `AsyncIterable` handed from producer to consumer. One primitive,
one broker: stdin and stdout are the same thing pointed in opposite
directions, and a copy from host to host is the same thing with both
ends on devices.

An end is one of:

| End | Where | How it produces or consumes |
|---|---|---|
| a command-mode process's fd 0 / fd 1 | a device | through the local relay: the pipe's frames on the UDS, the runner doing the HTTP |
| a job's fd 0 / fd 1 | a device | the runner attaches the HTTP body to the process it spawns for `job_start` |
| an `rpc` handler's `ctx.stdin` / `ctx.io.stdout` | the backend | an `AsyncIterable<Uint8Array>` / a writer |

The backend mints a pipe (`PipeBroker.open(source, sink)`, each end
either `{ deviceId }` or in-process) and tells each device end where to
connect: an `rpc` call's ends in `rpc_pipes`, a job's in `job_start`. On
the wire a pipe is `{ id, url }`; `url` is origin-relative
(`/api/pipes/<id>`), resolved against the runner's backend URL and sent
with `Authorization: Bearer <device token>`.

The three shapes are one picture with different ends.

An `rpc` command on a device — the handler runs in the backend:

```
A's model:   demi file write notes.md < big.txt

  A (command-mode process ─UDS─ runner)                backend
  rpc { …, stdin: true } ─────────────────────────────►  rpc_call { …, stdin: true }
                                                          P_in  minted  A → backend (sink: the handler's ctx.stdin)
                                                          P_out minted  backend → A (source: the handler's stdout)
  rpc_pipes { stdin: P_in, stdout: P_out } ◄──────────────
  pipe frames ─UDS─► runner ─ PUT /api/pipes/P_in ──────►  ═══ body → ctx.stdin, as it arrives ═══
  output frames ◄─UDS─ runner ◄─ GET /api/pipes/P_out ───  ═══ stdout writes → body, as they happen ═══
  rpc_output (the stderr view) ◄──────────────────────────
  rpc_exit ◄──────────────────────────────────────────────  after P_out drained
```

`demi host shell --id` — the caller's pipe ends attached to a job on
another host, both directions streaming while the job runs:

```
A's model:   tar c . | demi host shell --id B "tar x -C /work"        (B's job reads A's pipe)
             demi host shell --id B "tar c -C /work ." | tar x        (A's pipe reads B's job)

  A (caller)                     backend                              B (the job)
  rpc_call { stdin: true } ────►  P_in  minted  A → B
                                  P_out minted  B → A
  rpc_pipes ◄────────────────────  { stdin: P_in, stdout: P_out }
                                  job_start { script, stdin: P_in, stdout: P_out } ────►  spawn bash -c script
  PUT P_in ══════════════════════► piped, held in flight only ═════════════════════════► GET P_in → fd 0
  GET P_out ◄═════════════════════ piped, held in flight only ◄═════════════════════════ PUT P_out ← fd 1
                                                                                          (teed to output/<job>/stdout.txt as well)
  rpc_output ◄────────────────────  the job's stderr view ◄──────────────────────────────  job_output
  rpc_exit ◄──────────────────────  after P_out drained ◄────────────────────────────────  job_exit
```

A hostless caller — the backend's own tinybash runs the command, so its
end is in-process:

```
hostless model:   demi host shell --id B "cat /work/notes.md" > notes.md

  backend                                                             B
  P_out minted  B → backend (sink: the pipeline's next stage)
  job_start { script, stdout: P_out } ────────────────────────────►  spawn; PUT P_out ← fd 1
  ═══ body → the redirection, into the conversation's store ═══ ◄═══
```

- **Streaming, both ways.** A job's stdout reaches the far end as the job
  writes it; the tee still writes the output file on that machine, for
  the model's view and the transcript, and the pipe neither waits for it
  nor reads it back. An `rpc` handler reads its stdin as it arrives and
  its writes leave as they happen.
- **A pipe both ends know.** A command-mode process whose fd 0 is the
  job's live stdin (`DEMI_JOB_STDIN_FD`) declares `stdin: false` and no
  stdin pipe is minted. Every call has a stdout pipe; one that never
  writes closes it empty. A job gets a pipe only for the fd `job_start`
  names.
- **Backpressure is HTTP's.** A slow consumer stalls the producer through
  the broker; a runner holds no more than the body in flight; the backend
  holds no pipe bytes at all. Nothing is sized by the payload: a
  gigabyte crosses like a kilobyte, slower.
- **Authorization and lifetime.** The `PUT` is accepted only from the
  pipe's source device and the `GET` only from its sink device; an
  in-process end needs neither. Each id serves one exchange; an end that
  never arrives times the pipe out; a device disconnecting fails the pipes
  it is party to, and the command or job at the other end sees an
  ordinary error. The `PUT` completes only after the sink drained the
  body, so `pipe_done` is the end of the copy, and `rpc_exit` follows
  the stdout pipe's drain so the process has written everything before it
  exits with the code.
- **What is not a pipe.** stderr is a view: it goes to the person and the
  model, streams on the socket as `rpc_output` / `job_output`, and is
  what they see, not a data carrier. Live stdin (`shell_write`) is
  interactive input and rides `rpc_stdin` / `job_stdin`. Both are small
  by nature; the wire rules keep them so.

## Wire rules

**Protocols carry references, never bulk bytes.** File reads and writes
happen on the target (`runtime` modules); the tee keeps full output on the
target; media reaches the browser as `source.ref` plus a `GET`
(`backend.md`); every pipe between processes — an `rpc` command's stdin
and stdout, a cross-host job's — is an HTTP stream brokered by the backend
(§ Pipes). The runner socket is therefore small-message and
latency-bound, and the backend never holds a command's input or output in
memory.

**Recorded risk — control-message priority.** WebSocket is one ordered
stream: a `ping` or a user `abort` queued behind a large frame waits for
it — about 100 ms per MB at 10 MB/s. With job views in the 32 KB class the
largest frames are `spawn_output` chunks of raw spawns (the Claude Code
CLI) and `transcript_patch`. A channel layer beneath the carrier
(stream-id framing, ≤16 KB chunks, strict control-first priority,
`bufferedAmount` backpressure) removes it. **Deferred** on the
precondition that those stay small; to be re-examined if a large source
appears.

## Disconnect semantics

When the socket drops, running jobs and spawns on the runner are killed and
pending calls fail; the backend surfaces the failure into the affected turns
as ordinary tool errors — the session lives in the backend and is not lost.
On reconnect the device is simply online again. On a managed host the
runner is PID 1, so a runner crash is a VM death and the next tool call
takes the wake path (`managed-hosts.md`).

## Trust model

- The device token authorizes "this device executes for this user" and
  carries no credential of any other kind. The runner is never issued one.
- The runner trusts the backend for identity: operations arriving on its
  socket belong to the claiming user, and the runner performs them in
  whatever directory they name. Multi-user sharing of one runner is out of
  scope.
- The runner is thin and frozen — fs, spawn, jobs, tee, relay, claim — so
  that the component hardest to update on user devices contains the
  least-changing code. It gains no power in the backend → device direction
  from the jobs or the relay.

## Facts the contract rests on

- The `Host` contract is wire-ready: every `HostFileSystem` method is async
  with plain-data inputs and outputs; spawn params and exit are plain data;
  `kill` takes a string signal. The two non-wire-safe spots have their
  answers: `HostCwd` fd anchors → the contract's own `createLogicalHostCwd`
  fallback on the proxy side; sync `Host.identity` → carried in `hello`.
- Multi-Host-per-session is an existing, tested mechanism
  (`AgentHarness.host(action metadata)` with per-Host environment reuse and
  cross-Host shell-handle ownership checks); target switching builds on it.
- Spawn streams: the runner pushes one ordered, stream-tagged chunk
  sequence per spawn or job; the proxy derives `stdout`/`stderr`/merged
  views from it without double-counting.
