# Demi Next: Roadmap

| | |
|---|---|
| Date | 2026-09-02 |
| Status | M0–M9 delivered |
| Scope | Milestone order, contents and acceptance for the records in this directory |

Ordering principles: **lowest dependency first** — nothing is built before
the layer it runs on exists, so nothing is judged usable only after later
work; the riskiest long-lived contracts as early as their dependencies
allow; every milestone ends runnable; product surface last. Implementation
status, pitfalls and conclusions are tracked live per milestone in
`progress.md`.

## Delivered

**M0 — Groundwork.** claude-code injectable spawn + env overlay; provider
execution-requirement capability flag; the host-switch integration test.

**M1 — Runner protocol core.** `@demicodes/runner-protocol` (fs RPC,
streaming spawn, handshake) and the runner client, exercised against a bare
AgentServer.

**M2 — Backend skeleton.** `@demicodes/backend` serving the conversation
stream and the Web API multi-user-shaped; storage module; conversation
persistence and session index; the store-backed Host as the default target.

**M3 — Storage final shape.** `control.sqlite` + `conversations/<id>.sqlite`,
the block-row journal, media out of the transcript via `source.ref`,
`host_store` scoped per conversation, `ControlService` in front of
`control.sqlite`.

**M4 — Runner productized.** Claim-by-token flow, device registry with
online status, backend resolution of execution targets to remote Hosts.

**M5 — LLM module, vault, metering, Claude Code.** Vault key storage,
per-user provider assembly, usage ledger and enforcement; subscription
login flows and refresh; claude-code sessions spawning their CLI on the
conversation's runner with the vault token as process env.

**M6 — Target switching and attachments.** Turn-boundary switching with
context injection; workspaces CRUD; offline-target degradation;
message-attachment upload and workspace file drop. The prev slot, `switch`
and `release` verbs and the tar-pipe migration delivered here are removed
by M8–M10 (`sessions-and-targets.md`).

**M7 — tinyjs.** `packages/tinyjs`: QuickJS on `rquickjs`, the event
loop with its own liveness count, the module loader with `tinyjs:*`
private to the embedded bundle, the five modules over `hyper`,
`tokio-tungstenite`, `rustls`, `rmpv`; the JS conformance suite driven by
`cargo test`; static musl and macOS builds; the packed-binary entry
(bundle appended to the prebuilt runtime, as Bun's `--compile`). Measured in the Firecracker fixture: command-mode
hello 0.18 s first execution, tee at the pipe baseline.

## Planned

**M8 — Command system, loader, hostless execution** (`commands.md`,
`tinybash.md`, `sessions-and-targets.md`; depends on M7)
The command tree with kinds; the command ABI; the manifest and its build
step; `@demicodes/command-loader`; file commands as `runtime` modules,
`todo`/`agent`/`host` as `rpc`; `demi agent spawn` and the group/leaf
dispatcher rule; `@demicodes/tinybash`; the hostless tool description; two
embedders: the backend in-process (hostless conversations) and the test
fixture; a second root beside `demi` in the tests, to prove the mechanism is not
`demi`-specific; `tinyjsc` split out of the tinyjs binary and the packed
section's release check; the tinyjs fixes the M8 review found. tinyjs in
command mode on a target is M9 work: it is the runner's machine layer plus
the loader's runner side plus an entry bundle.
Accept: a
hostless conversation runs `demi file` and `demi todo`; heredoc, sequence
and refusal cases each covered; one `runtime` module behaves identically
under Bun, on tinyjs and in the test fixture; a third-party embedding
example using only the loader and a custom Host. Real hosts keep the
current runner untouched through M7–M8.
Status: delivered — the loader, tinybash, hostless conversations, the
second root, `tinyjsc` and the tinyjs fixes.

**M9 — Runner on tinyjs, old paths deleted** (`runner.md`; depends on M7, M8)
In dependency order, lightest first:
1. The Host over tinyjs primitives (the runner's machine layer), accepted
   by the Host conformance suite and by tinyjs in command mode running one
   `runtime` command through the loader (a directory `ManifestSource`,
   module import by file path, an entry bundle selecting the root from
   `argv[0]`).
2. The protocol, changed at both ends before the port so the port targets
   the final wire: MessagePack framing, per-op fs messages, `pong` with the
   job count, the one-connection-per-token rule.
3. The runner ported to tinyjs: the job table with real `bash -c`
   (foreground, background, kill, exit), session ids in the environment,
   the working directory carried between jobs, the tee with the model's
   view (head while running, tail at exit) and output files on the target,
   the disconnect semantics; the UDS relay and manifest cache completing
   the CLI's `rpc` path; the manifest served to the runner and its
   symlinks for the root set; the backend's shell environment for real
   hosts over the job messages.
4. What rests on the job table: the brokered cross-host transfer
   (`demi host shell --id`, the transfer messages, the backend broker) and
   browser-bound media by reference.
5. The package structure made final (`package-boundaries.md`), before
   anything is deleted so every deletion lands in its final place:
   `@demicodes/tinybash` standalone, declaring its own system interface
   (`TinybashFs`, `TinybashIO`, `DispatchIO`, `RootPaths`) and depending
   on `utils` alone; `HostlessEnvironment` moved from the backend to
   `@demicodes/shell/hostless`, taking the loader's root paths and
   dispatcher instead of the loader; `nodeFileSystem` under
   `@demicodes/shell/node` for the backend's data directory, with `LocalHost`
   beside it as the Node Host tests run against;
   `@demicodes/host-remote` holding `RemoteHost` and
   `RemoteShellEnvironment`; `@demicodes/runner-protocol` reduced to the
   wire; the Host over tinyjs folded into `@demicodes/runner` as
   `machine/`, `HostRpcServer` and `JobTable` as `serve/`; `AgentServer`'s
   `shellEnvironment` required, no engine default. `@demicodes/host-virtual`
   reduced to the store-backed Host, its spawn refusal deleted.
6. Deletion: `packages/just-bash` (submodule, workspace entry, tsconfig
   paths); the interpreter and portable-command parts of `@demicodes/shell`
   (`bash`, `host-fs`, the environment files, their tests) and the
   offset-paged 1 MB stream view with `runCommandLine`; `@demicodes/host-local`
   (the command bridge, the local agent server, the state layout);
   `@demicodes/host-runner` (now `runner/machine/`); `@demicodes/repl`,
   `@demicodes/agent-eval`, the web package's server and its e2e tests;
   the Bun runner (`RunnerClient`, `main.ts`, `state.ts`) and its bin; the
   old design records (`docs/bash-behavior.md`, `docs/command-bridge.md`,
   `docs/just-bash-fork-policy.md`, the REPL, eval, web and library-plan
   records under `docs/internal/`, the Host and UI guides). Tests that ran
   scripts through just-bash run them through `HostlessEnvironment` over
   `LocalHost`, rewritten to the tinybash subset; a case whose point was a
   real process moves to the tinyjs-runner suites.
Accept: every M1 and M4
integration test passes on the new runner; `cmd1 | cmd2` with zero wire
bytes; `demi todo` round trip over the UDS relay; runner killed mid-command
→ tool error, session continues, reconnect resumes.
Status: step 1 delivered — the Host over tinyjs accepted by the Host
conformance suite (`@demicodes/shell/testing`) on tinyjs; command mode runs `demi file create` and `demi file read`
from a manifest directory through the packed binary reached by its root
symlink (`packages/runner/src/tinyjs/entry.ts`), `file read` in 33 ms
end to end on macOS arm64. Step 2 delivered — the wire at both ends:
MessagePack frames over an injected codec, `fs_<op>` requests with typed
replies, `pong { jobs }`, `hello_error { code, reason }` with the
one-connection-per-token rule; the M1 and M4 suites pass on it, and a test
holds the Bun and tinyjs codecs to the same bytes. Step 3 delivered — the
runner on tinyjs: runner mode with the job table over the tee, the working
directory carried between jobs, the relay and the manifest cache with its
symlinks; the backend's `RemoteShellEnvironment`, manifest push and rpc
relay; the M1, M4 and M6 suites run on the tinyjs runner, a pipeline
crosses the wire as its view and exit only, `demi todo` round-trips the
relay, a runner killed mid-command is a tool error and the reconnect
resumes. Step 4 delivered — brokered transfers: `transfer_send` /
`transfer_receive` / `rpc_transfer` / `transfer_done`, the backend's
transfer broker with `PUT`/`GET /api/transfers/:id`, `demi host list` and
`demi host shell --id` running the script as a job on the named host with
its stdout file transferred (a 300 KB tree copied between two tinyjs
runners with the runner sockets carrying only the view and control
frames; a hostless caller takes the bytes in-process); media by
reference to the browser: outbound transcript frames carry `{ type:
'ref', ref, mediaType }` and `GET /api/blobs/:sha256` serves the bytes.
Steps 5 and 6 delivered — the package structure of `package-boundaries.md`
(tinybash standalone, `shell/hostless` and `shell/node`, `host-remote`,
the runner's `machine/` and `serve/`, the wire alone in `runner-protocol`),
and everything the old paths were: just-bash, the interpreter, host-local,
host-runner, repl, agent-eval, the web server, the Bun runner, the offset
paging, the stale records. M9 closed.

**M10 — Access model and managed hosts** (`sessions-and-targets.md`,
`managed-hosts.md`; depends on M9)
`conversation_host_grants`, auto-grant on switch, the grant check before a
cross-host spawn, `demi host list` / `current` / `shell --id`, wake on
`shell --id`; the Firecracker provisioner under jailer with the privileged
helper, chroot layout, tap networking, egress rules, crash-loop guard; the
guest kernel and preinstalled-rootfs pipeline, the ephemeral upper; the
home-image store with small nominal size and online growth, shrink on
hibernate, untouched-skip; idle rule, hibernate, wake, periodic checkpoint
with the liveness exemption; auto-provision on the first non-`demi` command
with placement of the hostless files; Cloud workspace provisioning. Accept:
full flows against a fake provisioner plus a local runner, including
owner-scoped authz; Firecracker smoke env-gated (Linux with `/dev/kvm`),
including cold-provision and wake latency.

**M11 — Multi-user systems** (`product.md`)
Real auth (username/password, cookie sessions, master/admin/user roles, no
registration, no recovery); user-management and instance-settings
endpoints; shared/isolated instance-mode enforcement; the tenant-isolation
authz matrix. At the end of M11 the entire API surface is complete and
frozen.

**M12 — Web UI** (`product.md`)
The entire `@demicodes/web` package in one concentrated phase: scaffold,
workspace-grouped sidebar, chat view on web-ui, settings dialogs, the target
picker with the hostless state, grant management, media by reference.
Consumes the M11-frozen API; adds no backend surface. The old local dev
product was deleted in M9 with the Node Host it ran on; until M12 the web
package is the Vite scaffold alone.

**M13 — Deployment packaging**
Container image for the backend (carrying the built web assets); tinyjs
builds per platform with the runner and root-command symlinks; the guest kernel
and the managed-host rootfs image as shipped images; the privileged
provisioner helper; a sample Litestream sidecar config; end-to-end
acceptance.

**M14 — Scaled deployment (post-v1)**
`demi-controld` as a standalone process, the workers' auth-token cache, the
S3 blob backend and home-image store, Litestream on every node, routing-key
plumbing and the sample reverse-proxy config, the user-migration procedure,
multi-instance smoke. No application code path changes — the
`ControlService` seam and the storage shape are final from M3.

Deliberately deferred: the control-priority channel layer beneath the
runner protocol (`runner.md`, with its precondition); the home-image
retention rule (`managed-hosts.md`); fs RPC batching (only when
measurements demand); per-wire usage reconciliation.

## Milestone verification

Test modules and their intended coverage, per milestone.

| Milestone | Coverage |
|---|---|
| M0 | Spawn-injection + `buildClaudeEnv` overlay assertions. Capability-flag tests. Host-switch integration: two temp-dir `LocalHost`s (`@demicodes/shell/node`), context block injected, per-Host environment isolation, transcript continuity. |
| M1 | Protocol codec round-trips. Remote-Host integration against a bare AgentServer: `cat`/`tee`/spawn on a runner in a temp dir; kill the runner mid-command → tool error; reconnect → next command succeeds. |
| M2 | Backend integration in one process: in-process `AgentClient` + store-backed Host; detach mid-turn → turn completes → reattach sees the result; cold-history read equals live transcript. |
| M3 | Block-row persistence: streamed turn appends rows; restore from the two databases equals the live transcript; media round-trips through `source.ref`; per-conversation `host_store` isolation. |
| M4 | Claim-flow integration (unclaimed → claim → reconnect with device token; bad/revoked token; claim-token expiry). Host routing to a claimed device; online status follows the socket. |
| M5 | Vault key storage + per-user assembly; ledger aggregation; login-flow state machines against mock endpoints + refresh; claude-code-on-runner chain against a mock upstream with nothing persisted on the device. Real-subscription smoke gated and manual. |
| M6 | Switch integration (real→real with files staying and an honest context block; mid-turn switch refused; concurrent switch has one winner); offline target → readable and chattable; attachment upload → ref → inline at provider; checkpoint round-trip. |
| M7 | Primitive conformance suite from JS: fs incl. errno cases and streaming reads; spawn incl. stdin/kill/uid/tee byte counts; WebSocket send backpressure and receive; HTTP request bodies from files and response bodies streamed to a fd; UDS listen mode and accept; MessagePack extension types; timers ordering; globals; `tinyjs:*` refused from a file-loaded module. Build-target matrix; guest cold-start (command-mode hello, cold cache), binary size and tee throughput measured in the Firecracker fixture. |
| M8 | Manifest build and hash stability; loader dispatch for both kinds; runtime-module conformance under Bun, tinyjs and fixture; tinybash grammar and builtin tables, the equivalence corpus against real bash + GNU coreutils in a Linux container (`tinybash.md`); parse-first, session-state and cancellation cases; hostless conversation runs file, todo and builtin pipelines; third-party embedding example. |
| M9 | Host conformance suite on the runner's machine layer; a `runtime` command run by tinyjs in command mode; M1 and M4 suites on tinyjs runner; job table (foreground, background, kill, exit); cwd carried between jobs; tee + the model's view (head, tail) + output file; UDS relay round trip with session attribution; MessagePack frames; media by reference in `transcript_reset`; pipeline with zero wire bytes. |
| M10 | Grant table and cross-host spawn authz; `demi host` command surface; fake-provisioner flows (provision, bind, hibernate, wake, checkpoint, crash-loop guard, idle rule with jobs, untouched-skip, owner-scoped authz); auto-provision with hostless-file placement; Cloud workspace once per project; env-gated Firecracker smoke with latency numbers. |
| M11 | Tenant-isolation authz matrix (every API action by user A against user B's data denied); instance-mode enforcement; device revoke + re-claim. |
| M12 | Manual checklist over the full layout, including the "everything Demi implements gets exposed" sweep. |
| M13 | Scripted smoke: build the images, claim a containerized runner against a local backend, one full turn; managed-host image boots and joins via the pre-issued token; optional CI stage. |
| M14 | `ControlService` contract tests against both implementations; two workers + one controld behind a mapped proxy; user migration preserves history; S3 round-trips; domain errors survive the RPC wire. |
