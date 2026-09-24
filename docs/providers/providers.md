# Providers and credentials

A provider connects Demi's agent to one inference service: the Anthropic API,
an OpenAI-compatible API, Gemini, a Codex or Grok Build subscription, or Claude
Code. The backend owns provider configuration, credentials, model discovery and
inference admission. Each provider crate owns one vendor's protocol, its token
refresh and the reading of its responses. A conversation selects a provider
entry and a model; it never holds a copy of the entry's credentials.

HTTP inference runs in the backend. Claude Code, the one provider that needs a
process, runs its CLI on the user's Cloud, whatever the conversation's execution
target is. That difference decides where a credential must be available:

```text
Backend
+-------------------------------------------+
| vault: entry, active account, secret      |
|   |                                       |
|   v                                       |
| provider: one per entry and account       |
|   | builds a runtime on the user's shard  |
|   v                                       |
| session runtime                           |
+---|--------------------------|------------+
    | HTTPS                    | Host process IO, token in the environment
    |                          v
    |               user's Cloud: runner -> Claude Code CLI
    v                                             |
vendor API <------------------ HTTPS -------------+
```

For example, an OpenAI API conversation infers without waking its user's Cloud.
A Claude Code conversation wakes the user's Cloud; its next request starts
Demi's copy of the CLI there with the active account's token, while its tools
keep running on the conversation's own execution target
([Claude Code](claude-code.md)).

Model catalogs and request parameters are defined in [Models](models.md); the
usage ledger, the request rate limit and vendor quota in
[Usage and quota](usage-and-quota.md). The crates and their public items are
listed in [Crates and packages](../architecture/crates-and-packages.md#crates).

## Families, vendors and endpoints

A **family** identifies the provider implementation and its credential
mechanism. A **vendor** identifies an API service whose protocol a family can
speak. An **entry** is a configured provider: a family, a label, and the entry's
credentials and settings. Several API-key entries can use the same family with
different keys, endpoints or model lists.

| Family | Credential | Wire | Runs in |
|---|---|---|---|
| `anthropic` | API key | Anthropic Messages API | Backend |
| `openai` | API key | OpenAI Responses or Chat Completions | Backend |
| `google` | API key | Gemini `generateContent` | Backend |
| `codex` | Subscription account, from device login | Codex Responses over a WebSocket or server-sent events | Backend |
| `grok-build` | Subscription account, from device login | Chat Completions through Grok Build's chat proxy | Backend |
| `claude-code` | Subscription account, from an imported setup token | stream-json with the Claude Code CLI | The user's Cloud |

An API-key entry stores its family, key, optional endpoint, optional wire
protocol, optional vendor ID, and optional typed model list. These are user
configuration: a change in the vendor's catalog does not overwrite them. A
subscription entry names its family and holds its accounts. Each owner has at
most one subscription entry per family; that entry can hold several accounts,
one of which is explicitly selected as the active account.

The backend stores an entry's configuration as one typed document and decodes
it strictly when it reads it: a field the type does not name is an error, so a
misspelled setting is reported instead of silently doing nothing. The provider
receives that configuration already typed. It reads its key, endpoint and
options only from there, never from the environment of the process it runs in.

### Vendors from models.dev

The backend offers the vendors listed by models.dev whose `npm` field maps to a
supported protocol:

| models.dev client package | Family and protocol |
|---|---|
| `@ai-sdk/openai-compatible` | `openai`, Chat Completions |
| `@ai-sdk/openai` | `openai`, Responses |
| `@ai-sdk/anthropic` | `anthropic` |
| `@ai-sdk/google` | `google` |

Other mappings are not offered. `github-copilot` is excluded because its
credential scheme is not the API-key scheme of these families.

A vendor's request requirements are typed settings of the backend's vendor
policy. The policy applies them to every model of that vendor and passes them
to the provider with the entry's configuration. For example, DeepSeek's Chat
Completions needs earlier thinking replayed as `reasoning_content` on tool-call
continuations, and some Responses gateways need replayed assistant items marked
as completed. In the other direction, the Chat Completions reader turns
reasoning that a compatible vendor streams as `reasoning_content` deltas into
thinking, for every vendor and without a setting.

Which models an entry offers (its own list, its vendor's catalog, or the
provider's directory) and how the backend caches them is defined in
[Catalog sources](models.md#catalog-sources).

### Endpoints

An API-key entry's endpoint is the base URL of the vendor's API. The provider
appends the path of the request it makes, unless the configured URL already
ends with that path. An entry without an endpoint uses the family's default:

| Family | Default base URL | Request |
|---|---|---|
| `openai` | `https://api.openai.com/v1` | `…/responses`, or `…/chat/completions` when the entry's wire protocol is Chat Completions |
| `anthropic` | `https://api.anthropic.com/v1` | `…/messages` |
| `google` | `https://generativelanguage.googleapis.com/v1beta` | `…/models/<model>:streamGenerateContent`, as server-sent events |

An Anthropic base URL includes the API version, usually `/v1`. Some
Anthropic-compatible services document a root such as
`https://api.kimi.com/coding/`, to which their own clients append
`/v1/messages`; an entry for such a service uses
`https://api.kimi.com/coding/v1`.

Google speaks Gemini's native API, not its OpenAI-compatible one. A video is
therefore a real inline part, so the model reads its frames and its audio
track, and thought summaries, thought signatures and thinking token counts
arrive as fields of their own instead of tags inside the answer text. A
function call that Gemini returns without an ID gets a new unique ID (a UUID),
so it never repeats an ID already in the transcript.

Subscription families talk to their vendors' own endpoints:

| Family | Inference | Other requests | Sign-in |
|---|---|---|---|
| `codex` | `https://chatgpt.com/backend-api/codex/responses` | `…/codex/models`, `…/wham/usage` | `https://auth.openai.com` |
| `grok-build` | `https://cli-chat-proxy.grok.com/v1/chat/completions` | `…/v1/models`, `…/v1/billing`, `…/v1/user` | `https://auth.x.ai` |
| `claude-code` | The CLI's own traffic | `https://api.anthropic.com/api/oauth/usage` | None: the account is a setup token |

Codex sends a request over a WebSocket to its Responses endpoint first, as one
`response.create` message. If the socket fails before its first event (it
cannot connect, the handshake is refused, or it closes), the same request goes
over server-sent events instead; a failure after the first event is the run's
failure. A WebSocket that cannot connect at all, because its connection fails
or times out, sends the provider's later requests over server-sent events at
once, so that a network that blocks WebSockets does not cost every request a
failed connect. The WebSocket connect waits at most 10 seconds, and a
server-sent events request waits at most 20 seconds for its response headers.
The WebSocket client is tokio-tungstenite, because the handshake must carry
Codex's own headers. Grok Build sends Chat Completions through the chat proxy
that Grok's own CLI uses.

## Provider contract

The contract has two roles. A **provider** is shared: one exists for each entry
and account, and every user and request of that entry uses it. A **runtime**
belongs to one session and runs that session's inference requests, one at a
time.

```text
provider   shared by every user and request of one entry and account
  identity, and whether it needs a process Host
  authentication and runtime status
  model directory, read fresh on each call
  reading its vendor's failure records
  quota, and the accounts of a subscription family
  build a runtime ------------------------------------+
                                                      v
runtime    one per session, on the user's shard
  run a request -> events, then the run ends
  fork -> an independent runtime with the same configuration
  close -> release what outlives a run, such as a kept CLI process
```

The split follows who uses what. Credentials, catalogs and quota serve every
user of a shared instance and the backend's request handlers (status, probes,
logins), so a provider can be used from any thread. A run serves one turn and,
for Claude Code, drives a process reached through the user's Host connection,
so a runtime lives on the user's shard and never leaves it
([The user shard](../architecture/concurrency.md#the-user-shard)). The backend
builds a runtime on the shard that will own it and gives it that shard's HTTP
client and, when the provider needs a process, the Host process interface to
start it with. The agent sees only runtimes; the shared provider stays with the
backend.

### A run

For example, a turn asks an Anthropic runtime for an answer. The run streams
reasoning, text and two tool calls, then ends with a response that carries the
request's token usage. The agent runs both tools and starts a second run with
their results. Had the user stopped the turn during the first run, the turn
would have cancelled it: the runtime drops its HTTP connection, and the stream
ends with no further event.

A request carries the session, turn and request IDs, the model and its output
limit, the system prompt, the transcript as inference items, the tools, the
thinking setting, the service tier and a cancellation token.
[Request parameters](models.md#request-parameters) defines what those
parameters mean for each vendor.

A run yields these events:

| Event | Meaning |
|---|---|
| Thinking start | The model opened a reasoning block |
| Thinking text | A piece of reasoning text |
| Thinking signature | The vendor's signature over the reasoning, sent back with it in later requests |
| Redacted thinking | Reasoning the vendor sends only in encrypted form, sent back as received |
| Text | A piece of answer text |
| Tool call | A tool-use ID, the tool's name and its input as JSON |
| Response | The token usage of the run's final API call |
| Error | A failure: its message, a typed code, its diagnostics and the wait the vendor asked for |

Signatures and redacted reasoning are sent back only to the vendor that
issued them, because a vendor refuses a signature it did not sign. Each
provider marks what it receives with its family's name so that it recognizes
its own on replay: `anthropic:`, `openai:`, `codex:` or `google:` before the
vendor's signature, or before the whole reasoning item a Responses stream
sends, which is what that API takes back. Thinking without a signature of its
own, or with another vendor's, is not replayed. Gemini requires the signature
of each function call it is sent, so a call without one of Google's, such as
history from another provider, is replayed as text, and so is its result.
Chat Completions signs nothing; a vendor that needs earlier thinking back
receives its text as `reasoning_content`
([Vendors from models.dev](#vendors-from-modelsdev)).

A run ends after its response, after an error, after the last tool call of a
batch, or when it is cancelled. An HTTP provider's tool calls are followed by
the response of the API call that produced them. Claude Code's CLI needs the
results before it can finish, so its run ends after the batch
([Tool-call batches](claude-code.md#tool-call-batches)). Cancellation ends a run
without an event: the run stops its work at once, dropping its connection or
closing its process, and the session records the abort. Dropping the stream at
any point is cancellation. A run reports every failure as its error event and
never fails outside the stream.

A run is one inference attempt; retrying and resuming belong to the agent
([Retries](../agent/failures-and-recovery.md#retries)). The one repeat inside a
run belongs to credentials: a subscription provider whose request is refused
with HTTP 401 before any event refreshes the account's token once and sends
the request again.

A run takes no input once it starts. Steers and agent messages reach the model
with the next request ([Input](../agent/runtime.md#input)).

### Failures

A failure's typed code is what the agent's retry policy reads, and its
diagnostics keep the vendor's failure record
([The failure record](../agent/failures-and-recovery.md#the-failure-record)).
A failure to decode or follow the vendor's protocol, such as a malformed frame
or an unexpected CLI line, gets its code from a fixed rule, never from its
message text, and the agent never retries it automatically.

The provider reads its own vendor's failure records: given a stored record, it
returns the facts Demi shows, such as when a request can succeed again. The
same reader sets a failure's wait when a run fails and reads stored error
blocks when the backend sends a transcript
([Reading a failure](../agent/failures-and-recovery.md#reading-a-failure)).

A message that Demi composes about a failure never contains a credential.

### Runtime forks and closing

A runtime fork is an independent runtime with the same configuration. It
shares what the provider shares (configuration, authentication, quota and
catalog) and none of the source runtime's execution state: no connection,
process, pending tool call or continuation. Either one can be closed without
affecting the other. Compaction runs on a session copy whose runtime is a fork
([Session copy](../agent/compaction.md#session-copy)).

Closing a runtime releases what outlives a run, such as a kept CLI process.
Releasing a process on another machine takes time, so closing is an explicit,
awaited step: when the session is disposed, and when the backend builds the
session a new runtime. A runtime that is dropped without being closed still has
its process killed, without waiting for it.

### Reading vendor input

Everything a vendor sends is untrusted input: stream frames, WebSocket
messages, CLI output lines, OAuth responses and stored secret documents. Each
is decoded into a type where it enters
([Validation at entry](../architecture/contracts.md#validation-at-entry)),
with these rules for vendor payloads:

- **Two steps by tag.** A payload tagged by `type` is decoded by reading the
  tag first. An unregistered tag is ignored, so a vendor that ships a new event
  does not break a live stream. A registered tag with a malformed payload is an
  error that names the field's path. A payload without a tag is an error.
- **Only what Demi reads.** Types describe the fields Demi reads. An item that
  is sent back to the vendor whole, such as a Responses reasoning item, keeps
  the fields Demi does not read.
- **Reported fields and counts.** A field Demi only reports, such as a vendor
  error's message or request ID, reads as absent when it is not a string. A
  token count is a whole number; a fraction or a string is an error.
- **OAuth durations.** A duration in an OAuth response is a number or a string
  of digits, because servers state it either way, wherever it appears.
- **Token claims.** Claims read from a vendor's token are decoded without
  verifying its signature and validated like any other input. Demi reads only
  the account's identity and the token's expiry from them.
- **Secrets stay out of errors.** A secret document that fails to decode
  reports the field's path and the kind of error, never the decoder's message,
  which quotes the offending value.

Server-sent events are framed by the eventsource-stream library, which follows
the specification, with one addition: a final event that the vendor ends
without its blank line is kept, because vendors end bodies that way. Frames
without data (comments, `event: ping`, an empty `data:`) are not dispatched. A
Responses stream that ends without its completion event still ends the run with
a response, with zero usage.

A request body that carries media is serialized off the user's shard, because
megabytes of base64 would hold up every conversation of that user
([Blocking work](../architecture/concurrency.md#blocking-work)).

## Inference admission and runtime ownership

[Instance mode](../product/product.md#instance-mode-shared-vs-isolated)
determines provider scope: shared instances use the master's entries; isolated
instances use each user's own. The backend checks scope and resolves the
current entry at each inference boundary. A missing, deleted or inaccessible
entry refuses the request, and closes the session's runtime, before the request
takes a slot of the [request rate limit](usage-and-quota.md#rate-limit). A
subscription entry without an account is unauthenticated, and its request is
refused before inference.

The backend wraps each session's runtime so that the rate limit applies before
every run and the usage of every response reaches the
[usage ledger](usage-and-quota.md#usage-ledger).

Each session owns its runtime and lends it to the running turn. It keeps that
runtime while the entry's configuration and label, the entry's active account,
the selected model and the Host that runs the provider's process stay the same.
An edit of the entry, another active account, another model or another Host for
the process builds a new runtime at the next inference boundary and closes the
previous one. A running request finishes with the runtime it started with. The
new runtime uses the current request's model, thinking setting and service tier.

The backend builds providers from a fresh read of the entry and reuses a
provider only while the entry it read is unchanged, so a lookup that completes
after an edit cannot keep later requests on a stale configuration. Selecting
another account builds a new provider for that account and invalidates the
entry's model directory ([Catalog cache](models.md#catalog-cache)).

A provider declares whether it needs a process Host; admission uses that
declaration, not the provider's name. For such a provider the backend asks the
[placement](claude-code.md#where-it-runs) for the machine, which obtains or
wakes the user's Cloud before inference, and the conversation
[uses that Cloud as `provider`](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)
for as long as that provider is its selection.

## Credential vault

The backend's vault owns product scope, credential records, account operations
and login lifetime. Provider crates own authentication protocols, token refresh
and the shape of each family's secret document. The backend calls them, and it
necessarily handles plaintext credentials in memory to make authenticated
requests.

### Where credentials live

Every credential of the product is a record in the control store, sealed with a
key derived from the instance secret and bound to its row
([Storage](../backend/storage.md#control-records)). Nothing the product
authenticates with is a file:

| Record | Holds |
|---|---|
| `providers.config` | An API-key entry's key and endpoint settings |
| `provider_credentials` | One subscription account: public metadata, the sealed secret document, its usage snapshot, and a version |
| `providers.active_credential_id` | Which account the entry uses for inference |

The backend never reads a vendor's own login from the machine it runs on: not
`~/.codex`, `~/.grok`, the Claude Code keychain item, nor a vendor token in its
environment. Those belong to whoever operates the machine, and a multi-user
product must not lend them to a user whose entry has no account. An entry
without an account is unauthenticated. The instance secret is deployment
configuration (`DEMI_INSTANCE_SECRET`); a single-backend deployment that does
not set it generates one into its data directory.

Because a credential is a record, every worker of a multi-worker deployment
reads the same accounts, the same active account and the same usage.

### Scope

A credential has no owner of its own. It belongs to a provider entry, and every
entry belongs to a user.
[Instance mode](../product/product.md#instance-mode-shared-vs-isolated) only
decides whose entries a user infers with: the master's on a shared instance,
their own on an isolated one. There is no ownerless entry and no second kind of
credential.

| | Shared instance | Isolated instance |
|---|---|---|
| Entries and accounts belong to | The master | Each user |
| Who adds, tests, refreshes, selects and removes accounts | The master | The owner |
| Who infers with the active account | Every user | The owner |
| Who sees account labels, plan and usage | The master | The owner |
| What a user who only infers sees | The entry and its models; no account, plan or usage | — |

Every credential operation resolves the entry first: management requires being
its owner, inference requires the entry to be the one the mode assigns to the
user. The pool the backend gives a provider is bound to one entry, so a
provider cannot list or read another entry's accounts.

On a shared instance the master's accounts are a shared resource, which
sharpens three rules:

- **Refresh.** Many users on many workers use the same account at once, so the
  versioned replace of [Token refresh](#token-refresh) is what keeps one user's
  refresh from invalidating everyone else's tokens. On an isolated instance the
  rule is the same and rarely contended.
- **Usage.** The vendor's windows are consumed by all users together. The
  snapshot describes the account, not a user; per-user consumption is the usage
  ledger's, not the vendor's ([Usage and quota](usage-and-quota.md)).
- **Disclosure.** A process provider sends the active account's token to the
  Cloud of the user who infers
  ([Execution boundary](claude-code.md#execution-boundary)), and to no paired
  device. On an isolated instance that is the owner's token on the owner's
  Cloud. On a shared instance it is the master's token on a Cloud another user
  has a shell in: every user who can infer with a Claude Code entry can read
  its token. Demi allows this and does not hide it; a master who adds a Claude
  Code account to a shared instance is trusting every user with that account.
  Network providers (Codex, Grok Build, API keys) send a credential only to
  their vendor.

### Subscription secrets

A subscription account's secret is one strictly typed document that Demi
defines for its family; it is not the vendor's own file format. It holds what
the family's requests and refreshes need:

| Family | The secret document holds |
|---|---|
| `codex` | The ChatGPT sign-in's tokens, the account they act for, and the time of the last refresh |
| `grok-build` | The OAuth tokens and their expiry, the issuer and client that issued them, the team or organization the tokens act for, and the user's ID and email |
| `claude-code` | The setup token |

A field the type does not name is an error, as is a missing required one. The
provider
validates the document on every read and validates a refreshed document before
it is stored. A corrupt document is refused, never repaired: requests with that
account fail with an authentication error.

### The credential pool contract

A provider crate does not know where credentials live. It receives a
**credential pool** bound to one entry and reads and refreshes its account's
secret document through it:

| Operation | Meaning |
|---|---|
| List, read metadata | Public metadata of this entry's accounts: ID, label, detail, identity key, source, update time |
| Read and set the active account | The account the entry infers with |
| Write | Insert an account, or replace the account with this ID |
| Read a document | The account's secret document and its version, or nothing |
| Replace a document | Store a refreshed document only if the version read is still current |
| Take a refresh turn | Wait until no other refresh of this account runs in this backend |
| Remove | Delete the account |

The backend's pool is the `provider_credentials` table. Every secret write
advances the account's version, and a write into an entry without an active
account also selects the account, in the same transaction. A login in
progress uses a pool held in memory until it completes
([Login and publication](#login-and-publication)).

### Token refresh

OAuth refresh tokens are single-use, so two refreshers of one account race.
Every family that refreshes follows one protocol:

1. Take the account's refresh turn. Within one backend, refreshes of one
   account run one at a time, first come first served; a failed refresh
   releases the turn like a successful one.
2. Read the stored document again. If it does not need a refresh, because
   another refresher stored new tokens while this one waited, use it.
3. Otherwise ask the vendor. On success, validate the new document and replace
   the stored one at the version read; if that replace finds a newer version,
   use the stored tokens. On failure, read again: if the version moved, use the
   tokens the winner stored; if not, report the failure.

The versioned replace is what keeps the workers of a multi-worker deployment,
and the users of a shared account, from invalidating each other's tokens. The
refresh turn only saves needless vendor calls within one backend.

| Family | Refreshes when |
|---|---|
| `codex` | A request was refused with HTTP 401 and its access token is still the stored one, or the stored access token expires within 5 minutes, or the sign-in was last refreshed 8 or more days ago |
| `grok-build` | A request was refused with HTTP 401 and its access token is still the stored one, or the stored access token expires within 5 minutes; a sign-in without a refresh token is used as it is |
| `claude-code` | Never: a setup token is used as it is |

A refused request names the token it was refused with, so a refresher that
waited behind another finds the other's new tokens no longer due and uses
them.

### An account is the unit

A provider stands for one entry and one credential: an API-key entry's key, or
one subscription account named by its ID. The entry's active account is only
the default the backend uses for inference. Everything else names the account
it means:

| Operation | Account |
|---|---|
| Inference, model discovery | The active account |
| Connection test | The account the user chose |
| Usage probe, plan | The account whose quota is requested |
| Token refresh | The account being used |

A provider never changes accounts, so a request's quota observations land on
the account that made it, and a request that outlives a switch cannot credit
the wrong account ([Vendor quota](usage-and-quota.md#vendor-quota)). Selecting
another account changes which provider the next request's runtime is built
from; a running request finishes with its own. For Claude Code, the switch
closes the kept CLI process, and the next request starts a new one with the
newly active account's token
([Process lifetime](claude-code.md#process-lifetime)).

### Login and publication

An account enters an entry in one of three ways:

| Family | Login |
|---|---|
| `codex` | Codex device login: the user opens `https://auth.openai.com/codex/device` on any device and enters a one-time code |
| `grok-build` | OAuth device authorization (RFC 8628) at `https://auth.x.ai`: the user opens the verification address and approves |
| `claude-code` | Setup-token import: the user runs `claude setup-token` on a machine with Claude Code and pastes the token |

A device login works on a headless or remote backend, because the user
completes it in any browser; Demi never starts a vendor CLI to log in. The
flows are written by hand on shared OAuth pieces, not with the `oauth2` crate,
which refuses the vendors' token responses: they omit `token_type` and state
`expires_in` as a string.

- **Codex** asks `https://auth.openai.com/api/accounts/deviceauth/usercode`
  for a user code (a 404 means device login is unavailable), polls
  `…/api/accounts/deviceauth/token` at the interval the server names (403 and
  404 mean the user has not confirmed yet; any other failure ends the login),
  and exchanges the result for tokens at `…/oauth/token`.
- **Grok Build** requests a device code at
  `https://auth.x.ai/oauth2/device/code` with Grok Build's client and its fixed
  scopes. It prefers the complete verification address, which carries the
  code, and accepts only an `https` address, or `http` on a loopback host,
  without control characters. It polls at the server's interval and at least
  once a second, continues on `authorization_pending`, waits 5 seconds longer
  after `slow_down`, and ends the login on any other error. Once the user
  confirms, it reads the user's ID and email from the chat proxy's `/v1/user`
  when that answers. When the tokens act for a team or organization, the team
  or organization is the account's user, and the account has no email.

A flow authenticates against a staged pool held in memory; nothing is stored
until it completes. Completion is one control-store transaction:

- for a new entry: insert the provider row, subject to the owner-and-family
  uniqueness rule, its first account, and the active selection;
- for an existing entry: insert or replace the account, and select it only when
  the entry had no active account.

Within its entry, an account is identified by an identity key that the family
derives from the account, so logging in again with the same account replaces
its record instead of adding a second one. Codex keys an account by its
ChatGPT account ID; Grok Build by its issuer and user ID, else its email, else
its OAuth client.

A flow that loses the uniqueness check, fails or is cancelled has stored
nothing, so there is nothing to clean up. A device login expires after ten
minutes, whatever the vendor's code allows. Cancelling a login stops it at
once, even while it waits between polls, and backend shutdown cancels and
drains active flows. Logging into an
existing entry reserves the entry until the login completes or fails; another
change to the entry meanwhile is refused as busy. A failed token import answers
with a fixed message that never contains the supplied token.

Selecting an account is explicit. Removing the active account is refused:
select another account first, or delete the provider. Deleting a provider
deletes its accounts in the same transaction. Public provider responses expose
configuration metadata, account metadata and usage, never token material. The
routes are listed in [Web API](../product/web-api.md#subscription-accounts).

## Usage and quota

The backend meters every response event into the usage ledger, applies a
per-user request rate limit before inference, and keeps each subscription
account's vendor quota snapshot with its record. All three are defined in
[Usage and quota](usage-and-quota.md).
