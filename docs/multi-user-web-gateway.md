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
3. **All model traffic flows through the gateway, and all credentials live
   server-side.** Every provider transport points `baseUrl` (or CLI env) at
   the gateway carrying only a gateway-issued runner/session token; the
   gateway injects the real credential (BYOK key or subscription OAuth) at
   forward time. Runners of every kind — user daemons included — hold zero
   provider credentials.

## Components

```
browser ──ws──▶ gateway ◀──ws (outbound)── runner daemon (user device)
 (web-ui)        │  ▲                        └─ AgentServer + LocalHost + providers
                 │  └── virtual runner = ephemeral in-gateway AgentServer (per active session)
                 │  └── docker runner (same daemon binary in a container)
                 ▼
             provider APIs (wire-level passthrough + credential injection)
```

### Gateway (new product package, e.g. `@demicodes/gateway`)

Control plane + data plane in one deployable: a single self-hostable Bun
process. No multi-runtime abstraction — serverless hosting is explicitly out
of scope so its constraints cannot leak into the interfaces:

- **Control plane**: users/auth; the device registry — devices are claimed by
  entering a daemon-printed token in the web UI, persist under the account,
  and expose live online status (= their multiplexed socket state); session
  index; the credential vault (all BYOK keys and subscription OAuth tokens,
  including running the providers' device-login flows and token refresh);
  usage accounting store.
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

Detailed design (responsibilities, multiplex protocol, control RPC surface):
`docs/runner-daemon.md`.

A headless sibling of the `@demicodes/web` server: `LocalHost` +
`createCodingAgentHarness` + `createLocalAgentServer` + the five providers,
minus any HTTP listener — it dials the gateway with one outbound WebSocket
(control channel) and attaches relayed agent sockets as transport bindings.
Outbound-only networking traverses NAT and needs no user firewall setup.

- The daemon holds **zero provider credentials** (invariant 3): providers are
  assembled in gateway mode (`baseUrl` → gateway + runner token), and the
  Claude Code CLI runs in token mode with a placeholder
  `CLAUDE_CODE_OAUTH_TOKEN` whose `Authorization` header the gateway swaps for
  the real server-held token at forward time (the CLI's adoption of the env
  token is verified below).
- The same binary packaged into a container image is the **docker runner**
  (operator-managed) — a hosting variant, not a new code path.

### Virtual runner (default entry) — ephemeral server-side executor

A gateway-side AgentServer over a new in-memory `Host` implementation,
**materialized on demand and disposed when idle**. The key observation: between
turns a session's entire truth is its checkpoint, so the virtual runner is not
a resident service — the gateway spins an AgentSession up via `fromCheckpoint`
when a turn starts (or a wakeup fires), runs it against the virtual Host
(in-memory fs backed by the gateway session store), persists, and tears it
down after an idle timeout. Zero provisioning for the user, near-zero resident
cost for the operator: virtual sessions have no real processes, only fetch
loops, and nothing runs between turns.

Why server-side and not in the browser page: a page-hosted AgentServer dies on
every refresh, navigation, laptop sleep, and mobile tab eviction — exactly
mid-turn, where checkpoint write-through cannot help (it restores to the last
persisted point; it cannot continue an in-flight turn). No browser mechanism
holds a long-lived agent loop reliably across a refresh (SharedWorker survival
with zero clients is browser-discretionary; Service Workers are aggressively
reclaimed). Server-side execution makes turns refresh-immune — the page
reattaches with `open` + `sync_transcript` — and `yield` delayed wakeups work,
since the gateway can rematerialize the session without any page open.

This also keeps the client architecture uniform: the browser is always an
`AgentClient` over WebSocket, for every runner kind; the virtual runner is
simply the runner with the shortest relay distance (in-process at the
gateway).

To verify during implementation: an in-flight turn must keep running when the
client transport detaches (AgentSession is transport-independent by design,
but the binding-close path must not abort the turn).

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
| claude-code | CLI inherits the daemon process env (`cli.ts:36`): daemon sets `ANTHROPIC_BASE_URL` → gateway plus a placeholder `CLAUDE_CODE_OAUTH_TOKEN`; the gateway swaps the `Authorization` header for the vault token (CLI adoption of the env token verified below). An explicit env-overlay option is cleaner (see Changes) | via env | models.dev catalog (`models.ts:26`); the `/api/oauth/usage` quota probe and OAuth refresh (`console.anthropic.com`, `login.ts:12`) move to the gateway vault |

Consequences to accept explicitly:

- **Auth-plane traffic (device login, OAuth refresh) runs entirely at the
  gateway** for every runner kind — credentials are server-side, so the
  hard-coded `auth.openai.com` / `auth.x.ai` / `console.anthropic.com`
  endpoints are called from the gateway's credential vault, never from
  runners.
- Codex `x-codex-*` quota values ride on inference response headers; the
  gateway must forward response headers verbatim or quota observation breaks
  (`packages/provider-codex/src/quota.ts:92`). The WS path never surfaces
  them — unchanged from today.
- Codex WS auth relies on the non-standard `new WebSocket(url, { headers })`
  extension (`transport.ts:303`); the runner runtime matrix must guarantee it
  (Bun does; browsers do not — irrelevant since runners are server-side).
  Runtime verification of WS reverse-proxying is a listed follow-up.
- Provider-side probes that need credentials (the claude-code
  `/api/oauth/usage` quota probe, codex/grok quota) become gateway concerns:
  runner-side providers send them with the placeholder token and the gateway
  injects, or the gateway probes directly from the vault.

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
   `@demicodes/provider-grok-build` — pass configured `headers` to the model
   catalog request (verified missing; see Runtime verification).
4. New packages: `@demicodes/gateway` (product leaf), `@demicodes/runner`
   (product leaf), a small platform-neutral relay/control protocol package
   (may start inside gateway and split when runner consumes it), and a virtual
   `Host` implementation (platform-neutral; candidate `@demicodes/host-virtual`
   mirroring `host-local`'s registry position without Node deps).
5. Gateway-backed `HostStore` implementation is transport glue, not a shell
   concern: the remote (runner-side) flavor lives in the runner package; the
   virtual runner uses the gateway session store directly, in-process.

`docs/package-boundaries.md` gains registry entries for the new packages when
implementation starts. `@demicodes/web` (single-user, Vite-dev product) is
untouched.

## Runtime verification results (2026-08-31, local mocks only)

Both planned live checks ran against local mock servers (Bun 1.3.11, macOS,
`claude` CLI 2.1.220); no real provider endpoint was contacted (external
attempts were caught by a local deny-proxy).

**HTTP providers → mock gateway.** Every provider was driven through
`providerRuntime(...).run(...)` with `baseUrl` pointed at the mock and a
`x-demi-gateway-token` extra header configured. Observed request surface:

| Call | Path observed at mock | Gateway token present |
|---|---|---|
| anthropic-api inference | `POST /v1/messages` (`x-api-key`, `anthropic-version`) | yes |
| openai-api responses wire | `POST /v1/responses` (`authorization: Bearer`) | yes |
| openai-api chat wire | `POST /v1/chat/completions` | yes |
| google inference | `POST /v1beta/models/<id>:streamGenerateContent?alt=sse` (`x-goog-api-key`) | yes |
| grok-build inference | `POST /v1/chat/completions` (`authorization` + `x-xai-token-auth` + session headers) | yes |
| grok-build model catalog | `GET /v1/models` | **no — configured headers not applied** (change item 3) |
| codex SSE inference | `POST <base>/codex/responses` (`authorization`, `chatgpt-account-id`, `session-id`, `thread-id`) | yes |
| codex model catalog | `GET <base>/codex/models?client_version=…` | yes |
| codex quota probe | `POST <base>/codex/responses` | **no** (change item 3); probe successfully parsed `x-codex-*` from mock response headers — verbatim response-header forwarding suffices |
| codex WS inference | WS upgrade `GET <base>/codex/responses` with `authorization`, `chatgpt-account-id`, `openai-beta: responses_websockets=…`, gateway token; first frame `response.create` delivered | yes |

Conclusions: the entire inference/catalog/quota surface follows `baseUrl` with
no stray absolute URLs; per-provider extra headers ride along everywhere except
the two catalogued gaps; and Bun's client `WebSocket(url, { headers })`
extension works, so WS auth reaches a self-hosted gateway endpoint.
API-key options on openai/anthropic/google are resolver functions, not strings.

**Claude Code CLI → mock.** `claude --print` with `ANTHROPIC_BASE_URL` set to
the mock, a fake `CLAUDE_CODE_OAUTH_TOKEN`, a fresh `CLAUDE_CONFIG_DIR`, and
`HTTP(S)_PROXY` set to a local deny-proxy:

- The inference path fully redirects: exactly one request class hit the mock —
  `POST /v1/messages?beta=true` with `authorization: Bearer <env token>` and
  the CLI's `anthropic-beta` feature list. The env-injected token was honored.
  Fed a minimal mocked SSE stream, the CLI completed the turn and exited 0.
- One best-effort direct `CONNECT api.anthropic.com:443` ignores
  `ANTHROPIC_BASE_URL` (present even with
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`); the deny-proxy refused it and
  the run still succeeded, so it is not load-bearing. Gateway deployments
  should expect this background attempt from daemon machines.

Residual (implementation-time) checks: the demi provider spawns the CLI in
stream-json mode with in-band MCP rather than `--print` — the wire endpoint is
the same, but the gateway smoke test should rerun through
`createClaudeCodeProvider` once the gateway exists; and the WS hop should be
re-verified through the actual gateway reverse-proxy implementation.

## Implementation roadmap

Ordering principles: the riskiest long-lived contracts (relay frame protocol,
backend session authority, gateway credential/metering model) are exercised by
real code first; every milestone ends runnable and demoable; product surface
(full UI, deployment targets) comes last because it is cheap to change.
Each item is its own branch off `main`.

**M0 — Groundwork (independent small branches, parallelizable)**
- `@demicodes/provider` execution-requirement capability flag; declared by
  claude-code.
- claude-code public CLI env-overlay option; plumb `usageUrl` / `modelsDevUrl`.
- codex quota probe + grok model catalog: apply configured `headers`.
- Integration test: checkpoint restored via `fromCheckpoint` on a second
  LocalHost in a different directory (simulated cross-machine), then continues
  a turn. The migration design rests on this; existing coverage is same-host.

**M1 — Protocol + minimal loop (first demoable node)**
Relay/control protocol types package; minimal `@demicodes/runner` daemon
(`createLocalAgentServer` + one outbound WS); minimal `@demicodes/gateway`
relay (static-token auth, in-memory registry). Accept: web-ui chats through
the gateway to a daemon in another directory; browser disconnect mid-turn
self-heals via `sync_transcript`. The data model is multi-user from this
milestone (userId on every row, one stub user) — auth can be retrofitted,
tenant-shaped data cannot.

**M2 — Session authority moves to the backend**
Gateway-backed `HostStore` write-through for checkpoints; session index; cold
history browsing; session lease (single owning runner, takeover semantics
lifted above `SessionOwnershipRegistry`). Accept: history readable with the
daemon offline; a second runner's open is fenced by the lease.

**M3 — Inference gateway + credential vault + metering**
Wire-level passthrough proxy (re-verify the Codex WS hop through the real
reverse proxy); server-side credential vault (BYOK keys, provider device-login
flows, token refresh) with injection at forward time — runners hold only
gateway tokens; usage ledger aggregated from authoritative-transcript
`TokenUsage`. Accept: all daemon inference traffic transits the gateway
(including the CLI via env overlay with placeholder-token swap) against mocks;
real-subscription smoke is manual only, never an ungated test.

**M4 — Ephemeral virtual runner as default entry** (parallelizable with M3;
depends on M2 for checkpoint authority)
`host-virtual` package (in-memory fs/store, platform-neutral; `process.spawn`
fails with an actionable "virtual environment — upgrade to a device" message);
gateway-side materialize-on-demand session lifecycle (`fromCheckpoint` on turn
start / wakeup, dispose on idle). Accept: zero-config chat with portable
commands; a page refresh mid-turn reattaches to the still-running turn;
`yield` wakeups fire with no page open; process-requiring providers routed to
the upgrade flow via the capability flag; verify the binding-close path does
not abort an in-flight turn.

**M5 — Migration primitive**
Turn-boundary lease transfer + target-runner replay + harness-injected
migration context block + virtual→real file materialization. Runner switching,
virtual upgrade, and offline fallback all derive from this one primitive.
Depends on M2 (authority) and M4 (virtual source).

**M6 — Multi-user product shell**
Real auth, device claiming (enter the daemon-printed token in the web UI;
persistent device registry with online status), session list / runner picker /
migration UI, tenant-isolation tests. UI is deliberately last-but-one: earlier
milestones accept with the stub user and existing web-ui surfaces.

**M7 — Deployment packaging**
Docker runner image (same daemon binary); gateway deployment packaging;
end-to-end acceptance. Pure hosting work, done only after protocol and storage
interfaces are frozen.

Deliberately deferred (off the critical path, no contract impact): relay
end-to-end encryption, per-wire usage reconciliation, artifact write-through
capacity policy.

## Milestone verification

Three verification tiers, matching existing repo conventions:

1. **Model-free automated tests** — StubProvider + local mocks, runnable with
   scoped `bun test packages/<pkg>`, CI-gating. Every milestone's acceptance
   criteria live almost entirely here.
2. **Env-gated real-credential smoke** — `real-*.e2e.test.ts` behind
   `DEMI_*_E2E` gates, never run by default, manual pre-release only. Real
   models are never a merge gate.
3. **Manual checklists** — only for UI look-and-feel and packaging smoke that
   cannot be automated without new infrastructure.

Per milestone (tier 1 unless marked):

| M | Verification |
|---|---|
| M0 | Capability-flag type/declaration tests. `buildClaudeEnv` assertions for the overlay option (no CLI). Injected-fetch header assertions for the codex quota probe and grok catalog. Cross-machine replay: two LocalHosts in two temp dirs + StubProvider — run turns on A, checkpoint, `fromCheckpoint` on B, continue; assert transcript identity and executing-tool force-complete. |
| M1 | Frame codec round-trip tests in the protocol package. Single-process integration: gateway (random port) + daemon (temp dir) + AgentClient, StubProvider turn through the relay. Fault injection: client disconnect/reconnect mid-turn → `sync_transcript` self-heal yields the full transcript; runner disconnect → error frame reaches the client. |
| M2 | Write-through HostStore unit tests (checkpoint lands in backend store; debounce). Integration: turn via daemon, kill daemon → cold history readable from the gateway; second runner `open` fenced by lease, takeover after release; daemon killed mid-turn → authoritative state equals last persisted point and restores. |
| M3 | Promote the mock-gateway verification harness into tests: mock upstream + real gateway proxy, asserting per-wire passthrough (paths, request headers, verbatim `x-codex-*` response headers), the WS hop through the real proxy, and credential injection (backend-held key added, runner gateway token stripped). Metering: StubProvider turns with usage → ledger aggregation rows. CLI chain (daemon env overlay → gateway → mock upstream) skips when no `claude` binary is present. Tier 2: one gated real-subscription smoke per provider. |
| M4 | host-virtual unit tests: fs/store semantics (`Uint8Array`/`bigint` round-trip), portable-command coverage matrix, spawn-failure message shape. Lifecycle integration with a slow StubProvider: detach client mid-turn → turn completes → reattach sees the full result (covers refresh-immunity and binding-close-must-not-abort in one test); short-delay `yield` wakeup rematerializes the session; idle dispose; `requiresProcessHost` provider rejected with upgrade guidance. |
| M5 | Migration integration: virtual→real asserts lease-transfer ordering, replay, migration context block content, file materialization; real→real asserts files absent and the context block says so; mid-turn migration refused; concurrent double-migration has exactly one winner. Tier 2 (optional): an agent-eval case that the model acts sensibly on the migration note. |
| M6 | Tenant-isolation authz matrix: every API action by user A against user B's sessions/runners/credentials asserts denial. Device-claim integration (happy path + claim-token expiry + re-claim). Tier 3: UI manual checklist — no new browser-test infrastructure. |
| M7 | Tier 3 scripted smoke: build the docker runner image, connect it to a local gateway, run one full turn end-to-end; optional CI stage. |

Test modules and their coverage get documented per milestone in this file (or a
sibling doc) as they land, per the repo's design-record rules.

## Open questions

- Command-artifact write-through cap and retention policy in the backend store.
- Relay end-to-end encryption (browser↔daemon) as a later privacy tier; the
  gateway currently sees frame plaintext by design (it needs usage + cold
  history anyway).
- Whether the virtual Host's fs should be per-session or per-user-workspace.
