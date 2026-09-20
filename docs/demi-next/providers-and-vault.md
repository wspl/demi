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
instances use the master's entries; isolated instances use each user's own.
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

`backend/vault` owns product scope, credential records, account operations, and
login lifetime. Provider packages own authentication protocols, token refresh,
and the shape of the secret document. The backend invokes those packages; it
necessarily handles plaintext credentials in memory to make authenticated
requests.

### Where credentials live

Every credential of the product is a record in the control store, encrypted with
the instance secret. Nothing the product authenticates with is a file:

| Record | Holds |
|---|---|
| `providers.config` | An API-key entry's key and endpoint settings |
| `provider_credentials` | One subscription account: public metadata, the encrypted secret document, its usage snapshot, and a version |
| `providers.active_credential_id` | Which account the entry uses for inference |

The backend never reads a vendor's own login from the machine it runs on: not
`~/.codex`, `~/.grok`, the Claude Code keychain item, nor a vendor token in its
environment. Those belong to whoever operates the machine, and a multi-user
product must not lend them to a user whose entry has no account. An entry
without an account is unauthenticated. The instance secret is deployment
configuration (`DEMI_INSTANCE_SECRET`); a single-backend deployment that does
not set it generates one into its data directory.

Because a credential is a record, any worker reads the same accounts, the same
selection and the same usage; there is no pool directory to distribute.

### Scope

A credential has no owner of its own. It belongs to a provider entry, and every
entry belongs to a user. [Instance mode](product.md#instance-mode-shared-vs-isolated)
only decides whose entries a user infers with: the master's on a shared
instance, their own on an isolated one. There is no ownerless entry and no
second kind of credential.

| | Shared instance | Isolated instance |
|---|---|---|
| Entries and accounts belong to | The master | Each user |
| Who adds, tests, refreshes, selects and removes accounts | The master | The owner |
| Who infers with the active account | Every user | The owner |
| Who sees account labels, plan and usage | The master | The owner |
| What a user who only infers sees | The entry and its models; no account, plan or usage | — |

Every credential operation resolves the entry first: management requires being
its owner, inference requires the entry to be the one the mode assigns to the
user. The store the backend hands a provider package is bound to one entry, so a
package cannot list or read another entry's accounts.

On a shared instance the master's accounts are a shared resource, which sharpens
three rules:

- **Refresh.** Many users on many workers use the same account at once, so the
  versioned `replace` below is what keeps one user's refresh from invalidating
  everyone else's tokens. On an isolated instance the rule is the same and
  rarely contended.
- **Usage.** The vendor's windows are consumed by all users together. The
  snapshot describes the account, not a user; per-user consumption is the usage
  ledger's, not the vendor's.
- **Disclosure.** A process provider sends the active account's token to the
  execution target ([Claude Code execution boundary](#claude-code-execution-boundary)).
  On an isolated instance that is the owner's token on the owner's machine. On a
  shared instance it is the master's token on a machine another user controls,
  Cloud included: every user who can infer with a Claude Code entry can read its
  token. Demi allows this and does not hide it; a master who adds a Claude Code
  account to a shared instance is trusting every user with that account.
  Network providers (Codex, Grok Build, API keys) never send a credential off
  the backend.

### The credential pool contract

A provider package does not know where credentials live. It receives a
**credential pool** bound to one entry and keeps the vendor-specific secret
document opaque to it
([Provider credentials](../provider-global-credentials.md#51-the-pool-is-injected)):

| Operation | Meaning |
|---|---|
| `list`, `readMeta`, `findByIdentityKey` | Public metadata of this entry's accounts |
| `writeEntry(meta, secret)` | Insert, or replace the account with this id |
| `document(id).read()` | `{ text, version }`, or nothing |
| `document(id).replace(text, version)` | Write a refreshed secret only if `version` is still current |
| `getActiveId`, `setActiveId` | The account the entry infers with |
| `remove(id)` | Delete the account |

The backend's pool is the `provider_credentials` table
(`backend/vault/credential-pool.ts`). The framework's file pool and each kit's
vendor pool implement the same contract for local consumers; a kit falls back
to the vendor's own login only when it composes those two itself, which it does
when no pool is passed. The backend always passes its pool, so an entry without
an account never stands for a vendor login of this machine.

**Refresh** goes through `replace`. OAuth refresh tokens are single-use, so two
refreshers of one account race: the one whose `replace` finds a newer version,
or whose vendor call is refused, reads the record again and uses the tokens the
winner stored. It reports a failure only when the record did not change. Within
one backend, refreshes of one account are a single flight.

### An account is the unit

The provider runtime is built for **one account**, named by credential id. The
entry's active account is only the default the backend passes for inference.
Everything else names the account it means:

| Operation | Account |
|---|---|
| Inference, model discovery | The active account |
| Connection test | The account the user chose |
| Usage probe, plan | The account the user chose |
| Token refresh | The account being used |

Usage is therefore known per account and stored with it. Selecting another
account changes which record inference reads; it clears nothing, and the account
left behind keeps its usage until someone refreshes it. Removing an account
removes its usage with it. Response-derived usage is written to the account that
made the request, so a request that outlives a switch cannot credit the wrong
account.

### Login and publication

Codex and Grok Build use cancellable device-login flows. Claude Code uses
setup-token import through the provider account API. A flow authenticates
against a staged store held in memory; nothing is stored until it completes.
Completion is one control-store transaction:

- for a new entry: insert the provider row, subject to the owner-and-family
  uniqueness rule, its first account, and the active selection;
- for an existing entry: insert or replace the account, and select it only when
  the entry had no active account.

A flow that loses the uniqueness check, fails or is cancelled has stored
nothing, so there is nothing to clean up. Device login expires after ten
minutes; backend shutdown cancels and drains active flows. Logging into an
existing entry reserves that provider operation until completion or failure.

Selecting an account is explicit. Removing the active account is refused: select
another account first, or delete the provider. Deleting a provider deletes its
accounts in the same transaction. Public provider responses expose
configuration metadata, account metadata and usage, never token material.

## Claude Code execution boundary

The Claude Code provider remains a backend component. It uses the Host process
interface to start the CLI on a runner and exchanges stream-json on stdin and
stdout. Which CLI that is, and which machine, are defined in
[The Claude Code CLI](claude-cli.md): Demi's own verified copy, on the
conversation's execution target for inference. The provider resolves the selected vault account and sends
its token as `CLAUDE_CODE_OAUTH_TOKEN` in the spawn environment. The runner and CLI
therefore receive this credential. A device selected for this transport must be
trusted with that account's token; on a shared instance that means every user
([Scope](#scope)).

The CLI sends its inference traffic directly to the vendor. Demi does not add
an inference proxy or remote-inference RPC. OAuth refresh and quota probes remain
backend-side provider operations. The runner does not need a copy of the
backend's credential pool.

The provider supplies transcript replay and disables CLI session persistence.
Switching targets starts the next transport on the new target with the
backend-owned transcript. This removes dependence on a previous CLI session.

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

A backend that finds pool directories of an older version under
`<dataDir>/vault/` imports each into its entry's account records — secrets,
active selection and usage snapshot — and removes it, before serving.

`DEMI_INSTANCE_SECRET` is not read yet: the instance secret is the file in the
data directory. Each backend holds the latest usage snapshot of an account in
memory beside its record, so with several workers a snapshot another worker
stored shows after the next probe or restart; multi-worker deployment is not
implemented.

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
