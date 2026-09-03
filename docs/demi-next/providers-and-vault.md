# Demi Next: Providers, Vault and Accounting

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (implemented in M5) |
| Scope | The LLM module, the credential vault, usage accounting, and the Claude Code special case |

## LLM module

The provider runtimes (`createAnthropicApiProvider`, `createCodexProvider`,
…) are instantiated **inside the backend** with vault credentials at their
native endpoints, one runtime per provider, keyed by
`(providerId, modelId)`. The backend never proxies or rewrites model
traffic. The module exposes the aggregated model catalog (live from each
runtime, never stored) and quota surfaces to the web UI.

Providers are assembled per user from the vault; for CLI transports the
assembly is conversation-scoped, because the execution target's spawn is
injected at assembly. A provider declares an **execution-requirement
capability flag** when it needs a process-capable target; the hostless
state refuses such a provider with upgrade guidance — gated by the flag,
never by provider names.

## Credential vault

BYOK keys and subscription OAuth tokens, the providers' device-login flows,
token refresh — including the hard-coded auth endpoints
(`auth.openai.com`, `auth.x.ai`, `console.anthropic.com`), only ever called
from here.

Each provider package owns its credential machinery; the vault is three
authStore implementations plus one `HostStore` implementation, all inside
`@demicodes/backend`:

- Every subscription provider creator accepts `authStore?:` — a two-method
  interface (`status()` + `resolveAuth`/`resolveAccess`) with refresh and
  persistence as implementation concerns; injection takes precedence over
  the file/pool stores. API-key providers take resolver functions.
- Device-login flows return token material without persisting
  (`runCodexDeviceLogin(): CodexAuthDotJson`); the vault stores the return
  value.
- `providers.config` is encrypted at rest with the instance secret
  (`storage.md`). The backend never touches credential bytes beyond naming
  where a provider's pool lives.
- The file-based implementations (`File*AuthStore`,
  `@demicodes/provider/credentials-pool`, the `~/.demi` layout) stay for the
  local products and the runner's machine-local state; no migration or
  compatibility layer.

Verified facts the design rests on: every HTTP provider runtime accepts
`baseUrl` + extra headers and its full endpoint surface follows `baseUrl`;
auth-plane endpoints are hard-coded and vault-only; Codex's WS transport is
a scheme swap on the same host/path and its auth relies on Bun's
`WebSocket(url, {headers})`; `x-codex-*` quota rides on inference response
headers observed firsthand by the runtime.

## Usage accounting

A ledger aggregated from the `TokenUsage` the LLM module observes
firsthand (`user × conversation × provider × model`), one raw row per
provider request, aggregation at query time; enforcement (rate limits,
over-quota refusal) at the inference entry points. There is no trust gap:
runners never self-report usage.

## Claude Code: the special case, contained

The Claude Code provider's transport is the CLI, which must run on a real
machine. The provider runs in the backend like every other provider and
spawns its CLI on the conversation's runner through the ordinary `spawn`,
speaking stream-json over the spawned process's stdio.

- `packages/provider-claude-code` runs the whole credential path itself:
  its auth store resolves and refreshes the OAuth token from the
  provider's vault pool and injects it as `CLAUDE_CODE_OAUTH_TOKEN` into
  the spawned CLI's env. The backend contributes two public factory
  options: **injectable spawn** (a `Host.process`-shaped function targeting
  the conversation's runner) and `stateDir` (the provider's vault
  directory). The CLI's Anthropic traffic goes directly upstream with that
  token — no base-URL override, no proxying. The CLI consumes zero
  device-local state: any device with the binary behaves identically, and
  the runner is never given a credential.
- Verified with local mocks (CLI 2.1.220): the CLI adopts the env token in
  its `Authorization` header; its Messages traffic is one request class
  (`POST /v1/messages?beta=true`); the public `env` overlay option
  (`ANTHROPIC_BASE_URL`) remains a test tool for pointing a CLI at a mock
  upstream.
- Transcript replay needs no CLI-side state (`--no-session-persistence`,
  plain-message replay), so target switching works for Claude Code
  conversations like any other; the next turn cold-starts a CLI on the new
  target.
- The `/api/oauth/usage` quota probe and OAuth refresh run in the backend
  through the provider's own auth store; the models.dev catalog fetch runs
  in the backend.
- The remaining hard-wired filesystem touches are covered: the transport's
  `statSync`/`child_process.spawn` by the injectable spawn; the tmpdir wire
  log disabled in the backend (`DEMI_CLAUDE_WIRE_LOG=0`).

Explicitly **not** part of this design, each unnecessary once sessions live
in the backend: per-provider proxy-mode `baseUrl`/headers options, any
"external auth" provider mode, a normalized remote-inference RPC, or
credentials of any kind on a runner.
