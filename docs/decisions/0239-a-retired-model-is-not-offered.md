---
status: accepted
---

# A retired model is not offered

## Problem

Alex adds a Gemini API key. The model menu and Settings › Providers list 21
Gemini models, and several of them cannot serve a conversation:

- `gemini-2.5-pro` answers HTTP 404, "This model models/gemini-2.5-pro is no
  longer available to new users".
- `gemini-3.5-flash` answers, but its answer's `modelVersion` is
  `gemini-3.6-flash`: Google routes it to its successor, which the menu
  lists too.
- `gemini-3.1-flash-live-preview` speaks only `bidiGenerateContent`, a live
  audio socket Demi does not open.

An API-key entry's models are its vendor's list on models.dev and nothing
else. models.dev marks a model `status: "deprecated"` for only some of them
(2 of Google's 36), and Demi does not read the mark even then. The vendor's
own list is no better alone: Google's still lists `gemini-2.5-pro` and the
preview it reroutes. Alex picks a model, sends a message and reads a 404, or
gets another model than the one the menu says.

## Decision

An entry whose models come from its vendor's list on models.dev offers a
model only when every source Demi has agrees that the model serves a
conversation. Four rules take a model out, each where its fact is known:

| Rule | Source | Applies |
| ---- | ------ | ------- |
| 1. The vendor does not offer it for chat | The vendor's `GET …/models` for the key, and models.dev's `type` | When the catalog is read |
| 2. Its vendor retired it | models.dev's `status: "deprecated"`, and a past `deprecation_date` in LiteLLM's model table | When the catalog is read |
| 3. The vendor refuses it to the key | A call answered "no longer available" | After that call |
| 4. The vendor answers with another model | A call whose answer names another model | After that call |

A model taken out is not in the model menu, in Settings › Providers' list or
in `demi providers show`, as a model the vendor never listed is not. The
rules apply only to a list read from models.dev: an entry's configured
list is the user's and is never filtered, and a subscription's directory is
the vendor's own word on what the account may use.

### 1. The vendor does not offer it for chat

- A family whose API lists models for a key reads that list beside
  models.dev when the catalog refreshes, through a new `Provider` method:

  ```rust
  /// The ids of the models the key may call for a conversation, by the
  /// vendor's own list read through `http`, the entry's client; `None` for
  /// a provider whose vendor keeps no such list Demi trusts.
  fn offered_models<'a>(&'a self, http: &'a ProviderHttp)
      -> BoxFuture<'a, Option<Result<Vec<String>, String>>>;
  ```

  - Google: `GET …/v1beta/models`, every page, the models whose
    `supportedGenerationMethods` hold `generateContent`.
  - Anthropic's and OpenAI's own endpoints: `GET …/v1/models`. Another
    vendor on their APIs may list fewer models than it serves, or none.
  - Every other provider: `None`, so its list is models.dev's as today.
- A model models.dev lists is taken out when the vendor's list holds
  neither it nor a snapshot of it: its id then `-` and a date or a number,
  in Anthropic's, OpenAI's or Google's forms, such as
  `claude-opus-4-5-20251101` for `claude-opus-4-5`, as Anthropic lists only
  the dated id of some models. The vendor's list adds no model: models.dev
  stays the one source of a model's facts.
- A failed read of the vendor's list keeps the entry's last good list, as a
  failed refresh does today, so a passing outage never empties the menu.
  An entry whose key the vendor refuses has never had a good list: it lists
  no model, and its page says the vendor's list could not be read.
- A models.dev model whose `type` is set and is not `chat`, such as
  `realtime`, `embedding`, `speech`, `image` or `video`, is not a model of
  the menu. An image, video or audio model stays `demi image`'s,
  `demi video`'s or `demi audio`'s, which list their vendors' media models
  apart.

### 2. Its vendor retired it

- A models.dev model with `status: "deprecated"` is taken out, and the
  model search that adds a model to a configured list no longer finds it.
- LiteLLM's `model_prices_and_context_window.json` gives a shutdown date
  (`deprecation_date`) for far more models than models.dev marks: 951 of
  its 4561. The backend reads it in the background when it starts and once
  a day after, so a catalog's refresh never waits for it, and keeps the
  last copy after a failed read (`crates/provider-registry/src/shutdowns.rs`).
  A model whose day has come is taken out.
- A LiteLLM model is matched to a Demi vendor by its `litellm_provider`,
  through one table beside the vendor list (`google` → `gemini`,
  `anthropic` → `anthropic`, `openai` → `openai`, …), and by its id after
  the provider's prefix. A vendor with no row has no dates.
- Demi's media catalog (`catalog/media/v1/models.json`) holds what vendors
  publish of their media models, so a media model its vendor shut down
  leaves it: Veo 3.1, Veo 3.1 Fast and Veo 3.1 Lite on the Gemini API.

### 3. The vendor refuses it to the key

- An HTTP 404 whose text names the model, such as Google's "is no longer
  available", OpenAI's `model_not_found` or Anthropic's `not_found_error`
  naming the model, is a new `ErrorCode::ModelUnavailable`. A 404 that
  names no model, such as a wrong endpoint's, is not. It is not retried,
  as `PermissionDenied` is not.
- The backend records the entry, the model and the SHA-256 of the entry's
  key in the control store, and reads the entry's list again, so the pages
  hear of it. The model leaves that entry's list; the turn that met it
  fails with the vendor's words, as it does today, which a new line of
  every language names: "The provider no longer offers this model."
- A record holds only for the key it was made with: another key may be
  another account the vendor still serves the model to. A conversation that
  keeps the model meets the refusal again on each turn, which records
  nothing new and reads no list again.

```sql
CREATE TABLE retired_models (
  provider_id TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  key_digest  TEXT NOT NULL,
  model_id    TEXT NOT NULL,
  -- The model that answered instead; null for a refusal.
  answered_by TEXT,
  retired_at  INTEGER NOT NULL,
  PRIMARY KEY (provider_id, key_digest, model_id)
) STRICT;
```

### 4. The vendor answers with another model

- A call's end, which the usage ledger reads, names the model that
  answered instead of the one asked:

  ```rust
  pub struct VendorCall {
      // …
      /// The model that answered instead of the one asked, which the vendor
      /// routes the one asked to; none for an answer of the model asked, or
      /// of a snapshot or an alias's model, which is the model asked.
      pub rerouted_to: Option<String>,
  }
  ```

  - Google sets it from the stream's `modelVersion` when it is neither the
    model asked nor a snapshot of it, and the model asked is not one of
    Google's aliases, whose name ends in `-latest`
    ([model version patterns](https://ai.google.dev/gemini-api/docs/models#model-versions)).
  - Every other provider leaves it empty until a vendor is seen rerouting:
    OpenAI and Anthropic answer a dated snapshot of an alias, such as
    `gpt-5` with `gpt-5-2025-08-07`, which is not another model.
- The call's turn goes on, since the answer is good; the backend records
  the model in `retired_models` with the model that answered, and the entry
  no longer offers it.
- A conversation keeps the model it chose, as it keeps one that leaves the
  catalog today, and its calls go on reaching the successor; picking a
  model in the menu offers the successor.

### What an older page, runner or data meets

The page and the runner read the catalog the backend builds and see fewer
models; nothing they send changes. The migration adds `retired_models`
empty.

## Spike

Each rule's facts were read with one Gemini API key, by the free models
list and `countTokens`, and by seven `generateContent` calls of at most one
output token:

| Model | In Google's list | `generateContent` |
| ----- | ---------------- | ----------------- |
| `gemini-2.0-flash` | no | 404, no longer available |
| `gemini-2.5-pro`, `gemini-2.5-flash`, `gemini-2.5-flash-lite` | yes | 404, no longer available to new users |
| `gemini-3.1-flash-lite-preview` | yes | answered by `gemini-3.1-flash-lite` |
| `gemini-3.5-flash` | yes | answered by `gemini-3.6-flash` |
| `gemini-3.7-flash` | yes | answered by `gemini-3.8-flash` |
| `gemini-3.1-pro-preview`, `gemini-3-flash-preview` | yes | answered by itself |
| `gemini-3.1-flash-live-preview` | yes, `bidiGenerateContent` only | not called |
| `veo-3.1-generate-preview` | no | not called (shut down) |

- `countTokens` succeeds for models that are shut down, so it proves
  nothing, and a model's own record (`GET …/models/<id>`) does not tell a
  rerouted model from a live one: only an answer does.
- LiteLLM dates `gemini-3.1-flash-lite-preview` (2026-05-25),
  `gemini-2.5-computer-use-preview-10-2025` (2026-07-28) and
  `veo-3.1-generate-preview` (2026-10-22); models.dev marks the first and
  neither of the others.

## Prior art

- [opencode](https://github.com/anomalyco/opencode) drops a models.dev model
  whose `status` is `deprecated` and a model whose `type` is not `chat`
  from the models it offers (`packages/core/src/plugin/models-dev.ts`).
- [LiteLLM](https://github.com/BerriAI/litellm) keeps each model's
  `deprecation_date` in its model table, which its proxy reports.
- Clients that read a vendor's `GET /models`, such as LobeChat and Cherry
  Studio, offer what the vendor lists. Demi departs by keeping models.dev's
  facts and using the vendor's list only to take models out.

None of them learns from a call that a model is refused or rerouted; Demi
does because the vendor's own list keeps such models.

## Alternatives

- **The vendor's list alone.** Google's keeps the models it refuses and
  reroutes, and it holds no window, price or thinking facts.
- **Demi's own list of retired text models**, kept in the repository like
  the media catalog. It could take a rerouted model out before anyone calls
  it, but it is a second owner of facts models.dev and LiteLLM keep, and it
  is stale whenever no one updates it.
- **A probe call per model on each refresh.** It finds every rerouted
  model at once, but it spends the user's money on every key, every day.

## Open questions

- A rerouted model nobody has called yet stays in the menu until its first
  call; Demi's own list is the only way to take it out sooner.
