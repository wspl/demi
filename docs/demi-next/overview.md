# Demi Next: Multi-User Web — Overview

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (M0–M6 implemented; see `roadmap.md`) |
| Scope | The hosted multi-user chat product. This document holds the core shape and the index; each subsystem has its own record in this directory. |

## Documents

| Record | Covers |
|---|---|
| `overview.md` | motivation, protocol layering, invariants, component map, prior art |
| `roadmap.md` | milestones, acceptance, deferred items |
| `backend.md` | the backend program: modules, deployment topology, routing, Web API |
| `storage.md` | control and conversation databases, `ControlService`, blob and home-image stores, replication |
| `product.md` | instance mode, users, conversations, attachments, provider management, web UI |
| `sessions-and-targets.md` | a conversation's execution target: hostless, user hosts, managed hosts, switching, grants |
| `commands.md` | the command system: root commands (`demi` built in, library users add their own), `rpc` and `runtime` kinds, the command ABI, manifest, loader, tinybash and hostless execution |
| `tinybash.md` | the small shell hostless conversations run in: the corpus-placed boundary, grammar, GNU-faithful builtins, refusals, the equivalence guarantee |
| `shell.md` | the QuickJS shell: the runtime under the runner and every root command on a target |
| `runner.md` | the runner program: handshake, Host RPC, jobs, tee, the local relay |
| `managed-hosts.md` | Firecracker provisioning, images, home persistence, lifecycle, security |
| `providers-and-vault.md` | the LLM module, credential vault, usage accounting, Claude Code |
| `progress.md` | live implementation log, review history, measurements |

Every record is a standalone final-state document. Review history and
rejected alternatives live only in `progress.md`.

## Motivation

A deployable, multi-user, pure-web chat GUI built on Demi. Differentiators
over ChatGPT/Claude web UIs:

- **BYOK and subscription reuse**: users bring API keys or connect their
  existing subscriptions (Claude Code, Codex, Grok, …); all credentials are
  stored server-side and usable from any of the user's devices.
- **Choice of execution environment**: agent tools run on the user's own
  devices via the runner program (user hosts) or in operator-provisioned
  microVMs (managed hosts).
- **Chat-first default**: most conversations are conversation with light
  tools and need no machine at all — they run `demi` commands in the
  backend's tiny shell (tinybash) and only get a machine when they first
  need one.

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
  and output messages (`runner.md`). Both ends are TypeScript: the backend
  on Bun, the runner as JS on the QuickJS shell (`shell.md`).
- Target ↔ backend for root commands (`demi` and any library-defined
  root): a root command on a target is the shell plus the loader;
  `runtime` commands run on the target, `rpc` commands travel to the
  backend as typed messages through the runner's socket (`commands.md`).
- The one special case is the **Claude Code provider**: its transport is the
  CLI, which must run on a real machine. The provider runs in the backend
  like every other provider and spawns its CLI on the conversation's runner
  through the ordinary `spawn`, speaking stream-json over the spawned
  process's stdio. The CLI's HTTPS goes directly to the Claude backend with
  the connection's vault OAuth token injected as process env. The backend
  never proxies or rewrites any provider's model traffic.

## Invariants

1. **Sessions live in the backend.** AgentSession, the transcript, tool
   orchestration and the command tree all run in the backend; the
   authoritative conversation store is backend-local. Runners hold no
   conversation state. Command output beyond the bounded view is the one
   thing that stays on the target (`sessions-and-targets.md`).
2. **The execution target is a mutable conversation property.** A
   conversation's tools execute against a `Host`: the store-backed hostless
   Host in the backend, or the remote Host of a runner — on a user-paired
   device or an operator-provisioned managed host. `AgentHarness.host`
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
   files on the target and the wire carries a bounded view; media reaches
   the browser by reference; bulk transfer, when needed, is an HTTP stream
   brokered by the backend (`runner.md`).
5. **One command manifest.** Every root command — `demi`, and any root a
   library user declares — is defined once in the backend and served to
   every execution surface by the loader; no target has a second
   implementation of a command (`commands.md`).

## Components

- **Backend** (`@demicodes/backend`): one program — Web API, conversation
  hosting, LLM module, vault, accounting, runner management, managed hosts,
  the command manifest — that scales by running more copies plus one
  control-plane process. `backend.md`, `storage.md`.
- **Shell** (`packages/demi-shell`, Rust): a small QuickJS runtime binary
  providing IO primitives, an event loop and the byte-level paths; the only
  Rust in the system. `shell.md`.
- **Runner** (`@demicodes/runner`, JS on the shell): the program on every
  execution target — one outbound socket, Host RPC, the job table, the tee,
  the local relay for root commands. `runner.md`.
- **Command loader** (`@demicodes/command-loader`, pure JS): serves the
  command manifest wherever commands run — inside the runner, inside the
  shell in command mode, inside the backend for hostless conversations, and
  inside any third-party embedder. **tinybash** (`@demicodes/tinybash`,
  pure JS): the tiny shell hostless conversations run in. `commands.md`.
- **Managed hosts**: Firecracker microVMs the backend provisions on demand,
  persisting only a home image. `managed-hosts.md`.
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
   auditable, and explicit device claiming plus per-conversation host grants
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
