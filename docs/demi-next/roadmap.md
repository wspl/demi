# Demi Next: Roadmap

| | |
|---|---|
| Date | 2026-09-02 |
| Status | M0–M6 delivered; M7 next |
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
online status, backend host resolution to remote Hosts.

**M5 — LLM module, vault, metering, Claude Code.** Vault key storage,
per-user provider assembly, usage ledger and enforcement; subscription
login flows and refresh; claude-code sessions spawning their CLI on the
conversation's runner with the vault token as process env.

**M6 — Target switching and attachments.** Turn-boundary switching with
context injection; workspaces CRUD; offline-target degradation;
message-attachment upload and workspace file drop. The prev slot, `switch`
and `release` verbs and the tar-pipe migration delivered here are removed
by M8–M10 (`sessions-and-targets.md`).

## Planned

**M7 — Shell** (`shell.md`; depends on nothing)
The Rust binary embedding QuickJS: ESM loading, the event loop, timers and
promise scheduling, the Web-platform globals; fs primitives with errno
fidelity; spawn with pipes, stdin, kill, tee with a bounded view; TCP, TLS,
WebSocket client, HTTP client, UDS; base64, MessagePack, UTF-8; static musl
builds for Linux x86_64 and aarch64, macOS builds; the two entry modes as
skeletons. Accept: the primitive conformance suite, driven from JS, passes
on every build target; first execution inside a fresh microVM under 0.2 s;
tee throughput at the guest's pipe baseline; the runner-protocol bundle
encodes and decodes on the shell. A library milestone in the M1 sense.

**M8 — Command system, loader, hostless execution** (`commands.md`,
`sessions-and-targets.md`; depends on M7)
The command tree with kinds; the command ABI; the manifest and its build
step; `@demicodes/command-loader`; file commands as `runtime` modules,
`todo`/`agent`/`host` as `rpc`; `demi agent spawn` and the group/leaf
dispatcher rule; the hostless parser; `@demicodes/host-virtual` reduced to
the store-backed Host; the hostless tool description; two embedders: the
backend in-process (hostless conversations) and the shell in command mode
running `runtime` commands (its `rpc` path completes in M9); a second root
beside `demi` in the tests, to prove the mechanism is not `demi`-specific.
Accept: a
hostless conversation runs `demi file` and `demi todo`; heredoc, sequence
and refusal cases each covered; one `runtime` module behaves identically
under Bun, on the shell and in the test fixture; a third-party embedding
example using only the loader and a custom Host. Real hosts keep the
current runner and command bridge untouched through M7–M8.

**M9 — Runner on the shell, old paths deleted** (`runner.md`; depends on M7, M8)
`@demicodes/host-shell` (the Host over shell primitives); the runner ported
to it; the job table with real `bash -c`, session ids in the environment,
artifact files on the target; the UDS relay and manifest cache, completing
the CLI's `rpc` path; MessagePack framing, per-op fs messages, `pong` with
the job count, the one-connection-per-token rule; artifact fetch by
reference and the brokered cross-host transfer; browser-bound media by
reference; the backend's own machine as a runner process; deletion of
`packages/just-bash`, the interpreter and portable-command parts of
`@demicodes/shell`, `host-local/command-bridge`, the Bun runner build,
`docs/bash-behavior.md`, `docs/command-bridge.md`. Accept: every M1 and M4
integration test passes on the new runner; `cmd1 | cmd2` with zero wire
bytes; `demi todo` round trip over the UDS relay; runner killed mid-command
→ tool error, session continues, reconnect resumes.

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
picker with the hostless state, grant management, media by reference,
offline-artifact notices. Consumes the M11-frozen API; adds no backend
surface. The old dev product is renamed `web-demo` here.

**M13 — Deployment packaging**
Container image for the backend (carrying the built web assets); the shell
builds per platform with the runner and root-command symlinks; the guest kernel
and the managed-host rootfs image as shipped artifacts; the privileged
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
| M0 | Spawn-injection + `buildClaudeEnv` overlay assertions. Capability-flag tests. Host-switch integration: two temp-dir LocalHosts, context block injected, per-Host environment isolation, transcript continuity. |
| M1 | Protocol codec round-trips. Remote-Host integration against a bare AgentServer: `cat`/`tee`/spawn on a runner in a temp dir; kill the runner mid-command → tool error; reconnect → next command succeeds. |
| M2 | Backend integration in one process: in-process `AgentClient` + store-backed Host; detach mid-turn → turn completes → reattach sees the result; cold-history read equals live transcript. |
| M3 | Block-row persistence: streamed turn appends rows; restore from the two databases equals the live transcript; media round-trips through `source.ref`; per-conversation `host_store` isolation. |
| M4 | Claim-flow integration (unclaimed → claim → reconnect with device token; bad/revoked token; claim-token expiry). Host routing to a claimed device; online status follows the socket. |
| M5 | Vault key storage + per-user assembly; ledger aggregation; login-flow state machines against mock endpoints + refresh; claude-code-on-runner chain against a mock upstream with nothing persisted on the device. Real-subscription smoke gated and manual. |
| M6 | Switch integration (real→real with files staying and an honest context block; mid-turn switch refused; concurrent switch has one winner); offline target → readable and chattable; attachment upload → ref → inline at provider; checkpoint round-trip. |
| M7 | Primitive conformance suite from JS (fs incl. errno cases, spawn incl. stdin/kill/tee, sockets, timers, globals); build-target matrix; guest cold-start and tee throughput measured in the Firecracker fixture. |
| M8 | Manifest build and hash stability; loader dispatch for both kinds; runtime-module conformance under Bun, shell and fixture; hostless parser table (tokens, heredocs, sequences, every refusal); hostless conversation runs file and todo; third-party embedding example. |
| M9 | M1 and M4 suites on the shell runner; job table (foreground, background, kill, exit); tee + bounded view + artifact file; UDS relay round trip with session attribution; MessagePack frames; artifact fetch by reference; media by reference in `transcript_reset`; pipeline with zero wire bytes; local-machine runner auto-registration. |
| M10 | Grant table and cross-host spawn authz; `demi host` command surface; fake-provisioner flows (provision, bind, hibernate, wake, checkpoint, crash-loop guard, idle rule with jobs, untouched-skip, owner-scoped authz); auto-provision with hostless-file placement; Cloud workspace once per project; env-gated Firecracker smoke with latency numbers. |
| M11 | Tenant-isolation authz matrix (every API action by user A against user B's data denied); instance-mode enforcement; device revoke + re-claim. |
| M12 | Manual checklist over the full layout, including the "everything Demi implements gets exposed" sweep. |
| M13 | Scripted smoke: build the images, claim a containerized runner against a local backend, one full turn; managed-host image boots and joins via the pre-issued token; optional CI stage. |
| M14 | `ControlService` contract tests against both implementations; two workers + one controld behind a mapped proxy; user migration preserves history; S3 round-trips; domain errors survive the RPC wire. |
