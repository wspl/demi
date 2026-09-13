# Demi Next: Providers, Credentials, and Usage

The backend owns provider configuration, credential access, model discovery,
and inference admission. Provider packages own vendor protocols, credential
refresh, and response parsing. A conversation selects a provider entry and model;
it does not own a copy of the provider's credentials.

HTTP inference runs in the backend. A provider that requires a process runs its
transport on the conversation's execution target. This distinction determines
where credentials must be available:

```text
Backend                                  External service / target
+----------------------------------+
| Provider entry -> credential     |
|                  resolution     |
|                       |          |
| Session runtime ------+----------+----> vendor HTTP API
|                       |          |
|                       +----------+----> runner -> Claude Code CLI
|                                  |      token in process environment
| Usage observer -> control store  |                |
+----------------------------------+                +--> vendor API
```

For example, an OpenAI API conversation can infer without waking Cloud. A Claude
Code conversation needs an available execution target with the CLI installed;
its next inference starts the CLI there using the backend's selected account.

## Provider entries and model discovery

A **family** identifies the provider implementation and credential mechanism.
A **vendor** identifies an API service whose protocol a family can speak.
Multiple API-key entries can use the same family with different keys, endpoints,
or model configurations.

| Family | Credentials | Transport |
|---|---|---|
| `anthropic` | API key | Anthropic HTTP API |
| `openai` | API key | Responses or Chat Completions |
| `google` | API key | Google HTTP API |
| `codex` | Subscription account | Provider-owned network transport |
| `grok-build` | Subscription account | Provider-owned network transport |
| `claude-code` | Imported setup token | CLI on the execution target |

An API-key entry stores its family, key, optional endpoint, optional wire
protocol, optional vendor ID, and optional typed model configuration. These are
user configuration: changing the upstream catalog does not overwrite them.
A subscription entry identifies its family and has a private credential pool.
Each owner scope has at most one subscription entry per family; that entry can
contain multiple accounts with one explicitly selected account.

The backend offers vendors from models.dev whose `npm` field maps to a supported
protocol:

| models.dev client package | Family and protocol |
|---|---|
| `@ai-sdk/openai-compatible` | `openai`, Chat Completions |
| `@ai-sdk/openai` | `openai`, Responses |
| `@ai-sdk/anthropic` | `anthropic` |
| `@ai-sdk/google` | `google` |

Other mappings are not offered. `github-copilot` is excluded because its
credential scheme is not the API-key scheme of these families. Vendor-specific
request requirements belong to the backend's vendor policy and provider adapter;
for example, DeepSeek Chat Completions preserves reasoning content when replaying
prior thinking and tool exchanges.

Model metadata comes from the entry's configured list when present, otherwise
its selected vendor's catalog, otherwise the provider's own model directory.
An unavailable selected vendor does not silently select a different source.
The backend persists fetched directories per provider entry and combines them
with current authentication and runtime health. Cache freshness, refresh,
invalidation, and frontend reuse are defined in
[Model catalog caching](../model-catalog-cache.md). Model metadata is not proof
that the selected execution target can run a process provider.

## Inference admission and runtime ownership

[Instance mode](product.md#instance-mode-shared-vs-isolated) determines provider scope: shared
instances use instance-owned entries; isolated instances use user-owned entries.
The backend checks scope and resolves the current provider entry at each
inference boundary. A missing, deleted, or inaccessible entry refuses the request
before taking a rate-limit slot. A subscription provider also requires a
configured account.

Each session owns its inference runtime. It reuses that runtime while the
provider snapshot, selected model, and required execution Host remain the same.
Editing configuration or the label, switching model, or changing a process
provider's Host creates a replacement runtime and disposes the previous one.
Rebuilding uses the current request's model, thinking setting, and service tier.
An already running request can finish with its original runtime.

The assembly checks freshly read configuration and label before reusing a
provider object. A lookup that completes after an edit cannot make subsequent
requests permanently reuse an old snapshot. Account changes invalidate the
provider and its model directory.

Process admission uses the provider's `requiresProcessCapableHost` capability,
not its name. The backend obtains or wakes Cloud, or refuses an unavailable
paired device, before inference. Each spawn resolves the current target again.
Target ownership and switching are defined in
[Sessions and execution targets](sessions-and-targets.md).

## Credential vault

`backend/vault` owns product scope, encrypted provider configuration, account
operations, and login lifetime. Provider packages own authentication protocols,
refresh, and credential-pool formats. The backend invokes those packages; it
necessarily handles plaintext credentials in memory to make authenticated
requests.

API-key configuration in the control store is encrypted with the instance
secret. Subscription credentials live in private per-provider pool directories;
these files have filesystem access protection and are not covered by the
configuration column's encryption. The storage boundary is described in
[Storage](storage.md). Public provider responses expose configuration metadata
and account status, not token material.

Codex and Grok Build use cancellable device-login flows. Claude Code uses setup-token
import through the provider account API. Creating a subscription entry
publishes credentials in this order:

1. Authenticate or import into a private temporary pool.
2. Move the completed pool to the final provider directory.
3. Insert the provider row, subject to the owner-and-family uniqueness rule.
4. Expose the completed provider to callers.

A concurrent login that loses the uniqueness check removes only its own
unpublished pool. Readers never observe a newly inserted row pointing to the
pending directory. Failed or cancelled flows remove unpublished credentials.
Device login expires after ten minutes; backend shutdown cancels and drains
active flows. Logging into an existing entry reserves that provider operation
until completion or failure.

Selecting an account is explicit. Removing the active account is refused: select
another account first, or delete the provider. Account changes invalidate the
provider's model and quota snapshots. Framework file stores remain available to
local framework consumers; they are not a second product configuration source.
See [Provider credentials](../provider-global-credentials.md) for the shared
credential contract.

## Claude Code execution boundary

The Claude Code provider remains a backend component. It uses the Host process
interface to start the CLI on the conversation's runner and exchanges stream-json
on stdin and stdout. The provider resolves the selected vault account and sends
its token as `CLAUDE_CODE_OAUTH_TOKEN` in the spawn environment. The runner and CLI
therefore receive this credential. A device selected for this transport must be
trusted with that account's token.

The CLI sends its inference traffic directly to the vendor. Demi does not add
an inference proxy or remote-inference RPC. OAuth refresh and quota probes remain
backend-side provider operations. The runner does not need a copy of the
backend's credential pool.

The provider supplies transcript replay and disables CLI session persistence.
Switching targets starts the next transport on the new target with the
backend-owned transcript. This removes dependence on a previous CLI session;
it does not promise identical behavior across different CLI installations or
inherited machine environments.

## Usage and quota

The backend meters provider `response` events and attributes reported token usage
to user, conversation, provider entry, and model. The ledger stores raw usage
records; query-time aggregation produces product totals. HTTP provider usage
comes from the vendor response. Claude Code usage comes from the CLI's
stream-json output over the runner connection, so its accuracy also depends on
the selected machine and CLI. It is not an independently verified billing record.

Admission applies the product's per-user request rate limit before starting
inference. Vendor quota is a separate snapshot supplied by the provider's quota
API; it can be observed during inference or obtained by an explicit probe.
Some probes make an inference request and have a cost. The quota contract and
account invalidation rules are defined in [Provider quota](../provider-quota.md).
A displayed quota snapshot is not itself a product token-budget enforcement rule.

## Implementation limits

The current request limiter is an in-memory, per-backend sliding window with a
default of 120 requests per user per minute. It is not a distributed limiter or
a token-budget implementation. Distributed admission and over-budget policy
remain implementation work if the product requires those guarantees.

`backend/llm/assembly.ts` currently appends usage asynchronously and ignores
storage failures. The metering wrapper records provider response events, but
requests that never produce such an event do not create a usage row. The current
ledger can therefore lose records; it does not satisfy a guarantee of one durable
row for every attempted provider request. Reliable accounting needs an explicit
failure and durability policy before it can support billing or strict budgets.
