# Demi Next: Providers, Vault and Accounting

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (implemented in M5) |
| Scope | The LLM module, the credential vault, usage accounting, and the Claude Code special case |

## LLM module

The provider runtimes (`createAnthropicApiProvider`, `createCodexProvider`,
…) are instantiated **inside the backend** with vault credentials at their
native endpoints. The assembly caches public providers by entry identity
and its current configuration and label; each session owns an independent
runtime. The backend never proxies or rewrites model
traffic. The module exposes the aggregated model catalog (live, never
stored) and quota surfaces to the web UI.

**Families and vendors.** The runtime families are the registered
provider types — six factories, each declaring how it is credentialed:
`anthropic`, `openai` (with a `wireApi` choice, Responses or Chat
Completions), `google` by API key; `claude-code`, `codex`, `grok-build` by
subscription login, of which a scope holds at most one entry each. The
vendors are models.dev's: the backend reads `https://models.dev/api.json`
through the provider kit's models.dev client (a day of freshness, etag
revalidation, the last copy served stale on failure) and offers every
vendor whose `npm` tag — the client package that catalog is written for,
its only protocol tag — names a protocol one of our families speaks:

```
models.dev npm tag              our family / protocol
@ai-sdk/openai-compatible  →    openai, chat-completions
@ai-sdk/openai             →    openai, responses
@ai-sdk/anthropic          →    anthropic
@ai-sdk/google             →    google
anything else              →    not offered (no runtime speaks it)
github-copilot             →    not offered (needs its own auth scheme)
```

An API-key entry records its family, key, endpoint, protocol, the vendor
id it came from and an optional typed model list; its catalog is the typed
list if any, else the vendor's models.dev list, else the runtime's own
(the packages' static lists, which stay for the local products). Vendor
endpoints and model lists are never stored.

Vendor request policy is selected alongside the runtime: DeepSeek Chat
Completions replays prior reasoning content so thinking/tool exchanges
retain the fields its endpoint requires.

Every inference resolves the provider from the current user's scope and
vault before rate-limit accounting. A missing or deleted entry refuses
the request. Each session reuses its runtime while the provider snapshot,
selected model and, for process providers, execution Host stay the same;
configuration edits, a model switch or a Host switch recreate that
runtime. Rebuilds use the current request's model, thinking and service
tier. A request already running may finish with its original runtime.
Each spawn also resolves the current Host at invocation time. Provider
cache lookups compare the freshly read configuration and label, so an
older lookup completing after an edit cannot leave later requests using
its snapshot. A provider declares an **execution-requirement
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
  value. A login publishes its completed credential pool into its final
  provider directory before inserting the provider row. Storage atomically
  enforces one subscription entry per owner scope and family, including
  the shared owner scope. Concurrent losing logins fail with a scope conflict
  and remove only their own unpublished pool; observers never see a row
  whose credentials are still in a pending directory.
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
