---
status: proposed
---

# A compatible API sends headers the user sets

## Problem

Alex points a compatible API entry at a gateway, `https://llm.example.com/v1`,
that routes each request to one of several upstream accounts. The gateway
keeps a conversation's requests on one account, so the upstream prompt cache
holds, only when each request names its conversation in an `x-session-id`
header. It also wants `User-Agent: alex-agent/1.0` to tell Demi's traffic
apart, and its Anthropic upstream turns on a beta only with
`anthropic-beta: context-1m-2025-08-07`. An entry sends no header of the
user's, so the gateway spreads one conversation over several accounts, each
request writes the whole prompt into a cold cache, and the beta never turns
on.

## Decision

A compatible API entry has Headers, the last row of its connection, under
API Key: a list of names and values that every request of the entry
carries. A vendor's entry has no Headers.

```text
API Key     ••••••••••••
Headers     Sent with every request. {conversation} is replaced     [Add Header]
            with the conversation's ID.
            ┌──────────────────┬───────────────────────────────┬───┐
            │ User-Agent       │ alex-agent/1.0                │ ⊖ │
            │ x-session-id     │ {conversation}                │ ⊖ │
            │ anthropic-beta   │ context-1m-2025-08-07         │ ⊖ │
            └──────────────────┴───────────────────────────────┴───┘
```

- Add Header adds a row with an empty name and value and puts the focus in
  its name. A row without a name is kept and shown with its placeholder, as
  an empty field of the entry is; it sends nothing until it has a name.
- Each field commits as the entry's other fields do, on Return or when it
  loses the focus. A name that is no HTTP header name, or that another row
  already has in any case, and a value with a line break, stay in the field
  with the problem under the row, until fixed or until Escape gives the
  stored text back.
- The remove button beside a row removes it at once.
- `{conversation}` in a value is replaced with the ID of the conversation
  the request serves: the same for its agent, its subagents, its side
  questions, its title and its compaction. A request that serves no
  conversation, such as Test Connection, a model list or a generated image,
  leaves out every header whose value names `{conversation}`. Any other text
  in braces is sent as written.
- The headers go after Demi's own and replace any of the same name, so
  `User-Agent` or `Authorization` the user sets is the one sent.
- Through a device (0199), the device's runner sends them as it sends the
  rest of the request.
- `demi providers show` lists them. The command line changes them as it
  changes the entry's other settings, by the entry's patch (0219).

The design to build from:

```rust
// crates/web-api-protocol/src/providers.rs
pub struct ProviderHeader {
    /// Empty while the user has not named it; such a row sends nothing.
    pub name: String,
    pub value: String,
}
// ProviderEntry (state): `headers: Vec<ProviderHeader>`, a compatible
// entry's, empty for any other.
// ProviderPatch: `headers: Option<Vec<ProviderHeader>>`, the whole new list;
// refused for a vendor's or a family's entry, as `baseUrl` is.
```

- The list is stored in the entry's sealed configuration (`ApiKeyConfig`)
  beside its endpoint, so it needs no migration: an entry without it has
  none.
- The entry's `ProviderHttp`, the one client of its requests, holds the
  list and adds it to every request it sends; a request built for a
  conversation names that conversation to the client.
- `InferenceRequest` gains `conversation: Option<String>`, which the agent
  server sets for every request of a conversation.
- An older page shows no Headers and keeps them as they are. The runner
  does not change.

## Spike

None.

## Prior art

- Codex: `model_providers.<id>.http_headers` holds fixed headers, and
  `env_http_headers` takes each value from an environment variable, sent
  only when the variable is set
  ([Configuration](https://github.com/openai/codex/blob/main/docs/config.md)).
  The leaving out of a `{conversation}` header without a conversation
  follows the latter.
- OpenCode: `provider.<id>.options.headers`, whose values take `{env:NAME}`
  and `{file:path}`; Demi's `{conversation}` takes the brace form
  ([Providers](https://opencode.ai/docs/providers/)).
- Claude Code: `ANTHROPIC_CUSTOM_HEADERS`, one `Name: Value` per line, and its
  own `X-Claude-Code-Session-Id` on every request, so a gateway groups a
  session's requests without reading their bodies
  ([LLM gateway](https://docs.anthropic.com/en/docs/claude-code/llm-gateway)).
  `{conversation}` gives a gateway that grouping under the header name it
  expects.
- Postman and Insomnia edit a request's headers as a table of names and
  values, a row added with a button.

Where Demi departs: the headers are set in Settings rather than a file, and
the only value they fill in is the conversation's ID, since the backend's
environment is not the user's.

## Alternatives

- One `{session}` per agent instead of `{conversation}`: a subagent would
  then reach the gateway as a session of its own. A gateway groups by what
  the user sees as one session, as Claude Code's header does, and the
  conversation's ID is the one the user can read in the address bar.
- A JSON object in one text field, as a configuration file holds it: no
  field of the page is edited as JSON, and a typo would lose every header.
- Headers on every entry: a vendor's API takes the headers its vendor
  documents, which Demi sends already; the user's own ones are a gateway's.

## Open questions

None.
