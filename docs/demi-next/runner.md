# Demi Next: The Runner

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (M1/M4 delivered on the Bun build; the shell build with jobs, tee and the relay lands in M9) |
| Scope | The program on every execution target: identity and connection, Host RPC, the job table, the tee, the local relay, the wire rules |

## Role

The runner turns a machine into an **execution target**. It is not an agent
process: it contains no LLM logic and runs no AgentSession. It is a remote
implementation of Demi's `Host` contract (`packages/shell/src/host.ts`) —
this machine's filesystem and process execution over Demi's own protocol —
plus the job table that runs the agent's commands in real bash, the tee that
keeps their full output on this machine, and the relay through which the
`demi` CLI reaches the backend. Even the Claude Code CLI reaches the runner
as an ordinary remote `spawn`; the runner has no claude-specific code.

It is JS running on the shell (`shell.md`), so its protocol schemas are the
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
   the full output teed to an artifact file here.
4. **Relaying `demi`.** A Unix domain socket for the `demi` CLI: manifest
   cache misses, and `rpc` command invocations forwarded to the backend
   attributed to the invoking conversation.

Non-responsibilities: user authentication; credentials of any kind (the
Claude Code CLI it spawns receives its token as process env from the
backend-side provider, nothing is persisted here); transcript or checkpoint
storage; command implementations (`runtime` modules run in the `demi` CLI,
`rpc` commands in the backend); provider logic.

## Process shape and local state

The shell binary in runner mode: `demi-runner run [--backend <url>]`. First
start prints the claim token and waits; later starts authenticate with the
persisted device token. On a managed host the runner is PID 1 and performs
init duties (`managed-hosts.md`).

```
~/.demi/
  runner.json            backend URL, device id
  runner-token           device token (0600)
  runner.sock            the local relay (0600)
  commands/<hash>/       manifest cache: tree + modules, by manifest hash
```

Dependency footprint: `@demicodes/host-shell`, `@demicodes/runner-protocol`,
`@demicodes/command-loader` (cache and relay only), `@demicodes/utils`. No
agent, coding-agent or provider packages.

The backend's own machine is a target the same way: in self-host
deployments the backend starts a runner process on its machine, registered
as a device without a claim; nothing in the backend executes commands
in-process.

## Connection model

One outbound WebSocket. Frames are **MessagePack**, so `Uint8Array` and
`Date` are native wire types; the schemas are `zod` in
`@demicodes/runner-protocol`, shared verbatim by both ends.

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

Failure branch: a revoked or invalid token answers `hello_error { reason }`
and the runner prompts to pair again. **A token holds at most one live
connection**: a second `hello` carrying a token already connected is
rejected and logged.

### Messages

Handshake and liveness:

| Direction | Message | Purpose |
|---|---|---|
| r → b | `hello { deviceToken?, runner: { name, platform, version, identity } }` | authenticate; `identity` because `HostIdentity` is read synchronously at shell creation |
| b → r | `hello_ok { deviceId }` / `claim_pending { claimToken }` / `claimed { deviceToken }` / `hello_error { reason }` | outcomes |
| b → r | `ping` | liveness, backend-driven interval |
| r → b | `pong { jobs }` | liveness plus the count of running jobs, which the idle rule reads (`managed-hosts.md`) |

Host RPC — the wire form of the `Host` contract's `fs` and `process`
facets (`Host.store` never crosses this protocol):

| Direction | Message | Purpose |
|---|---|---|
| b → r | one message per fs operation, e.g. `fs_stat { id, path }`, `fs_read { id, path, offset?, length? }` — a discriminated union over the `HostFileSystem` method set | typed per op, so argument and result shapes are checked |
| r → b | `fs_ok { id, op, result }` / `fs_error { id, code, message }` | result or a typed error carrying the errno code |
| b → r | `spawn { spawnId, command, args, cwd, env, stdin? }` | start a raw process (the Claude Code CLI, the directory browser) |
| r → b | `spawn_output { spawnId, stream, bytes }` / `spawn_exit { spawnId, code, signal? }` | streamed stdio, exit |
| b → r | `spawn_stdin { spawnId, bytes }` / `spawn_stdin_end` / `spawn_kill { spawnId, signal }` | input and termination |

Jobs — the agent's commands:

| Direction | Message | Purpose |
|---|---|---|
| b → r | `job_start { jobId, script, cwd, env, background, viewLimit }` | run `bash -c script`; `env` carries the conversation and shell ids |
| r → b | `job_output { jobId, stream, bytes }` | the bounded view; stops at `viewLimit` per stream |
| r → b | `job_exit { jobId, code, signal?, artifact: { stdoutPath, stderrPath, stdoutBytes, stderrBytes } }` | exit plus where the full output lives on this machine |
| b → r | `job_stdin { jobId, bytes }` / `job_kill { jobId, signal }` | interactive input (`shell_write`), termination |
| r → b | `job_list` reply and `pong.jobs` | the job table is the runner's; the backend reads it |

Relay and artifacts:

| Direction | Message | Purpose |
|---|---|---|
| r → b | `rpc_call { callId, conversationId, shellId, command, args, stdin }` streamed | a `demi` `rpc` command invoked on this target |
| b → r | `rpc_output { callId, stream, bytes }` / `rpc_exit { callId, code }` | its result back to the CLI |
| b → r | `manifest { hash, tree, modules }` on connect and on change | the command manifest the runner caches for the CLI |
| b → r | `artifact_upload { path, uploadUrl }` | the backend wants the full bytes of an artifact: the runner `PUT`s the file to the one-shot URL over HTTP |
| b → r | `transfer_send { path, uploadUrl }` / `transfer_receive { path, downloadUrl }` | a brokered cross-host copy: the source `PUT`s, the destination `GET`s, the backend pipes the two |

The exact `fs_*` op set mirrors `HostFileSystem` and is fixed in the
protocol package.

## Jobs and the tee

The job table is the runner's. A job is one `bash -c` process with the
conversation's cwd and env; a background job (`… &`) is the same with its
handle kept after the tool call returns. The runner owns process groups,
reaps children, and reports the count of running jobs in every `pong`.

The **tee** is a shell primitive (`shell.md`): each job's stdout and
stderr are written in full to artifact files under the target's
`commandArtifactsDir`, and only the first `viewLimit` bytes of each stream
travel to the backend. The backend's tool result is built from the view;
the artifact path in `job_exit` is what the model sees as "full output
available at …". Artifacts stay on the target and are not uploaded at
hibernation; the backend fetches them by reference over HTTP when a user
opens a past command's full output while the host is online
(`sessions-and-targets.md`).

`cmd1 | cmd2` is an OS pipe on the target. Zero bytes of it cross the wire.

## The local relay

`~/.demi/runner.sock` accepts connections only from the `demi` CLI. Two
request types: `manifest?` (answered from the cache or, on a miss, fetched
over the socket) and `rpc { conversationId, shellId, command, args }` with
stdin streamed in and stdout/stderr streamed out. The runner forwards `rpc`
on its authenticated socket; the CLI never holds a credential. Attribution
is by the ids the backend put into the job's environment; a process on the
same machine that forges them can only reach the conversations already
executing here, which it could already read and modify.

## Wire rules

**Protocols carry references, never bulk bytes.** File reads and writes
happen on the target (`runtime` modules); the tee keeps full output on the
target; media reaches the browser as `source.ref` plus a `GET`
(`backend.md`); bulk transfer is an HTTP stream brokered by the backend.
The runner socket is therefore small-message and latency-bound, and the
backend never holds a command's whole output in memory.

**Recorded risk — control-message priority.** WebSocket is one ordered
stream: a `ping` queued behind a 1 MB view chunk, or a user `abort` queued
behind a large `transcript_patch`, waits for it — about 100 ms at 10 MB/s,
about 1 s at 1 MB/s. A channel layer beneath the carrier (stream-id framing,
≤16 KB chunks, strict control-first priority, `bufferedAmount`
backpressure) removes it. **Deferred** on the precondition that views stay
at the 1 MB class and transcript patches stay small; to be re-examined if a
large patch source appears.

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
