# Add a provider

This guide adds a provider package: the code that connects Demi to one more
inference service. The package implements the
[provider contract](../providers/providers.md#provider-contract); read that
first, because this guide shows where each of its rules lands in code instead of
restating them.

Before you write a package, check whether the service needs one:

- A service that speaks a wire Demi already reads (OpenAI Responses or Chat
  Completions, Anthropic Messages, Gemini `generateContent`) is an entry of that
  family with its own base URL
  ([Endpoints](../providers/providers.md#endpoints)).
- A vendor that models.dev lists with a supported client package appears in the
  vendor catalog without any code
  ([Vendors from models.dev](../providers/providers.md#vendors-from-modelsdev)).
- A field that one vendor requires or refuses is a typed setting of the
  backend's vendor policy, not a package.

Write a package when the service has its own wire, its own login, or runs as a
process.

## Create the package

1. Create the package `internal/providers/<vendor>`.
2. Import `internal/provider`, which holds the contract and the shared building
   blocks; a provider that starts a process also imports `internal/host` for the
   Host process interface. Never import what sits above a provider: the
   agent, the backend or a Host implementation.
3. Add the package and its edges to the Go graph in
   [Dependency graphs](../architecture/packages.md#dependency-graphs),
   and its entry to [Packages](../architecture/packages.md#go-package-boundaries).
   The [boundary check](../architecture/packages.md#boundary-checks)
   refuses an import that the graph does not list.

## Implement the provider

The provider is the shared half of the contract: one exists per entry and
account, and any goroutine may use it.

```go
// Provider is one provider entry, shared by every user and request of the entry.
type Provider interface {
	Capabilities() Capabilities // {ProcessHost bool}
	AuthStatus(context.Context) core.AuthState
	RuntimeState() core.RuntimeState
	// ListModels reads the provider's directory afresh; the backend's catalog cache is the cache.
	ListModels(context.Context) (core.ProviderModelList, error)
	ReadFailure(*core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts
	Quota() *Quota
	Accounts() SubscriptionAccounts
	// Runtime is called on the shard that will own the runtime.
	Runtime(RuntimeEnv) (Runtime, error)
}

// RuntimeEnv supplies the HTTP client of the runtime's owner.
type RuntimeEnv struct{ HTTP *http.Client }
```

| Method | What your provider does |
|---|---|
| `Capabilities` | Sets `ProcessHost` only when the runtime starts a process on a Host; the backend then builds the runtime over a placement that starts the process ([How a runtime gets its process](../providers/claude-code.md#how-a-runtime-gets-its-process)) |
| `AuthStatus`, `RuntimeState` | Says whether the credential is present and usable, and whether the provider can run. Reading either never makes an inference request |
| `ListModels` | Reads the vendor's model directory on each call and keeps no cache: the backend's [catalog cache](../providers/models.md#catalog-cache) is the only one |
| `ReadFailure` | Reads your vendor's own fields out of a stored failure record ([Report failures](#report-failures)) |
| `Quota` | Returns your quota when the vendor reports one, else nil ([Optional: quota](#optional-quota)) |
| `Accounts` | Returns the account operations of a subscription family, else nil ([Credentials](#credentials)) |
| `Runtime` | Builds a runtime for one session, in the calling goroutine, with the shard's client in the `RuntimeEnv`; a provider whose runtime starts a process refuses, since only the backend chooses the machine |

Keep what runtimes share (configuration, authentication, quota, the catalog
client) in one value that the provider and all its runtimes point to.

## Implement the runtime

The runtime is the per-session half. It lives on the user's shard and runs one
request at a time.

```go
// Runtime runs one session's requests, one at a time. Close releases retained resources.
type Runtime interface {
	Run(context.Context, InferenceRequest) Run
	Fresh() Runtime // same configuration, none of this runtime's execution state
	Close(context.Context) error
	RequestLimits(core.Model) RequestLimits
}

// Run yields the events of one inference attempt.
type Run = iter.Seq[Event]

type InferenceRequest struct {
	SessionID, TurnID, RequestID, ModelID string
	OutputLimit   *uint32 // the model's
	OutputCap     *uint32 // the request's own, below the model's
	SystemPrompt  string
	Items         []InferenceItem
	Tools         []ToolDefinition
	Thinking      core.ThinkingConfig
	ServiceTierID *string
	PromptCache   PromptCache
}
```

An `Event` is one of `*ThinkingStart`, `*ThinkingDelta`, `*ThinkingSignature`,
`*RedactedThinking`, `*TextDelta`, `*ToolCall` (with `ToolUseID`, `ToolName`
and the raw JSON `Input`), `*Response` (the usage of the single final API
request) and `*Error` (terminal, carrying a `Failure`).

Write `Run` as an iterator that keeps the rules of
[A run](../providers/providers.md#a-run):

1. Build the request body from the request and the entry's typed
   configuration. Map the thinking setting, the output limit and the service
   tier as [Request parameters](../providers/models.md#request-parameters)
   defines for your vendor; the output limit you send is
   `request.MaxOutputTokens()`, which lowers the model's limit to the
   request's cap. Build the body with `provider.EncodeBody`, in the run's own
   goroutine and outside any state lock
   ([Blocking work](../architecture/concurrency.md#blocking-work)).
2. Send it once, with the client from `RuntimeEnv`. Never build a client of your
   own: the shard's client owns the connection pool and its lifetime
   ([Programs and threads](../architecture/concurrency.md#programs-and-threads)).
3. Yield events as the vendor streams them. End after `*Response`, after
   `*Error`, or after the last `*ToolCall` of a batch when the vendor needs the
   results before it can finish, and yield nothing after that.
4. Pass the run's context to every wait: the request, every read of the
   response body, and the process. When it ends, close the connection or stop
   the process and return without an event. When `yield` returns false, the
   consumer has stopped iterating: stop the work the same way
   ([Cancellation and cleanup](../architecture/concurrency.md#cancellation-and-cleanup)).
5. Report every failure as an `*Error` event. Never panic, and never fail
   outside the iterator.
6. For a subscription family, when the vendor answers HTTP 401 before any
   event, refresh the account's token once and send the request again. That is
   the only repeat a run makes.

`Fresh` returns a runtime that shares the provider's value and nothing of this
runtime's execution state: no connection, process, pending tool call or
continuation
([Runtime forks and closing](../providers/providers.md#runtime-forks-and-closing)).
`Close` releases what outlives a run; an HTTP runtime has nothing to release. A
runtime that keeps a process between runs follows
[Claude Code](../providers/claude-code.md#process-lifetime): the process moves
into the run and back to the runtime only where the conversation can continue
in it, so every other exit stops it and the Host kills it.

## Read the vendor's stream

Everything the vendor sends is untrusted input, decoded at the point of entry
under the rules of
[Reading vendor input](../providers/providers.md#reading-vendor-input). Reuse
the building blocks of `internal/provider` instead of writing your own:

| Need | Use |
|---|---|
| Server-sent events | `SSEData`: the data string of each frame, with the trailing-frame rule |
| Payloads tagged by `type` | `DecodeTagged` with a map from each tag you map to its decoder; an unregistered tag is reported as absent |
| OpenAI Responses or Chat Completions | `ResponsesSSEEvents` with `MapResponsesEvents`, and `MapChatSSE`, which decode and map the whole stream; your package writes only its request body, and your vendor's name is only a label in error text |
| Fields you only report | `Reported[T]`, `ReportedString` |
| OAuth responses | `DecodeJSONResponse`, `OAuthSeconds`, `PollInterval`, `Lifetime` |
| Claims of a vendor token | `JWTClaims` |

Describe only the fields you read, as contract types with generated decoders
([Contracts](../architecture/contracts.md)). On a value you send back to the
vendor whole, keep it as raw JSON and read your fields with
`contract.ObjectFields`, so the rest goes back unchanged. Token counts are
`*uint64`.

## Report failures

- A response with a failure status becomes a failure through
  `provider.HTTPFailure`, which reads the body and builds the message
  (`<label> API request failed with HTTP <status>: <body>`), the failure record
  and the wait. Pass it your failure reader.
- Implement `ReadFailure` for your vendor's own fields, such as a reset time
  in its error body, and fall back to the standard `Retry-After` reading
  (`provider.ReadHTTPFailure`). The same function sets a failure's wait when a
  run fails and reads stored records later
  ([Reading a failure](../agent/failures-and-recovery.md#reading-a-failure)).
- Turn decode, protocol and process failures into a `Failure` at the run's
  boundary with a fixed code (`ProtocolFailure`, `TransportFailure`,
  `EventStreamFailure` and the other constructors in `internal/provider`).
  Never derive a code by matching message text
  ([Failures](../providers/providers.md#failures)).
- A message you compose never contains a credential.

## Credentials

An API-key family reads its key from the entry's typed configuration, which the
backend passes in. It reads no key, base URL or option from environment
variables.

A subscription provider is built for one account: its family receives the
entry's credential pool and the account's ID
([The credential pool contract](../providers/providers.md#the-credential-pool-contract)).

1. Define your family's secret document as a strict contract type that holds
   what your requests and refreshes need
   ([Subscription secrets](../providers/providers.md#subscription-secrets)),
   and decode it with `provider.DecodeSecretDocument`, whose errors report the
   field's path and the kind of error, never the decoder's message.
2. Read the account's document through the pool when a request needs it
   (`provider.ReadSecret`).
3. Refresh with `provider.Renew`. You supply two functions: whether a stored
   document still needs a refresh, and the vendor call that refreshes it.
   `Renew` takes the account's refresh turn, reads again, calls the vendor,
   validates the result and replaces the document at the version it read
   ([Token refresh](../providers/providers.md#token-refresh)).
4. Implement your family's `provider.AccountKit`: how to label an account from
   its document (a label, a detail and the identity key), and your login flow
   or token import. Listing, selecting, importing and removing accounts are
   the shared operations of `provider.NewAccounts`.
5. Write a device login by hand on the OAuth pieces of `internal/provider`.
   Report the verification address and code once, wait between polls in a
   `select` on a timer and the login's context, so that ending the context
   cancels the login at once, and return the new document
   ([Login and publication](../providers/providers.md#login-and-publication)).

## Optional: quota

When the vendor reports usage windows, implement a `provider.QuotaSource`:
whether a probe is free or costs a request, a probe that reads the plan, the
account's label and the windows, and an observation of a response's headers or
a CLI line. An observation is pure and never fails a run. `provider.NewQuota`
connects your source to the account's snapshot store. What a snapshot holds and
how probes and observations merge is defined in
[Vendor quota](../providers/usage-and-quota.md#vendor-quota).

## Register the family

The backend's built-in families (`BuiltinFamilies` in
`internal/backend/builtins.go`) register each family, a
`providers.Family`: its credential kind, an API key or a subscription
account, its wires, and how it builds a provider. `Provider` builds yours from
the entry's configuration and the models.dev client and, for a subscription
family, from the entry's pool, the account's ID and the account's quota
snapshot store. Add your family there. Then:

- If models.dev lists vendors for your wire, add the mapping from their client
  package to your family (`offeredVendor` in
  `internal/backend/providers/catalog.go`;
  [Vendors from models.dev](../providers/providers.md#vendors-from-modelsdev)).
- The family is a value of the REST contract, so regenerate the web app's
  contracts ([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
  How users add your family's entries (a key, a device login or a token import)
  is a `web-ui` change that lands in `web` and the gallery together.

## Test it

No automated test calls a real model. Tests feed your code inline vendor
streams, and `internal/provider/providertest` supplies the fakes:

| Fake | What it does |
|---|---|
| `StartVendor` (`MockVendor`) | An in-process HTTP server that scripts status, headers, body chunks and delays, and records each request |
| `SSEBody` | Builds a server-sent events body from a list of payloads |
| `NewScriptedRuntime` | A runtime with scripted runs, for tests of what sits above providers |
| `FixedClock`, `NewManualClock`, `JWT` | A fixed or stepped wall clock; a token with the claims you give it |
| `provider.NewMemoryCredentialPool` | A credential pool held in memory, for account and refresh tests |

A WebSocket vendor's tests can follow Codex's `codextest.FakeWebSocket`, which
records the handshake headers, scripts frames and records the close reason; a
process provider's tests can follow Claude Code's scripted CLI Host in
`internal/providers/claudecode/cli_test.go`.

Run timeouts, polling intervals and freshness windows in `testing/synctest`
bubbles ([Tests and time](../architecture/concurrency.md#tests-and-time)).
Cover at least:

- each way a run ends, including cancellation that closes the connection, and
  no event after the last one;
- an unregistered tag that is ignored, and a malformed registered payload that
  fails with its field's path;
- the failure record, its message and the facts your reader returns;
- a refresh race over the memory pool, in which the loser uses the winner's
  tokens;
- your login flow against `MockVendor`.

A check against the real vendor is run by hand during acceptance, never in the
test suite.
