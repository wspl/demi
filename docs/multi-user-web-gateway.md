# Multi-User Web, Gateway, and Runners

| | |
|---|---|
| Date | 2026-08-31 |
| Status | Proposed (design; verified against current code paths) |
| Scope | New hosted multi-user chat product: gateway control plane, runner daemon, virtual runner, session portability, inference gateway; small additive changes to existing packages |

## Motivation

A deployable, multi-user, pure-web chat GUI built on Demi. Differentiators over
ChatGPT/Claude web UIs:

- **BYOK and subscription reuse**: users bring API keys or reuse existing CLI
  logins (Claude Code, Codex, Grok, …).
- **Choice of execution environment**: agents run on the user's own devices via
  a daemon (Remote-Control style), or in operator-managed environments.
- **Chat-first default**: most sessions are conversation with light tools and do
  not need a real machine at all.

This is a product-layer design. It does not change Demi's runtime architecture;
it is the first external consumer of several existing abstractions (provider
contract, `baseUrl` overrides, `TokenUsage` normalization, checkpoint restore,
client-owned session ids).

## Invariants

1. **The backend owns the authoritative transcript.** A session's durable truth
   is its `AgentSessionCheckpoint` stored in the backend. Runners are
   replaceable replay executors; runner-local state is cache.
2. **The execution target is a mutable session property.** Moving a session to
   another runner/directory is a first-class operation: a lease transfer at a
   turn boundary, a checkpoint replay on the target, and an injected context
   block that tells the model what changed. Virtual→real upgrade, runner
   switching, and offline degradation are all this one primitive.
3. **All model traffic flows through the gateway.** Every provider transport
   points `baseUrl` (or CLI env) at the gateway. Credentials are modeled
   per-runner; the injection point for backend-held credentials is the gateway.

## Components

```
browser ──ws──▶ gateway ◀──ws (outbound)── runner daemon (user device)
 (web-ui)        │  ▲                        └─ AgentServer + LocalHost + providers
                 │  └── virtual runner (in-process AgentServer + virtual Host)
                 │  └── docker runner / CF sandbox runner (same daemon binary)
                 ▼
             provider APIs (wire-level passthrough + credential injection)
```

### Gateway (new product package, e.g. `@demicodes/gateway`)

Control plane + data plane in one deployable, written runtime-portable
(self-host Bun process; Cloudflare Workers with Durable Objects per session for
the relay):

- **Control plane**: users/auth, device pairing (device-code flow), runner
  registry, session index, credential vault for backend-held keys, usage
  accounting store.
- **Frame relay**: forwards `ClientFrame`/`ServerFrame` JSON between the
  browser's `AgentClient` WebSocket and the owning runner's WebSocket. The
  agent protocol already tolerates relay reconnects: transcript frames carry a
  monotonic `revision` and the client self-heals gaps with `sync_transcript`
  (`packages/agent/src/client.ts:200`).
- **Inference gateway**: wire-level reverse proxy per provider family. No
  protocol translation — Anthropic Messages, OpenAI/Codex Responses (SSE and
  WebSocket), Gemini `generateContent`, and Grok proxy traffic pass through in
  native format. The Claude Code CLI is just another Anthropic-wire client.
- **Backend session store**: serves checkpoint reads/writes for runners (see
  Session model) and renders cold history.

### Runner daemon (new product package, e.g. `@demicodes/runner`)

A headless sibling of the `@demicodes/web` server: `LocalHost` +
`createCodingAgentHarness` + `createLocalAgentServer` + the five providers,
minus any HTTP listener — it dials the gateway with one outbound WebSocket
(control channel) and attaches relayed agent sockets as transport bindings.
Outbound-only networking traverses NAT and needs no user firewall setup.

- Credentials stay on the device: the daemon reuses the machine's existing CLI
  logins (`~/.claude` keychain/pool, `~/.codex` auth, `~/.grok/auth.json`)
  through the existing provider auth stores. Nothing is uploaded; provider
  assembly happens on the runner.
- The same binary packaged into a container image is the **docker runner**
  (self-host) and the **CF sandbox runner**. These are hosting variants, not
  new code paths. Their difference is credential source: the gateway injects
  backend-held credentials at proxy time, so sandboxes never see raw keys.

### Virtual runner (default entry)

An AgentServer inside the gateway process over a new in-memory `Host`
implementation. Zero provisioning cost, so it is the default for new sessions:
pure chat and light tool use work immediately; complex tasks upgrade to a real
runner via migration.

Verified feasibility: the `Host` contract is platform-neutral and enforced so
(`packages/shell/src/__tests__/root-entry.test.ts:15`), `BashEnvironment`
routes portable commands (`cat`, `ls`, `grep`, `tee`, redirection, …) through
`Host.fs` without ever calling `Host.process.spawn`
(`packages/shell/src/__tests__/environment.test.ts:1485` proves zero spawn
calls), and in-memory `Host`/`HostStore` shapes already exist as test doubles
(`packages/coding-agent/src/__tests__/coding-harness.test.ts:256`).

Known limits (surface in the product, not papered over):

- `bash`, `sh`, `sleep`, and background jobs (`&`) intentionally require real
  spawn (`packages/shell/src/portable-commands.ts:18`,
  `packages/shell/src/environment.ts:616`); any non-portable binary (`curl`,
  `python3`, `node`) is unavailable. The virtual Host's `process.spawn` returns
  a clear "this session runs in a virtual environment — upgrade to a device to
  run real programs" failure the model can act on.
- The Claude Code provider spawns the CLI and therefore never runs on the
  virtual runner (see Provider routing).

### Web frontend

`@demicodes/web-ui` unchanged: it already consumes an injected `AgentClient`
and a transport-agnostic control client. The new product shell adds login,
device/runner management, session list with runner badges, and the
migrate/upgrade flow. The gateway's control API is a superset of the existing
four-method `/control` RPC (`packages/web-ui/src/transport/protocol.ts:40`).

## Session model

### Authority

`AgentSessionCheckpoint` (`{ transcript, state, phase, queue, cwd, model,
harnessName }`, `packages/agent/src/types.ts:199`) is the restorable truth and
already flows through `Host.store` at
`agent-sessions/<sessionId>/checkpoint.json` with debounced writes
(`packages/agent/src/server.ts:807`). The mechanism for backend authority is a
**gateway-backed `HostStore` facet** given to every runner Host: checkpoint
writes write through to the backend (the portable JSON codec in
`@demicodes/utils` exists precisely for `HostStore` implementations that cross
a wire). Runner-local disk remains a cache for offline daemon use of the same
machine.

Command artifacts (`commands/<id>/artifact.json`) are size-capped on the
write-through path; the full artifact stays runner-local. Cold history browsing
therefore always has the transcript and bounded tool views
(`docs/session-storage-and-naming.md`), and may lack full command output for
offline runners — acceptable.

### Lease and takeover

The gateway holds a per-session lease: exactly one runner binding may own a
session id at a time. This mirrors semantics Demi already has — session ids
are client-owned on the `open` frame and `SessionOwnershipRegistry` makes a
second open take over by closing the first binding
(`packages/agent/src/server.ts:182`). The gateway enforces the same rule one
level up, across runners.

### Migration (= switch runner, upgrade virtual→real, resume elsewhere)

At a turn boundary only:

1. Gateway revokes the current lease (runner closes the session binding; final
   checkpoint write-through completes).
2. Target runner opens the session id; `AgentSession.fromCheckpoint` restores
   it (`packages/agent/src/session.ts:106`). `harnessName` must match;
   executing tool calls are force-completed with an error result — consistent
   with "migration only between turns".
3. The harness injects a migration context block into the transcript: previous
   runner/directory, new runner/directory, and (except virtual→real) "previous
   filesystem state is unavailable". This block enters the authoritative
   transcript, so every future replay carries it.
4. Virtual→real upgrade additionally materializes the virtual Host's files
   into the target directory (the virtual fs lives in the backend and is
   small by construction).

Files never move in real→real migration. Daemon offline ⇒ the session is
read-only from the backend transcript, and the user may migrate it to the
virtual runner to keep talking (without the files).

Provider-side portability is already built: the Claude Code provider replays
the transcript into a fresh CLI process with `--no-session-persistence` — no
CLI session id, no config-dir state travels; cross-machine resume needs only a
`claude` binary and local credentials
(`packages/provider-claude-code/src/jsonl.ts:79`,
`packages/provider-claude-code/src/cli.ts:14`). HTTP providers are stateless
per request.

## Inference gateway

### Why wire-level, not a normalized protocol

A gateway-normalized inference protocol can never absorb the Claude Code CLI,
which speaks only the Anthropic wire — normalization would recreate the very
fragmentation it is meant to remove. Demi's normalization layer already exists
on the runner: the provider contract emits uniform provider events,
`TokenUsage`, and `ProviderQuota` regardless of transport. The gateway stays a
dumb, per-wire passthrough with authentication, routing, credential injection,
and rate limiting.

### Verified per-provider routing matrix

| Provider | baseUrl → gateway | Extra headers (gateway token) | Traffic that bypasses the gateway |
|---|---|---|---|
| openai-api | `baseUrl` option, env fallback (`provider.ts:201`) | `headers` resolver per request (`provider.ts:65`) | none (static model catalog, no probes) |
| anthropic-api | `baseUrl` option (must include `/v1`) | `headers` resolver | none |
| google | `baseUrl` option | `headers` resolver | none |
| grok-build | `baseUrl` option; models + billing/quota probes follow it (`models.ts:145`, `quota.ts:53`) | static `headers` map | OAuth refresh + device login against `auth.x.ai` (hardcoded issuer, `auth.ts:77`) |
| codex | `baseUrl` drives SSE, WS (scheme swap, same host/path, `transport.ts:206`), `/codex/models`, and the quota probe | `headers` option on inference + catalog; **not** on the quota probe (`quota.ts:16`) | OAuth refresh + device login against `auth.openai.com` (hardcoded, `auth.ts:103`, `device-login.ts:52`) |
| claude-code | CLI inherits the daemon process env (`cli.ts:36`), so the daemon sets `ANTHROPIC_BASE_URL` (+ gateway token var) before spawn; an explicit env-overlay option is cleaner (see Changes) | via env | provider-side quota probe `api.anthropic.com/api/oauth/usage` (option exists but not plumbed, `quota.ts:19`); models.dev catalog (`models.ts:26`); OAuth refresh against `console.anthropic.com` (`login.ts:12`) |

Consequences to accept explicitly:

- **Auth-plane traffic (OAuth refresh, device login) does not transit the
  gateway** for daemon runners. That is correct: those exchanges belong to the
  user's own credential custody. For backend-managed runners the gateway holds
  the credentials and performs refresh itself, so the sandbox sees neither
  tokens nor auth endpoints.
- Codex `x-codex-*` quota values ride on inference response headers; the
  gateway must forward response headers verbatim or quota observation breaks
  (`packages/provider-codex/src/quota.ts:92`). The WS path never surfaces
  them — unchanged from today.
- Codex WS auth relies on the non-standard `new WebSocket(url, { headers })`
  extension (`transport.ts:303`); the runner runtime matrix must guarantee it
  (Bun does; browsers do not — irrelevant since runners are server-side).
  Runtime verification of WS reverse-proxying is a listed follow-up.
- Daemon-held OAuth tokens transit the gateway in request headers (pass-through
  custody: seen, never stored). Users who reject even transit self-host the
  gateway. Keychain-sourced Claude Code tokens are not injectable by design
  (`oauth.ts:25`) — on such machines the CLI authenticates itself, and its
  traffic still routes via `ANTHROPIC_BASE_URL`.

### Usage accounting

Metering does not parse wire formats. Every provider already normalizes usage
into `TokenUsage` on transcript blocks, and checkpoints/frames flow to the
backend anyway — the accounting pipeline aggregates from the authoritative
transcript stream into a `user × session × provider × model` ledger.
Enforcement at the gateway starts coarse (auth, request counting, rate limits,
over-quota refusal); per-wire usage extractors are a later, purely gateway-side
addition if runner-reported usage ever needs adversarial reconciliation
(daemon runners burn the user's own credentials, so misreporting only harms
the reporter; managed runners are operator-controlled).

## Provider routing rule

`@demicodes/provider` capability metadata gains an execution-requirement flag
(shape TBD, e.g. `requiresProcessHost: true`), declared by `claude-code` and
readable by the product layer. Session routing derives from the flag, never
from provider names — core/provider packages stay ignorant of concrete
providers. UI consequence: selecting a process-requiring provider on a virtual
session prompts the upgrade/bind-device flow; self-host deployments may expose
the backend host itself as a local runner to relax this.

## Changes to existing packages (all additive)

1. `@demicodes/provider-claude-code` — public options for: CLI env overlay
   entries (at minimum `ANTHROPIC_BASE_URL` + auth header var), quota
   `usageUrl`, and catalog `modelsDevUrl` (both exist internally, unplumbed).
2. `@demicodes/provider` — execution-requirement capability flag in provider
   metadata.
3. `@demicodes/provider-codex` — pass configured `headers` to the quota probe.
4. New packages: `@demicodes/gateway` (product leaf), `@demicodes/runner`
   (product leaf), a small platform-neutral relay/control protocol package
   (may start inside gateway and split when runner consumes it), and a virtual
   `Host` implementation (platform-neutral; candidate `@demicodes/host-virtual`
   mirroring `host-local`'s registry position without Node deps).
5. Gateway-backed `HostStore` implementation lives in the runner package (it is
   transport glue, not a shell concern).

`docs/package-boundaries.md` gains registry entries for the new packages when
implementation starts. `@demicodes/web` (single-user, Vite-dev product) is
untouched.

## Follow-up verifications (runtime, before implementation)

Code-path verification is done (this document). Two behaviors need a live
check against local mock servers only — no real-model calls:

1. Point each HTTP provider at a local mock gateway and record exact paths,
   headers, and the Codex WS upgrade behavior through a reverse proxy.
2. Spawn the Claude Code CLI with `ANTHROPIC_BASE_URL` set to a local mock and
   enumerate every path it requests (its endpoint surface beyond
   `/v1/messages` is CLI-internal and undocumented).

## Open questions

- Command-artifact write-through cap and retention policy in the backend store.
- Relay end-to-end encryption (browser↔daemon) as a later privacy tier; the
  gateway currently sees frame plaintext by design (it needs usage + cold
  history anyway).
- Whether the virtual Host's fs should be per-session or per-user-workspace.
