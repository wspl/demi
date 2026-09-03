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
   forwarded to the backend attributed to the invoking conversation. The
   runner also maintains the root-command symlinks in `PATH` from the
   manifest.

Non-responsibilities: user authentication; credentials of any kind (the
Claude Code CLI it spawns receives its token as process env from the
backend-side provider, nothing is persisted here); transcript or checkpoint
storage; command implementations (`runtime` modules run in command-mode
processes, `rpc` commands in the backend); provider logic.

## Process shape and local state

The tinyjs binary in runner mode: `demi-runner run [--backend <url>]`. First
start prints the claim token and waits; later starts authenticate with the
persisted device token. On a managed host the runner is PID 1 and performs
init duties (`managed-hosts.md`).

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
| b → r | `job_start { jobId, script, cwd, env, background }` | run `bash -c script`; `env` carries the conversation and shell ids |
| r → b | `job_output { jobId, stream, bytes }` | live output while the job runs, up to the view budget per stream, then silence |
| r → b | `job_exit { jobId, code, signal?, cwd, output: { stdoutPath, stderrPath, stdoutBytes, stderrBytes, stdoutTail, stderrTail } }` | exit, the working directory the script ended in, where the full output lives on this machine, and the last bytes of each stream |
| b → r | `job_stdin { jobId, bytes }` / `job_kill { jobId, signal }` | interactive input (`shell_write`), termination |
| r → b | `job_list` reply and `pong.jobs` | the job table is the runner's; the backend reads it |

Relay and outputs:

| Direction | Message | Purpose |
|---|---|---|
| r → b | `rpc_call { callId, conversationId, shellId, root, command, args, stdin }` streamed | an `rpc` command of some root invoked on this target |
| b → r | `rpc_output { callId, stream, bytes }` / `rpc_exit { callId, exitCode }` | its result back to the command-mode process |
| b → r | `rpc_transfer { callId, url }` | the call's stdout is a brokered transfer: the runner `GET`s it and relays the body, then `rpc_exit` follows |
| b → r | `manifest { manifest }` on connect and on change | the command manifest the runner caches for the CLI |
| b → r | `transfer_send { transferId, path, url }` / `transfer_receive { transferId, path, url }` | a brokered cross-host copy: the source `PUT`s the file at `path`, the destination `GET`s into `path` |
| r → b | `transfer_done { transferId, ok, error? }` | the HTTP exchange ended |

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
invocation with the pipe's bytes — followed by `stdin { bytes }` frames for
the live stdin and `stdin_end`; back come `output { stream, bytes }` frames
and `exit { exitCode }`, or `error { message }`. The runner forwards `rpc`
on its authenticated socket as `rpc_call` / `rpc_stdin` / `rpc_stdin_end`
and relays `rpc_output` / `rpc_exit` back; the backend runs the leaf
against the tree of the conversation `agentSessionId` names. The
command-mode process never holds a credential. Attribution is by the ids
the backend put into the job's environment; a process on the same machine
that forges them can only reach the conversations already executing here,
which it could already read and modify.

## Transfers

A cross-host copy is a file moved by HTTP between two runners with the
backend in the middle, never a byte stream on a runner socket. The unit
is a job's stdout file: `demi host shell --id A <script>` runs `script` as
a job on A (the pipe's bytes as its stdin, its stderr view streaming back
as it runs, its exit code passed through), and at exit the job's full
`stdoutPath` is the file transferred. The backend mints a single-use
transfer id, tells A `transfer_send { transferId, path, url }`, and
delivers the bytes to the caller:

```
B's model:   demi host shell --id A "tar c -C /work ." | tar x

  A (source)                 backend                          B (caller)
  job_start ◄─────────────   the script as a job     ◄──── rpc_call (relayed)
  tee → output/<job>/stdout.txt
  job_exit ───────────────►  transfer minted
  transfer_send ◄─────────   { transferId, path, url }
                              rpc_transfer ─────────────────►  { callId, url }
  PUT /api/transfers/<id> ─►  ═══ piped, held in flight only ═══ ◄─ GET /api/transfers/<id>
                                                                body → relay output → stdout → tar x
  transfer_done ──────────►
                              rpc_exit ─────────────────────►  the job's exit code
```

- A caller on a device (a command-mode process, via the relay) receives
  `rpc_transfer { callId, url }`: its runner `GET`s the URL with the
  device token and writes the body to the process's stdout through the
  relay, before `rpc_exit`. A hostless caller (the backend's own
  tinybash) takes the `PUT` body straight into the command's stdout.
- `transfer_receive { transferId, path, url }` is the symmetric end: the
  destination runner `GET`s into a file. The hostless → managed upgrade
  uses it to place the store's files (`managed-hosts.md`).
- `url` is origin-relative (`/api/transfers/<id>`); a runner resolves it
  against its backend URL and sends `Authorization: Bearer <device
  token>`. The backend accepts the `PUT` only from the transfer's source
  device and the `GET` only from its destination device; each id serves
  one exchange; a side that never arrives times the transfer out; a
  device disconnecting fails what it was party to. The `PUT` completes
  only after the destination drained the body, so `transfer_done` is
  the end of the copy.
- The stdout of `host shell --id` arrives once the remote script has
  exited: the transfer is of a finished file. Its caller is a pipe, not a
  view, so nothing needs it earlier; the job's 32 KB view still streams to
  the backend as the job runs and goes nowhere.

## Wire rules

**Protocols carry references, never bulk bytes.** File reads and writes
happen on the target (`runtime` modules); the tee keeps full output on the
target; media reaches the browser as `source.ref` plus a `GET`
(`backend.md`); bulk transfer is an HTTP stream brokered by the backend.
The runner socket is therefore small-message and latency-bound, and the
backend never holds a command's whole output in memory.

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
