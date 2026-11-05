---
status: accepted
---

# An Anthropic model thinks the way Anthropic's model list says it does

## Problem

Alex adds an Anthropic API key, picks Claude Opus 4.5 at Low and sends a
message. The turn fails with HTTP 400, `adaptive thinking is not supported
on this model`, before Alex has had one answer.

Demi decides how a Messages API model takes a reasoning level from
models.dev: a model whose record lists an `effort` option gets adaptive
thinking at that effort, and any other model a token budget. models.dev
lists the same options, `effort` and `budget_tokens`, for Claude Opus 4.5 and
Claude Opus 4.6, yet only 4.6 takes adaptive thinking. Its records cannot
tell the two apart, so no rule over them gets both right.

Anthropic's own list of the key's models, `GET /v1/models`, which Demi
already reads to leave out the models a key cannot call, states it per
model:

```json
{
  "id": "claude-opus-4-5-20251101",
  "capabilities": {
    "thinking": {
      "supported": true,
      "types": {"adaptive": {"supported": false}, "enabled": {"supported": true}}
    }
  }
}
```

## Decision

**For an entry of Anthropic's own API, Anthropic's model list decides how
each model takes a reasoning level.** A model it lists as taking adaptive
thinking gets adaptive thinking at the chosen effort; one that takes only a
budget gets the effort's budget. Every other Messages API entry, another
vendor's or a compatible one, keeps deciding as today: from models.dev, or
from what its user set. A model the list states nothing about also keeps
models.dev's answer.

The answer travels with the entry's model catalog, which already holds each
model's form:

```text
 GET /v1/models ──> offered models ──> the entry's catalog ──> the provider
 (ids + forms)      (id, form)         (models.dev's facts,    (sends adaptive
                                        form from the list)     or a budget)
```

- `Provider::offered_models` answers each offered model with what the
  vendor states of its thinking, not its id alone:

  ```rust
  pub struct OfferedModel {
      pub id: String,
      /// How the model takes a reasoning level, when the vendor states it.
      pub thinking_form: Option<ThinkingForm>,
  }
  ```

  The Anthropic provider reads `capabilities.thinking.types`: adaptive
  supported is `Effort`, otherwise enabled supported is `Budget`, otherwise
  none. The OpenAI and Google providers state none.
- The catalog's refresh, which keeps only the offered models, sets each kept
  model's `thinking_form` from its offered model where that states one. An
  alias such as `claude-opus-4-5` takes the form of the dated model it names,
  as it is kept today.
- The provider's per-model facts (`ModelWires`) of a vendor entry come from
  the entry's catalog record as the cache holds it, read without starting a
  refresh, so building a provider never waits for a source. An entry with no
  record yet takes models.dev's facts, as every vendor entry does today.
- Nothing new is stored: the catalog record already keeps `thinking_form`.
  A record written before the upgrade keeps models.dev's forms until its
  next refresh, at most 15 minutes later or at Refresh in Settings ›
  Providers.

## Spike

- Whether Anthropic's list states the capability: `GET /v1/models` with a
  real key lists it for every model. Opus 4.7 and later, the Claude 5
  models and Haiku 5.5 take adaptive thinking only; Opus 4.6 and Sonnet 4.6
  take both; Opus 4.5 and Haiku 4.5 take a budget only.
- Whether a budget works where adaptive fails: Opus 4.5 refuses adaptive
  thinking with the 400 above, and answers with thinking to a 1,024-token
  budget, with or without `output_config.effort`.

## Prior art

- [Anthropic's TypeScript SDK](https://github.com/anthropics/anthropic-sdk-typescript)
  types the same `capabilities` object (`ModelCapabilities`,
  `ThinkingTypes`) for clients to read instead of naming models.

## Alternatives

- **A budget for every model models.dev lists `budget_tokens` for.** It fixes
  Opus 4.5 without new code, but Opus 4.6 and Sonnet 4.6 would lose adaptive
  thinking for the deprecated budget, and with it the thinking between tool
  calls that Demi gets without the interleaved-thinking beta header.
- **Retry with a budget after this 400.** Every first request to such a model
  would fail once, and the retry depends on the words of an error message.
- **A list of model names in Demi's code.** Each new model would need a
  release, and the list would be a second source for what Anthropic states.

## Open questions

None.
