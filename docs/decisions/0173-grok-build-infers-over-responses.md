---
status: accepted
---

# Grok Build infers over Responses

## Problem

Alex runs a conversation on a Grok Build subscription. Demi sends each request
to the Grok CLI's chat proxy as Chat Completions, and three things are lost on
the way:

- **Search.** xAI's web search and X search run only as tools of the Responses
  API. Alex asks what developers say on X about a new release, and the model
  can only call `demi web search`, which cannot see X.
- **Reasoning.** Chat Completions streams the model's reasoning as text and
  takes none of it back, so every request starts the model's thinking afresh.
  The proxy's catalog names Responses as every model's backend
  (`api_backend: "responses"`), which is what the Grok CLI itself speaks.
- **Images in tool results.** The proxy takes no media in a Chat Completions
  tool message, so Grok never sees a screenshot a tool took
  (`ToolResultKinds::NONE`).

## Decision

A Grok Build request goes to the proxy's `/v1/responses` instead of
`/v1/chat/completions`. Nothing on screen changes but what the model can now do:

- A tool's images reach the model inside the tool's output.
- The model's reasoning summary streams as its thinking, and the reasoning
  items come back in later requests, ciphertext included, so the model keeps
  its line of thought across a turn's requests.

xAI's web and X search, the purpose of the move, are a decision of their own:
they add what the transcript keeps and what the conversation shows. This
record only gives Grok Build the protocol they run on.

### Design

- `provider-grok-build`:
  - `request.rs` builds a Responses body with the shared
    `openai_request::responses_input`, as the Codex and OpenAI API providers
    do:
    - `model`, `instructions` (the system prompt), `input`, `tools` as
      `function` tools, `tool_choice: "auto"` when there are tools;
    - `reasoning` from the request's thinking by the shared
      `responses_reasoning`: its effort, and `summary: "auto"` unless the
      user turned summaries off;
    - `stream: true`, `store: false`;
    - no output limit or service tier, which the proxy takes neither of.

    The dialect: `signature_tag: "grok-build:"`,
    `AssistantReplay::Minimal`, `ReasoningReplay::Replayable`,
    `ToolMedia::Inline`.
  - `run.rs` posts to `{proxy}/responses` and maps the stream with
    `wire::responses::map_events`; the headers, the refresh after HTTP 401
    and the quota read from `x-ratelimit-*` stay as they are.
  - `request_limits` allows images in tool results.
- The reasoning tag is Grok Build's own and not the `xai:` of an xAI API key
  entry: the two endpoints are separate products, and neither is known to
  accept the other's ciphertext. A thinking block another vendor signed is
  left out of a request, as for every Responses provider.
- Stored data: nothing changes. A Grok Build transcript written over Chat
  Completions has thinking without a signature, which a Responses request
  leaves out, as it does another vendor's.
- `chat_completions` stays for the compatible APIs that use it; Grok Build no
  longer does, and nothing keeps its old path.

## Spike

Every point ran against the proxy with a subscription account:

| Question | What ran | What it showed |
| --- | --- | --- |
| Does the proxy take Responses? | `GET /v1/models`, then `POST /v1/responses` | Every model lists `api_backend: "responses"`; the request returns 200 |
| Function tools and streaming | A function tool and a request that needs it | `response.output_item.done` carries a `function_call`, like OpenAI's |
| A tool result's images | A user image and a `function_call_output` holding an `input_image` | The model names both images' colors |
| Reasoning replay | A turn's output items sent back as input | 200; the model answers from them |
| Reasoning summary | `reasoning: { effort: "low", summary: "auto" }` | 200, with the summary streamed |
| Quota | The response's headers | `x-ratelimit-limit-tokens` and the other headers the quota reads |

## Prior art

- The Grok CLI 1.0.62 reads the catalog's `api_backend` and speaks Responses
  to the proxy.
- CLIProxyAPI sends a Grok account's requests to the proxy's `/v1/responses`
  ([`xai_executor_request.go`](https://github.com/router-for-me/CLIProxyAPI/blob/90c447d3a8727d115aa7552ec93ce023736ed492/internal/runtime/executor/xai_executor_request.go#L232-L391)).
  It also rewrites tools, caps them at 200 and folds namespaces, for clients
  it does not control; Demi sends its own tools and needs none of that.

## Alternatives

- **Keep Chat Completions and run search as a second request**, a
  `demi x search` command that asks the proxy once with only `x_search`. It
  works for any model, but it costs a second model call per search, keeps the
  other two losses, and a model other than Grok can reach X search through a
  Grok subagent.
- **Chat Completions with `search_parameters`** (Live Search). xAI's public
  API answers it with HTTP 410, and the proxy's catalog does not name it.

## Open questions

None.
