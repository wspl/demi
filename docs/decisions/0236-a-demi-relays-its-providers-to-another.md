---
status: accepted, superseded in part by 0245
---

# A Demi relays its providers to another, which keeps its conversations

## Problem

Alex runs two Demi servers: `demi.example.com`, signed in to a Codex and a
Claude Code subscription and holding an Anthropic API key, and a development
server on a laptop. To use Codex in a conversation on the laptop, Alex has to
sign in to Codex there too. Copying the sign-in does not work: each renewal of
a Codex or Claude Code sign-in revokes the refresh token it spends, so two
copies of one sign-in sign each other out at the first renewal of either.
Copying an API key works, but hands the raw key to whoever holds the copy, and
nothing on the first server can take it back or see what it spent.

## Decision

A server lends one of its providers to another server, which uses it as a
provider of its own while its conversations, tools and files stay where they
are. Only the provider's calls cross: each inference request, and each image,
video and audio file to make, goes to the lending server, which runs it with
its own provider and sends the answer back.

In the example, `demi.example.com` is the lender and the laptop the borrower.

### What the user sees

On the lender, a provider's header gains a Share button beside its name, as
a conversation's header has one. It opens a dialog of the same shape as Share
Conversation:

```text
Settings › Providers › Codex  [⇪]           [+ Add Account]  [on]
                               │
          ┌────────────────────┴──────────────────────────────┐
          │ Share Codex                                    ✕  │
          │ With a share key, another Demi server can use this │
          │ provider. Its conversations stay on that server,   │
          │ and only their model requests come here. A key is  │
          │ shown only once, when it's made.                   │
          │                                                    │
          │ [demi-share-key-eyJ1cmwiOiJodHRwczovL2RlbW…      ] │
          │ Keys                                               │
          │ ┌────────────────────────────────────────────────┐ │
          │ │ Not used yet                          [Revoke] │ │
          │ │ Expires in 7 days                              │ │
          │ │ Used by laptop.example.net           [Revoke…] │ │
          │ │ Last used 5 minutes ago                        │ │
          │ └────────────────────────────────────────────────┘ │
          │ [+ New Key]                             [Copy Key] │
          └────────────────────────────────────────────────────┘
```

- **Opening the dialog makes a key** when the provider has none, as Share
  Conversation makes its link: the key stands in a read-only field with
  Copy, already copied, as a new share's link is. New Key makes another the
  same way. A key is shown only in the dialog that made it: the server keeps
  its SHA-256 alone, as it keeps a browser's session token, so a key that is
  lost is revoked and replaced.
- **The keys below** are every key of the provider, newest first. A key that
  has been used shows the server that uses it, by the name that server gives
  when it connects, and when it last did. One that has not shows when it
  expires.
- **Expiry.** A key that is never used expires 7 days after it is made, so a
  key pasted into a chat and forgotten does not stay valid; a key used once
  stays valid until it is revoked. The limit is the one way a key ends by
  itself; 7 days is how long Discord keeps an invite by default.
- **Revoke** deletes a key and closes every connection that uses it. A key
  that is in use asks once first, since another server's provider stops
  working; one that is not is revoked at once, as Stop Sharing is.
- Deleting the provider deletes its keys.
- **Who shares**: whoever manages the provider, by the rules of the instance
  mode. A key works only while its creator still manages the provider.
- A borrowed provider has no Share button: a server never relays what it
  borrows, so no chain of servers forms.

On the borrower, Add Provider gains a row beside Compatible API:

```text
  Another Demi     Use a provider another Demi server shares
```

It creates a provider at once, as Compatible API does, named Another Demi,
with its Share Key shown as missing. Pasting the key connects: the provider
takes the lender's name for it, such as Codex, and lists the lender's models
for it. From then on it is a provider like any other, which conversations,
images, videos and audio choose.

What the borrower never learns: how many accounts the provider has, which
one answers, their email addresses, their quotas and the vendor's own
messages. Its provider page shows no accounts and no quota, the context
popover no account, and a failure the lender's own words (see Failures).

### The relay

```text
borrower (keeps the conversation)          lender (keeps the provider)
+------------------------------+  one WebSocket   +--------------------------+
| agent session                |  per session,    | key owner's user shard   |
|   run(request)               |  Bearer key      |   the entry's own        |
|   Relayed runtime  ----------+----------------->|   ProviderRuntime        |
|     sends what changed  <----+---- events ------+--   (Codex, Claude Code, |
|   tool calls run here        |                  |    an API key, ...)      |
+------------------------------+                  +--------------------------+
```

- **Where the cut is.** Every provider implements `Provider` and
  `ProviderRuntime` (`crates/provider-common/src/contract.rs`). The borrower
  gains one family, `relay`, whose runtime sends each `InferenceRequest` to
  the lender and turns what comes back into `ProviderEvent`s. The lender runs
  the request on the runtime of its own entry, so its rotation, its device
  egress and Claude Code's process placement work as they do for its own
  conversations. A tool call comes back as an event, as it does today, and
  the borrower's agent runs it on the conversation's Host.
- **One socket per session.** The relayed runtime opens a WebSocket to the
  lender with the session's first run, and its `close` closes it. The lender
  builds the entry's runtime when the socket opens and closes it when the
  socket closes, so a Claude Code process lives as long as the borrower's
  session. Cancel and `idle` are messages on it. The borrower never
  prepares a request ahead, so the lender's Claude Code process starts with
  the first run. A socket that drops mid-run ends the run: the borrower asks
  the lender why, so a revoked key says so, and anything else is a network
  failure, which the agent's retry handles. The next run opens a new socket,
  and Claude Code resumes its session from the blocks the request carries.
- **What changed only.** A request carries the whole transcript, media
  included, so a long conversation with screenshots resends megabytes on
  every tool round. The socket keeps the session's last request on both
  ends: a request sends the system prompt and the tools only when they
  changed, and the items and blocks as the count of the last request's that
  are unchanged, then the rest. When the prefix differs, as after a
  compaction, it sends them whole. The two ends hold the same last request
  because they hold it for the same socket.
- **Stateless reads** are HTTPS requests with the key as a bearer token: the
  provider's status, its models, and an image, a video or audio to make,
  whose bytes come back in the answer.
- **Media.** The borrower's provider makes the kinds of media the lender's
  made when the key was pasted, in the lender's shapes, which the borrower's
  entry keeps; a lender that later starts or stops making a kind is seen
  when a key is pasted again. Each kind has one maker on the borrower whose
  calls go under `/api/relay/media`: the maker with its models, an image, an
  audio file, a speech model's voices, a video's ask, its job's state, its
  file and its cancel. What the backend asks a maker between calls, such as
  the options a model's request has no field for, is what the lender said
  with its models. The lender sends each call on its entry's key, or its
  accounts in turn, as it sends its own, and writes no ledger row. It keeps
  a video's job under an id of its own until its video is downloaded or it
  is cancelled, so the vendor's id stays with the lender; revoking the key
  cancels it.

```rust
// crates/provider-relay, its wire
pub const VERSION: u32 = 1;

// Every relay request, the socket's included, carries the key as a bearer
// token and two headers: `demi-relay-version`, and `demi-relay-server`, the
// name the lender shows for the key, the host of the borrower's public URL.
// A lender of another version answers 409 with its own.

pub enum ClientMessage {
    Run { run: u64, request: WireRequest },
    Cancel { run: u64 },
    Idle,
}

pub enum ServerMessage {
    Event { run: u64, event: WireEvent },
    End { run: u64 },
}

pub struct WireRequest {
    pub system_prompt: Option<String>,   // none: as the last request's
    pub tools: Option<Vec<ToolDefinition>>,
    pub items: Delta<InferenceItem>,
    pub blocks: Delta<RequestBlock>,
    // the request's other fields, whole
}

pub enum Delta<T> {
    Whole { list: Vec<T> },
    After { kept: usize, appended: Vec<T> },
}
```

The protocol's types are a copy of the contract's that are its own, as the
runner's protocol is: a change of the contract changes the wire only when its
conversion does, and only a change of the wire raises `VERSION`. Both ends
require the same version. A borrower that meets another version shows which
server to update, and the two servers run different releases otherwise.

### Opacity

The lender strips, before anything leaves it:

- `Account` and `AccountSwitch` events, and the quota windows of
  `CallEvent::Observed`. The relayed runtime's `next_account` keeps none.
- A failure's diagnostics, the vendor's message and its wait, since a wait
  says when the lender's limit resets. The borrower gets the failure's code
  alone, under a message the lender composes, such as "The provider on
  demi.example.com reached its limit." The agent's retry still does what the
  code asks (`crates/agent-session/src/retry.rs`): a used-up limit is not
  retried, and a rate limit is retried at most three more times, under its
  own backoff of a few seconds instead of the vendor's wait.
- A media call's failure but its kind and code, a failed video's words, and
  the vendor's id of an image, a job or a call. A refused image reads "OpenAI
  on demi.example.com refused the request: no reason is shared". A refused
  shape keeps the shapes the vendor takes, such as `16:9`, which is what the
  agent asks again with.
- The entry's configuration: its endpoint, headers and keys.

The borrower's `Provider::quota` and `accounts` are none.

### Storage

- The lender's control database gains `provider_share_keys`: the key's
  SHA-256, its id, its entry, its creator, when it was made, and, once it
  is used, the name the server using it gave and when it last connected.
  Deleting the entry deletes them. "Share key" is the name in code as in
  the page; a conversation's share is another thing, and keeps its name.
- The lender keeps no usage of a key: a relayed run is not wrapped in the
  usage meter, so its usage ledger, whose rows are each a conversation's,
  gains no row and no column. A key's row shows only when it was last used.
  What the relayed calls spend still shows in the lender's quota windows,
  which the provider keeps from the vendor's answers, apart from the ledger.
- The borrower's entry is an API-key entry of the `relay` family: its
  `base_url` is the lender's public URL and its `api_key` the key, both sealed
  as every entry's configuration is. The share key the user pastes is one
  line, `demi-share-key-` and the base64url of `{"url": ..., "token": ...}`,
  which the borrower's server splits into the two.
- The borrower's own ledger records the relayed calls as it records any
  provider's, under its own user and conversation.

### What an older server meets

An older lender answers the relay's routes with 404, which the borrower shows
as "demi.example.com does not lend providers. Update it." An older borrower
has no Another Demi row.

## Spike

Between two development servers, the borrower relaying a lender's API-key
provider:

- **A relayed turn with tool calls** answered: the model's shell calls ran on
  the borrower's Host, two requests and one tool round per turn.
- **What changed only.** The first request of a socket crosses whole, 8.6 KB
  here. The next tool rounds of the same conversation sent 1.0 KB, 1.0 KB and
  0.97 KB where the whole request was 11.8 KB, 12.2 KB and 12.6 KB: about 8%,
  falling as the conversation grows. A conversation's first round after a
  user message sent 3.2 KB of 11.3 KB.
- **A key revoked mid-run** ended the reply where it stood within the same
  second. The first build said the provider was unreachable; the borrower now
  asks the lender why the socket closed and shows that the key is no longer
  accepted.
- **Failures cross as codes.** A lender whose own key the vendor refused gave
  the borrower "The provider on 127.0.0.1 could not answer (auth_expired)",
  with nothing of the vendor's message.

Not verified: a relayed Codex or Claude Code turn, since neither development
server had a signed-in subscription.

## Prior art

- [LiteLLM](https://github.com/BerriAI/litellm) issues virtual keys to the
  clients of its proxy and stores only their SHA-256
  (`litellm/proxy/utils.py`, `hash_token`); a key is shown once.
- [new-api](https://github.com/QuantumNous/new-api) issues tokens with an
  expiry time and records when each was last used (`model/token.go`); it
  stores the token itself.
- Both relay requests in the vendors' own protocols, the OpenAI and Anthropic
  APIs, to any client. Demi relays its own contract to another Demi instead,
  so that nothing a provider tells the agent is lost on the way.
- [Cherry Studio](https://github.com/CherryHQ/cherry-studio) and
  [CC Switch](https://github.com/farion1231/cc-switch) share a provider by a
  link holding its endpoint and key, which the other app imports. Neither
  shares a subscription.

## Alternatives

- **Copying the provider's configuration.** Works for an API key and hands
  it over for good, and cannot work for a subscription, whose sign-in the
  two copies would revoke in turn.
- **The lender speaks the OpenAI and Anthropic APIs**, and the borrower adds a
  Compatible API. Nothing new on the borrower, and other applications could
  use it, but the vendors' protocols carry none of what Claude Code resumes a
  session from, nor images, videos and audio, nor a failure's wait.
- **One socket per pair of servers**, carrying every session. Fewer
  connections, but a session's state then outlives its connection, and a
  reconnect has to restore it.

## Open questions

None.
