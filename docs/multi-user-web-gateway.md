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
3. **Inference executes at the gateway; credentials live server-side; runners
   speak only Demi's own protocols.** Runner sessions request inference over
   the remote inference RPC; the gateway runs the real provider runtimes with
   vault credentials, and the official provider wire protocols exist only on
   the gateway→LLM leg. Runners of every kind hold zero provider credentials.
   Sole exception: the Claude Code CLI's Anthropic-wire traffic, which passes
   through a gateway endpoint with token swap (see Inference service).

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
- **Inference service**: hosts the real provider runtimes with vault
  credentials; runner sessions call it over the remote inference RPC; plus one
  Anthropic-wire passthrough endpoint used solely by the Claude Code CLI (see
  Inference service).
- **Backend session store**: serves transcript/checkpoint persistence for
  runners (incremental — see Session model) and renders cold history by
  feeding the stored transcript to the ordinary web-ui components as a
  full-sync `transcript_reset` frame (or an equivalent read endpoint; follow
  mainstream practice, no second rendering path).

### Runner daemon (new product package, e.g. `@demicodes/runner`)

Detailed design (responsibilities, multiplex protocol, control RPC surface):
`docs/runner-daemon.md`.

A headless sibling of the `@demicodes/web` server: `LocalHost` +
`createCodingAgentHarness` + `createLocalAgentServer`, minus any HTTP
listener — it dials the gateway with one outbound WebSocket and attaches
relayed agent connections as transport bindings. Outbound-only networking
traverses NAT and needs no user firewall setup.

- The daemon assembles **no official providers and holds no credentials**
  (invariant 3): its sessions use the single remote provider, which forwards
  `InferenceRequest`s to the gateway over the multiplexed socket. The one
  exception is the Claude Code provider (CLI on the device), whose env
  overlay points `ANTHROPIC_BASE_URL` at the gateway passthrough with the
  runner's gateway token as `CLAUDE_CODE_OAUTH_TOKEN` (token swap at the
  gateway; CLI adoption of the env token is verified below).
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
(`packages/agent/src/server.ts:807`). Backend authority means these writes
land in the gateway store — but shipping a full checkpoint on every debounce
tick across the network is not acceptable. The write path is therefore
**incremental**: implement the `journal.jsonl` design already planned in
`docs/session-storage-and-naming.md` (append `TranscriptPatch` entries during
streaming — the same patch type that feeds live UIs — and rewrite the
checkpoint + truncate the journal only at action boundaries), and send
patches/boundary checkpoints to the gateway over the multiplexed socket's
store messages. The concrete transcript-store abstraction gets its own design
record before M2 lands. Runner-local disk remains a cache for offline daemon
use of the same machine.

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

## Inference service

### Inference executes at the gateway; official wires exist only gateway→LLM

The protocol layering is: browser ↔ gateway ↔ runner all speak **Demi's own
protocols** (agent frames, the multiplex envelope, and the remote inference
RPC below); the **official provider wire protocols exist only on the
gateway→LLM leg**, spoken by the real provider runtimes
(`createAnthropicApiProvider`, `createCodexProvider`, …) instantiated inside
the gateway with vault credentials at their native default endpoints.

A daemon-side AgentSession therefore does not assemble official providers at
all. It uses one **remote provider**: an `AgentProvider` implementation whose
`run(request)` serializes the existing `InferenceRequest` (items, tools,
thinking, model) over the multiplexed socket and yields the `ProviderEvent`
stream the gateway sends back. Demi's provider contract is already the
normalized interface — events, `TokenUsage`, `ProviderQuota` are uniform
across transports — so this remote hop is written once and covers every HTTP
provider forever. Steering and cancellation map to two extra message types.

This dissolves what the earlier wire-passthrough design needed: no per-wire
proxy code, no credential slot rewriting, no "external auth" provider modes
for codex/grok (their runtimes run at the gateway with real auth), and no
per-provider gateway-mode options. Auth-plane traffic (device login, OAuth
refresh against the hard-coded `auth.openai.com` / `auth.x.ai` /
`console.anthropic.com` endpoints) runs entirely inside the gateway vault.
Codex specifics — `x-codex-*` quota headers, the non-standard
`WebSocket(url, { headers })` auth (Bun supports it) — become gateway-internal
concerns, observed firsthand by the runtime.

### The single passthrough exception: the Claude Code CLI

The CLI must run on a real runner and speaks only the Anthropic wire, so it is
the one place a wire passthrough exists: the daemon's env overlay sets
`ANTHROPIC_BASE_URL` → a gateway endpoint and `CLAUDE_CODE_OAUTH_TOKEN` → the
runner's gateway token; the CLI puts that token in its `Authorization` header
(verified below), and the gateway authenticates it and swaps in the vault
OAuth token before forwarding to `api.anthropic.com`. The claude-code
provider's own quota probe and the models.dev catalog fetch run wherever the
provider runs; the OAuth-usage probe becomes a vault-side concern.

The earlier per-provider `baseUrl`/headers verification below is retained as
the factual record; with this design its load-bearing conclusions are the CLI
row (passthrough feasibility) and the freedom to point any provider at a
non-default endpoint when assembling inside the gateway.

### Usage accounting

Simpler than before: the gateway hosts the provider runtimes, so it observes
`TokenUsage` and quota events firsthand for every HTTP provider, and the CLI
passthrough is metered at the proxy. The ledger aggregates
`user × session × provider × model`; enforcement (auth, rate limits,
over-quota refusal) sits on the remote-inference entry point. There is no
trust gap to reconcile — runners never self-report usage.

## Provider routing rule

`@demicodes/provider` capability metadata gains an execution-requirement flag
(shape TBD, e.g. `requiresProcessHost: true`), declared by `claude-code` and
readable by the product layer. Session routing derives from the flag, never
from provider names — core/provider packages stay ignorant of concrete
providers. UI consequence: selecting a process-requiring provider on a virtual
session prompts the upgrade/bind-device flow; self-host deployments may expose
the backend host itself as a local runner to relax this.

## Changes to existing packages (all additive)

1. `@demicodes/provider-claude-code` — a public CLI env overlay option (at
   minimum `ANTHROPIC_BASE_URL` + `CLAUDE_CODE_OAUTH_TOKEN`).
2. `@demicodes/provider` — execution-requirement capability flag in provider
   metadata.
3. `@demicodes/agent` — implement the planned `journal.jsonl` incremental
   persistence from `docs/session-storage-and-naming.md` (append
   `TranscriptPatch` during streaming, checkpoint rewrite + journal truncation
   at boundaries), so runner→gateway persistence sends patches, not repeated
   full checkpoints. The concrete transcript-store abstraction gets its own
   design record before M2.
4. New packages: `@demicodes/gateway` (product leaf), `@demicodes/runner`
   (product leaf), a small platform-neutral relay/control protocol package
   carrying the envelope + remote inference RPC types and the remote
   `AgentProvider` implementation (both runner and gateway consume it), and a
   virtual `Host` implementation (platform-neutral; candidate
   `@demicodes/host-virtual` mirroring `host-local`'s registry position
   without Node deps).
5. Session persistence glue (journal/checkpoint write-through, store
   messages) is transport work in the runner/protocol packages, not a shell
   concern; the virtual runner uses the gateway session store directly,
   in-process.

No longer needed under this design (dropped from earlier drafts): per-provider
gateway-mode `baseUrl`/headers options for daemon use, codex/grok header-gap
fixes, claude-code `usageUrl`/`modelsDevUrl` plumbing, and any codex/grok
"external auth" mode — the gateway assembles those providers natively with
vault credentials.

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
- claude-code public CLI env-overlay option.
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
Transcript-store design record, then implementation: `journal.jsonl`
incremental persistence in `@demicodes/agent` (the planned remaining work in
`docs/session-storage-and-naming.md`), store messages on the multiplexed
socket, patches during streaming + checkpoints at boundaries landing in the
gateway store; session index; cold history browsing; session lease (single
owning runner, takeover semantics lifted above `SessionOwnershipRegistry`).
Accept: history readable with the daemon offline; a second runner's open is
fenced by the lease; steady-state streaming transfers patches, not repeated
full checkpoints.

**M3 — Remote inference + credential vault + metering**
The remote inference RPC (serialize `InferenceRequest` / stream
`ProviderEvent` over the multiplexed socket, plus steer/cancel messages) and
the runner-side remote `AgentProvider`; gateway-side provider runtimes
assembled from the credential vault (BYOK keys, provider device-login flows,
token refresh); the single Anthropic-wire passthrough endpoint for the Claude
Code CLI with token swap; usage ledger fed by gateway-observed `TokenUsage`.
Accept: a daemon session completes turns with every HTTP provider against a
mock LLM without the daemon holding any credential; the CLI chain works
through the passthrough (skip when no `claude` binary); real-subscription
smoke is manual only, never an ungated test.

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
| M0 | Capability-flag type/declaration tests. `buildClaudeEnv` assertions for the overlay option (no CLI). Cross-machine replay: two LocalHosts in two temp dirs + StubProvider — run turns on A, checkpoint, `fromCheckpoint` on B, continue; assert transcript identity and executing-tool force-complete. |
| M1 | Frame codec round-trip tests in the protocol package. Single-process integration: gateway (random port) + daemon (temp dir) + AgentClient, StubProvider turn through the relay. Fault injection: client disconnect/reconnect mid-turn → `sync_transcript` self-heal yields the full transcript; runner disconnect → error frame reaches the client. |
| M2 | Journal unit tests in `@demicodes/agent` (patch append during streaming, boundary checkpoint + truncation, restore = checkpoint + journal replay). Store-message integration: streaming a turn transfers patches, not repeated full checkpoints (assert on message sizes/counts); turn via daemon, kill daemon → cold history readable from the gateway; second runner `open` fenced by lease, takeover after release; daemon killed mid-turn → authoritative state equals checkpoint + journal and restores. |
| M3 | Remote inference RPC round-trip tests: `InferenceRequest` / `ProviderEvent` serialization (portable JSON incl. `Uint8Array`), streaming, steer, cancel, error propagation. Integration: daemon session completes turns with each HTTP provider runtime at the gateway against a mock LLM endpoint, daemon holding zero credentials; vault injection unit tests; CLI passthrough chain (env overlay → gateway token swap → mock upstream) skips when no `claude` binary is present. Metering: gateway-observed usage → ledger aggregation rows. Tier 2: one gated real-subscription smoke per provider. |
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
