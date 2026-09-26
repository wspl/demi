# Add a provider

This guide adds a provider crate: the code that connects Demi to one more
inference service. The crate implements the
[provider contract](../providers/providers.md#provider-contract); read that
first, because this guide shows where each of its rules lands in code instead of
restating them.

Before you write a crate, check whether the service needs one:

- A service that speaks a wire Demi already reads (OpenAI Responses or Chat
  Completions, Anthropic Messages, Gemini `generateContent`) is an entry of that
  family with its own base URL
  ([Endpoints](../providers/providers.md#endpoints)).
- A vendor that models.dev lists with a supported client package appears in the
  vendor catalog without any code
  ([Vendors from models.dev](../providers/providers.md#vendors-from-modelsdev)).
- A field that one vendor requires or refuses is a typed setting of the
  backend's vendor policy, not a crate.

Write a crate when the service has its own wire, its own login, or runs as a
process.

## Create the crate

1. Create the library crate `crates/provider-<vendor>`.
2. Depend on `provider`, which holds the contract and the shared building
   blocks; a provider that starts a process also depends on `shell` for the
   Host process interface. Never depend on what sits above a provider: the
   agent, the backend or a Host implementation.
3. Add the crate and its edges to the Rust graph in
   [Dependency graphs](../architecture/crates-and-packages.md#dependency-graphs),
   and its entry to [Crates](../architecture/crates-and-packages.md#crates).
   The [boundary check](../architecture/crates-and-packages.md#boundary-checks)
   refuses an edge that the graph does not list.

## Implement the provider

The provider is the shared half of the contract: one exists per entry and
account, and it is used from any thread.

```rust
/// One provider entry, shared by every user and request of the entry.
pub trait Provider: Send + Sync + 'static {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn capabilities(&self) -> Capabilities;              // { process_host: bool }
    fn auth_status(&self) -> BoxFuture<'_, AuthState>;
    fn runtime_state(&self) -> RuntimeState;
    /// A fresh read of the provider's directory; the backend's catalog cache is the cache.
    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>>;
    fn read_failure(&self, d: &ProviderErrorDiagnostics, received_at: Timestamp) -> ProviderFailureFacts;
    fn quota(&self) -> Option<&ProviderQuota>;
    fn accounts(&self) -> Option<&dyn SubscriptionAccounts>;
    /// Called on the shard that will own the runtime.
    fn runtime(&self, env: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError>;
}

pub struct RuntimeEnv {
    pub http: reqwest::Client,                       // the shard's client
    pub process_host: Option<Rc<dyn ProcessHost>>,   // required iff capabilities().process_host
}
```

| Method | What your provider does |
|---|---|
| `id`, `display_name` | Returns the entry's identity as the backend configured it |
| `capabilities` | Sets `process_host` only when the runtime starts a process on a Host; the backend then places that process and passes its interface in `RuntimeEnv` |
| `auth_status`, `runtime_state` | Says whether the credential is present and usable, and whether the provider can run. Reading either never makes an inference request |
| `list_models` | Reads the vendor's model directory on each call and keeps no cache: the backend's [catalog cache](../providers/models.md#catalog-cache) is the only one |
| `read_failure` | Reads your vendor's own fields out of a stored failure record ([Report failures](#report-failures)) |
| `quota` | Returns your quota when the vendor reports one ([Optional: quota](#optional-quota)) |
| `accounts` | Returns the account operations of a subscription family ([Credentials](#credentials)) |
| `runtime` | Builds a runtime for one session, on the calling thread, with the client and process interface in `env` |

Keep what runtimes share (configuration, authentication, quota, the catalog
client) in one `Arc` that the provider and all its runtimes hold.

## Implement the runtime

The runtime is the per-session half. It lives on the user's shard, so it has no
`Send` bound, and it runs one request at a time: `run` borrows it.

```rust
/// A session's inference runtime. One run at a time: `run` borrows the runtime.
pub trait ProviderRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_>;
    /// Same configuration, none of this runtime's execution state.
    fn fresh(&self) -> Box<dyn ProviderRuntime>;
    /// Releases what outlives a run (a retained CLI): kill and reap.
    fn close(&mut self) -> LocalBoxFuture<'_, ()>;
}
pub type ProviderRun<'a> = LocalBoxStream<'a, ProviderEvent>;

pub struct InferenceRequest {
    pub session_id: String, pub turn_id: String, pub request_id: String,
    pub model_id: String, pub output_limit: Option<NonZeroU32>,
    pub system_prompt: String, pub items: Arc<[InferenceItem]>,
    pub tools: Arc<[ToolDefinition]>, pub thinking: Option<ThinkingConfig>,
    pub service_tier_id: Option<String>, pub cancel: CancellationToken,
}

pub enum ProviderEvent {
    ThinkingStart, ThinkingDelta(String), ThinkingSignature(String),
    RedactedThinking(String), TextDelta(String), ToolCall(ToolCall),
    Response(TokenUsage),           // usage of the single final API request
    Error(ProviderFailure),         // terminal
}
pub struct ToolCall { pub tool_use_id: String, pub tool_name: String, pub input: serde_json::Value }
```

Write `run` as a stream that keeps the rules of
[A run](../providers/providers.md#a-run):

1. Build the request body from the request and the entry's typed
   configuration. Map the thinking setting, the output limit and the service
   tier as [Request parameters](../providers/models.md#request-parameters)
   defines for your vendor. Serialize a body that carries media on the blocking
   pool, not on the shard
   ([Blocking work](../architecture/concurrency.md#blocking-work)).
2. Send it once, with the client from `RuntimeEnv`. Never build a client of your
   own: pooled connections belong to the Tokio runtime that created them
   ([Programs and threads](../architecture/concurrency.md#programs-and-threads)).
3. Yield events as the vendor streams them. End after `Response`, after
   `Error`, or after the last `ToolCall` of a batch when the vendor needs the
   results before it can finish, and yield nothing after that.
4. Wait on the vendor inside `select!` against `request.cancel`. When the token
   fires, drop the connection or stop the process, and end the stream without
   an event. Dropping your stream must stop the work too
   ([Cancellation and cleanup](../architecture/concurrency.md#cancellation-and-cleanup)).
5. Report every failure as `Error`. Never panic, and never fail outside the
   stream.
6. For a subscription family, when the vendor answers HTTP 401 before any
   event, refresh the account's token once and send the request again. That is
   the only repeat a run makes.

`fresh` returns a runtime that shares the provider's `Arc` and nothing of this
runtime's execution state: no connection, process, pending tool call or
continuation
([Runtime forks and closing](../providers/providers.md#runtime-forks-and-closing)).
`close` releases what outlives a run; an HTTP runtime has nothing to release. A
runtime that keeps a process between runs follows
[Claude Code](../providers/claude-code.md#process-lifetime): the process moves
into the run and back to the runtime only where the conversation can continue
in it, so every other exit drops it and the Host kills it.

## Read the vendor's stream

Everything the vendor sends is untrusted input, decoded at the point of entry
under the rules of
[Reading vendor input](../providers/providers.md#reading-vendor-input). Reuse
the building blocks of the `provider` crate instead of writing your own:

| Need | Use |
|---|---|
| Server-sent events | `sse_data`: the data string of each frame, with the trailing-frame rule |
| Payloads tagged by `type` | `decode_tagged` with a `tagged_wire!` list of the tags you map; `Tagged<T>` where a tagged value sits inside another |
| OpenAI Responses or Chat Completions | `responses::map_sse` and `chat_completions::map_sse`, which decode and map the whole stream; your crate writes only its request body, and your vendor's name is only a label in error text |
| Fields you only report | `ReportedString` |
| OAuth responses | `decode_json_response`, `OAuthSeconds`, `PollInterval`, `Lifetime` |
| Claims of a vendor token | `jwt_claims` |
| Model limits in a catalog | `ModelLimits` |

Describe only the fields you read. On a value you send back to the vendor
whole, keep the fields you do not read in a `#[serde(flatten)]` map. Token
counts are `Option<u64>`.

## Report failures

- A response with a failure status becomes a failure through `http_failure`,
  which reads the body and builds the message
  (`<label> API request failed with HTTP <status>: <body>`), the failure record
  and the wait. Pass it your failure reader.
- Implement `read_failure` for your vendor's own fields, such as a reset time
  in its error body, and fall back to the standard `Retry-After` reading
  (`read_http_failure`). The same function sets a failure's wait when a run
  fails and reads stored records later
  ([Reading a failure](../agent/failures-and-recovery.md#reading-a-failure)).
- Give decode, protocol and process failures their own `thiserror` types and
  turn them into a failure at the run's boundary with a fixed code. Never derive
  a code by matching message text
  ([Failures](../providers/providers.md#failures)).
- A message you compose never contains a credential.

## Credentials

An API-key family reads its key from the entry's typed configuration, which the
backend passes in. It reads no key, base URL or option from environment
variables.

A subscription provider is built for one account: its factory receives the
entry's credential pool and the account's ID
([The credential pool contract](../providers/providers.md#the-credential-pool-contract)).

1. Define your family's secret document as a strict serde type
   (`deny_unknown_fields`) that holds what your requests and refreshes need
   ([Subscription secrets](../providers/providers.md#subscription-secrets)). Its
   decode errors report the field's path and the kind of error, never the
   decoder's message.
2. Read the account's document through the pool when a request needs it.
3. Refresh with the shared `renew` protocol. You supply two things: whether a
   stored document still needs a refresh, and the vendor call that refreshes
   it. `renew` takes the account's refresh turn, reads again, calls the vendor,
   validates the result and replaces the document at the version it read
   ([Token refresh](../providers/providers.md#token-refresh)).
4. Implement your family's account kit: how to label an account from its
   document (a label, a detail and the identity key), and your login flow or
   token import. Listing, selecting, importing and removing accounts are the
   shared `Accounts<K>` operations.
5. Write a device login by hand on `provider::oauth`. Report the verification
   address and code once, poll with `tokio::time::sleep` so that dropping the
   login's future cancels it at once, and return the new document
   ([Login and publication](../providers/providers.md#login-and-publication)).

## Optional: quota

When the vendor reports usage windows, implement a quota source: whether a
probe is free or costs a request, a probe that reads the plan, the account's
label and the windows, and an observation of a response's headers or a CLI
line. An observation is pure and never fails a run. `ProviderQuota::new`
connects your source to the account's snapshot store. What a snapshot holds and
how probes and observations merge is defined in
[Vendor quota](../providers/usage-and-quota.md#vendor-quota).

## Register the family

The backend's family registry (`FamilyRegistry::builtins`) maps each family to
its credential kind, an API key or a subscription account, and to a factory.
The factory builds your provider from the entry's configuration and the
models.dev client and, for a subscription family, from the entry's pool, the
account's ID and the account's quota snapshot store. Add your family there.
Then:

- If models.dev lists vendors for your wire, add the mapping from their client
  package to your family
  ([Vendors from models.dev](../providers/providers.md#vendors-from-modelsdev)).
- The family is a value of the REST contract, so regenerate the browser's
  contracts ([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
  How users add your family's entries (a key, a device login or a token import)
  is a `web-ui` change that lands in `web` and the gallery together.

## Test it

No automated test calls a real model. Tests feed your code inline vendor
streams, and the `provider` crate's `testing` feature supplies the fakes:

| Fake | What it does |
|---|---|
| `MockVendor` | An in-process HTTP server that scripts status, headers, body chunks and delays, and records each request |
| `sse_body` | Builds a server-sent events body from a list of payloads |
| `FakeWebSocket` | A WebSocket server that records the handshake headers, scripts frames and records the close reason |
| `ScriptedCli` | A process Host whose process scripts its output, reacts to its input, and counts kills and waits |
| `ScriptedRuntime` | A runtime with scripted runs, for tests of what sits above providers |
| `FixedClock`, `jwt` | A fixed wall clock; a token with the claims you give it |
| `MemoryCredentialPool` | A credential pool held in memory, for account and refresh tests |

Run timeouts, polling intervals and freshness windows on Tokio's paused clock
([Tests and time](../architecture/concurrency.md#tests-and-time)). Cover at
least:

- each way a run ends, including cancellation that drops the connection, and
  no event after the last one;
- an unregistered tag that is ignored, and a malformed registered payload that
  fails with its field's path;
- the failure record, its message and the facts your reader returns;
- a refresh race over the memory pool, in which the loser uses the winner's
  tokens;
- your login flow against `MockVendor`.

A check against the real vendor is run by hand during acceptance, never in the
test suite.
