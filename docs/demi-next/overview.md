# Demi Next: Multi-User Web — Overview

| | |
|---|---|
| Date | 2026-09-08 |
| Status | Target architecture contract |
| Scope | The hosted multi-user chat product. This document holds the core shape and the index; each subsystem has its own record in this directory. |

## Documents

| Record | Covers |
|---|---|
| `overview.md` | motivation, protocol layering, invariants, component map, prior art |
| `roadmap.md` | milestones, acceptance, deferred items |
| `execution-coordination.md` | execution identity, tree admission, shared-device admission and recovery |
| `backend.md` | the backend program: modules, deployment topology, routing, Web API |
| `storage.md` | control and conversation databases, `ControlService`, blob and machine-image stores, replication |
| `product.md` | instance mode, users, conversations, attachments, provider management, web UI |
| `sessions-and-targets.md` | a conversation's execution target: Cloud, paired devices, workspaces, switching, attached hosts |
| `commands.md` | the command system: root commands (`demi` built in, library users add their own), `rpc` and `runtime` kinds, the command ABI, manifest, loader |
| `native-runtime.md` | Rust runner, embedded shell, resident command services and artifact distribution |
| `runner.md` | the runner program: handshake, Host RPC, jobs, tee, the local relay |
| `managed-hosts.md` | Firecracker provisioning, images, system/home persistence, reset, lifecycle, security |
| `providers-and-vault.md` | the LLM module, credential vault, usage accounting, Claude Code |
| `scenarios.md` | the scenario suite over the headless system: the world fixture, the driver, the teardown invariants, the scenarios and restarts |

Every record describes the architecture of its subsystem. Implementation history
belongs to Git; test results belong to the test runner and CI.

## Motivation

A deployable, multi-user, pure-web chat GUI built on Demi. Differentiators
over ChatGPT/Claude web UIs:

- **BYOK and subscription reuse**: users bring API keys or connect their
  existing subscriptions (Claude Code, Codex, Grok, …); all credentials are
  stored server-side and usable from any of the user's devices.
- **Choice of execution environment**: agent tools run on the user's own
  devices via the runner program (user hosts) or in operator-provisioned
  microVMs (managed hosts).
- **Chat-first default**: new conversations select the user's Cloud immediately.
  The VM starts only when a file, process or process-backed provider needs it;
  reading history and ordinary inference do not require an active machine.
- **Personal Cloud**: one managed device per user, persistent system and home,
  project directories shared on that machine, and self-service system reset
  that preserves home.

## Protocol layering (the core shape)

```
web  ←— our protocol —→  backend  ←— official provider wires —→  LLM providers
                            │
                            └←— our runner protocol (Host RPC) —→ runner (user host / managed host)
                                     ├─ real bash on the target, `demi` commands via the loader
                                     └─ claude code CLI (spawned by backend via the runner;
                                        stream-json over stdio to the backend)
                                            └── its own native Anthropic HTTPS ——▶ Claude backend
```

- Browser ↔ backend: Demi's agent protocol (`ClientFrame`/`ServerFrame`) on
  the per-conversation stream socket, plus the Web API — plain HTTP REST for
  everything else the page calls (`backend.md`).
- Backend ↔ LLM providers: the official wire protocols, spoken by the real
  provider runtimes instantiated inside the backend with vault credentials at
  their native endpoints (`providers-and-vault.md`).
- Backend ↔ runner: Demi's runner protocol — a remote form of the `Host`
  contract (filesystem ops, process spawn with streamed stdio) plus the job
  and output messages (`runner.md`). The backend runs TypeScript on Bun and the runner runs Rust. Shared schemas generate the Rust wire contract.
- Target ↔ backend for root commands (`demi` and any library-defined
  root): a root command uses the runner’s local HTTP/2 dispatcher;
  `native` commands run in resident target-side executables, `rpc` commands travel to the
  backend as typed messages through the runner's socket (`commands.md`).
- The one special case is the **Claude Code provider**: its transport is the
  CLI, which must run on a real machine. The provider runs in the backend
  like every other provider and spawns its CLI on the conversation's runner
  through the ordinary `spawn`, speaking stream-json over the spawned
  process's stdio. The CLI's HTTPS goes directly to the Claude backend with
  the provider's vault OAuth token injected as process env. The backend
  never proxies or rewrites any provider's model traffic.

## Invariants

1. **Sessions live in the backend.** AgentSession, the transcript, tool
   orchestration and the command tree all run in the backend; the
   authoritative conversation store is backend-local. Runners hold no
   conversation state. Command output beyond the model's view is the one
   thing that stays on the target, and stays there
   (`sessions-and-targets.md`).
2. **The execution target is a mutable conversation property.** A
   conversation's tools execute against the remote `Host` of a runner — on a
   user-paired device or the user's unique managed Cloud device. `AgentHarness.host`
   resolves a stable Host per execution target from action metadata.
   Switching targets is a first-class operation at a turn boundary,
   announced to the model with an injected context block.
3. **All credentials are stored server-side, and each provider package owns
   its own credential machinery.** API keys and subscription OAuth material
   live in the vault; login and refresh run server-side through the
   provider's own flows. The one CLI transport (Claude Code) receives its
   access token only as process env at spawn time — the device never
   persists a credential, and the runner program itself is never given one.
4. **Protocols carry references, never bulk bytes.** File reads and writes
   happen on the target; the runner tees full command output to output
   files on the target and the wire carries only the model's view of it;
   media reaches the browser by reference; every pipe between processes —
   an `rpc` command's stdin and stdout, a cross-host job's — is an HTTP
   stream brokered by the backend (`runner.md` § Pipes).
5. **One command manifest.** Every root command — `demi`, and any root a
   library user declares — is defined once in the backend and served to
   every execution surface by the loader; no target has a second
   implementation of a command (`commands.md`).

## Vocabulary

Four words are easy to confuse and are used in exactly one sense each:

| Word | Means | Never means |
|---|---|---|
| **target** (execution target) | the conversation's pointer to where its commands run: Cloud, a paired device/directory, or a workspace | a machine |
| **device** | a row in the registry with a token: a paired user device or a managed one | |
| **host** | a machine that executes for Demi, seen through the `Host` contract: a paired device or the user's managed Cloud | the machine that runs a VM |
| **guest** / **backend machine** | virtualization terms only: the microVM, and the machine running the backend and Firecracker | a Demi host |

So a managed host is a *guest* on the *backend machine*; the word "host"
on its own is always Demi's sense.

The `Host` contract (`@demicodes/shell`) has one production implementation the
backend injects into the agent and one internal
realization inside the runner:

| Where | Role | Runs in |
|---|---|---|
| `@demicodes/host-remote` | the Host of every user host and managed host as the backend sees it: each call forwarded over the runner wire | the backend |
| `crates/runner/src` | filesystem, process and shell-job execution requested through `host-remote` | native Rust runner |

The wire is defined by Zod schemas in `@demicodes/runner-protocol`. The backend
uses that package; runner generates its Rust bindings during Cargo builds.

## Components

- **Backend** (`@demicodes/backend`): one program — Web API, conversation
  hosting, LLM module, vault, accounting, runner management, managed hosts,
  the command manifest — that scales by running more copies plus one
  control-plane process. `backend.md`, `storage.md`.
- **Runner** (`crates/runner`): the execution-host executable. Owns in-process
  brush and standard utilities, Host RPC, command parsing/dispatch/forwarding,
  artifact caching and resident command-process lifetimes. `runner.md`.
- **Command loader** (`packages/command-loader`, TypeScript): declares the Zod
  manifest structure, serializes declarations and supplies TS loading. Runner's
  commands module consumes generated manifest bindings. `commands.md`.
- **Command service SDK** (`crates/command-service`): shared HTTP/2 client/server,
  streaming IO and generated protocol types. Its authoritative Zod definitions
  belong to `packages/command-protocol`. `native-runtime.md`.
- **Demi commands** (`crates/demi-commands`): independently released native Demi
  command program using the command service SDK. `commands.md`.
- **Managed hosts**: Firecracker microVMs the backend provisions on demand,
  persisting a pinned base plus a writable system layer and home. `managed-hosts.md`.
- **Web frontend** (`@demicodes/web`): the product SPA over
  `@demicodes/web-ui` and the Web API. `product.md`.

## Prior art and the empty quadrant

Every component of this architecture has large-scale precedent; only the
combination is rare.

- "Agent loop in a service, execution environment across a wire" is the
  standard cloud-sandbox agent shape (E2B/Modal/Daytona-style sandbox APIs
  are fs + exec over HTTP; Devin, Codex cloud, Copilot coding agent all run
  this way).
- "Orchestration in the cloud, execution on user-owned machines" is the CI
  self-hosted-runner shape (GitHub Actions runners; Ansible control nodes).
- Command-granular remote execution is SSH-shaped: one round trip per
  command plus streamed output, proven over WAN for decades.
- Firecracker microVMs with a shared read-only rootfs and a per-owner
  persistent volume is the cloud-workspace shape (E2B, Gitpod's
  stop/backup/resume, Codespaces).
- Shipping command implementations as modules to a thin runtime, keyed by
  content hash, is the plugin-host shape (editor extension hosts, edge
  function runtimes).

The genuinely unoccupied quadrant is the combination: **loop in a
datacenter + execution target on user devices + a fine-grained fs
protocol**. It is empty for two reasons, in this order of importance:

1. **Trust asymmetry (structural, the main reason).** A datacenter service
   holding "execute arbitrary commands on user devices" means a backend
   compromise turns every claimed device into a bot. We accept the
   asymmetry deliberately, with three answers: self-host-first positioning
   (the user controls the datacenter), a runner so thin and frozen it is
   auditable, and explicit device claiming plus per-conversation attached hosts
   (`sessions-and-targets.md`). If this product ever becomes public
   multi-tenant SaaS, device-side capability narrowing is the next step; the
   line exists and we know where it is.
2. **Fine-grained fs over WAN has a famous failure (engineering, the lesser
   reason).** VS Code Remote originally tried "editor logic local, files
   remote," failed on per-op latency, and moved the extension host to the
   file side. Here the load is agent-turn-granular, the heavy operations are
   command-granular (real bash on the target), and file commands run on the
   target as `runtime` modules; the chatty residue is bounded.

**Considered and closed: running the agent loop on the runner.** It would buy
zero-latency tool execution at the price of putting the wire through the
system's fastest-moving interface instead of its most stable one. The Host
contract is essentially frozen; the agent internals change constantly — and
the runner on user devices is the hardest component to update, so it must
contain the least-changing code. A runner-side loop also resurrects
everything this design deleted: transcript sync back to the backend, a
browser↔runner relay, sessions with two homes. The latency win is bounded
(turn wall-clock is inference-dominated) while the costs are structural.
