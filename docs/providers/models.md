# Models

A conversation infers with one model of one provider entry. This document
defines where Demi learns which models an entry offers and what each model can
do, how the backend and the web app keep that information, and which facts of
the selected model travel with every inference request. Provider entries,
accounts and endpoints are defined in [Providers](providers.md); the Claude
Code CLI is defined in [Claude Code](claude-code.md).

```text
web app   one catalog for the signed-in account, asked for again after a minute
   |  GET /api/models             GET /api/models?refresh=true
   v
backend   an entry's configured list, read from the entry; for any other entry,
          the catalog cache: one record per entry, fresh for 15 minutes,
          kept in memory and in the control store, one refresh at a time
   |  a missing or expired record, or a forced refresh: one read of the source
   v
source    the entry's vendor in models.dev | the provider's own directory
```

## Catalog sources

An entry's **catalog** is the list of models the entry offers, with what each
model can do. For example, a user has three provider entries:

- *Work OpenAI*, an API-key entry of the `openai` family with a configured list
  of two models. Its catalog is exactly those two models, read from the entry's
  configuration without a network request.
- *DeepSeek*, an API-key entry the user added from the vendor list. Its
  catalog is DeepSeek's model list in the models.dev document.
- *Codex*, a subscription entry. Its catalog is the list Codex's service
  returns for the entry's active account.

An entry's catalog comes from one source, chosen in this order:

1. **The entry's configured model list**, when it has one. Only API-key entries
   have one. It is user configuration: no fetched catalog changes it and a
   refresh never clears it; removing the list returns the entry to the next
   source ([Web API](../product/web-api.md#model-configuration-and-provider-inspection)).
   A configured model states its facts directly. Its Fast tier, when it names
   one, is its only service tier, and it is taken to call tools.
2. **Its vendor's model list in models.dev**, when the entry was added from the
   vendor list. [Providers](providers.md#vendors-from-modelsdev) defines which
   vendors the list offers.
3. **The provider's own directory** ([Directories](#directories)).

A source that is unavailable is not replaced by the next one. A vendor entry
whose vendor models.dev does not list has an empty catalog, not its family's
built-in list.

Each model in a catalog states the following facts. A capability the source
does not state is null, which means unknown.

| Field | Meaning |
|---|---|
| `id`, `displayName`, `description` | The model as its source names it |
| `contextWindow`, `outputLimit` | Token limits; a null `outputLimit` means no model-specific limit ([Output limit](#output-limit)) |
| `supportsTools` | Whether the model can call tools |
| `supportsAttachments`, `supportsVideo`, `acceptedExtensions` | Which files the model reads natively ([Accepted attachment types](#accepted-attachment-types)) |
| `supportsReasoning`, `supportedThinkingEfforts`, `canDisableThinking` | Which thinking settings the model offers ([Thinking and service tiers](#thinking-and-service-tiers)) |
| `serviceTiers`, `defaultServiceTierId` | The service tiers the model offers; a tier marked `fast` is Fast |
| `cost` | Prices, as models.dev reports them |
| `sourceFetchedAt`, `stale` | When the source was fetched, and whether this is a copy kept after a failed refresh |

### The models.dev document

[models.dev](https://models.dev) publishes one JSON document that describes
many vendors and their models. The backend keeps one copy of it, with the
document's `ETag` and `Last-Modified` values, because several readers use the
same document: the catalogs of vendor entries, the Claude Code catalog, and the
list of vendors the providers page offers.

- A catalog read asks models.dev again, with a conditional request when a copy
  exists. A 304 answer confirms the copy and keeps its fetch time, so
  `sourceFetchedAt` says when the content was last downloaded, not when it was
  last confirmed.
- The vendor list reuses a copy downloaded or confirmed less than a day ago.
- Concurrent readers share one request.
- A failed request, or a document that cannot be read, returns the last copy
  marked stale, with the warning "Using stale models.dev catalog: …". Without a
  copy, the read fails.

A models.dev model becomes a catalog model as follows: `limit.context` and
`limit.output` are its token limits, `attachment` is attachment support,
`reasoning` is thinking support, the values of its `effort` reasoning option
are its thinking efforts, `tool_call` is tool support, and `cost` is its
prices. A limit that is not a positive whole number, such as an output limit
of 0, states no limit and reads as unknown.

### Directories

A provider keeps no catalog between reads: each read of its directory goes to
the source. The backend's [catalog cache](#catalog-cache) is the only place an
entry's catalog is kept.

| Family | Directory |
|---|---|
| `anthropic`, `openai`, `google` | A list built into the provider for its family. Reading it makes no request. |
| `codex` | The account's model list on the ChatGPT backend ([The Codex catalog](#the-codex-catalog)). |
| `grok-build` | `GET /v1/models` on the Grok Build service, with the account's session. The service answers a `data` envelope or a bare list. A model is named by its `id`, else its `model`; one that names neither is skipped. Its efforts are its `reasoning_efforts`, in the service's order. Every model takes tools and images; its context window is unknown unless the service states one. A catalog that cannot be read is a failed refresh: the service is unreachable or refuses, the account has no session, the payload cannot be read, or it lists no model. |
| `claude-code` | The `anthropic` vendor of the models.dev document: the models whose id starts with `claude-` and whose version is 4.6 or later. Opus models come first, then Sonnet, then Haiku, then others, newest version first within each family. An id whose version cannot be read is skipped with a warning. Thinking cannot be turned off, because the CLI's `--effort` option only levels it. |

### The Codex catalog

The Codex provider reads the account's model list from the ChatGPT backend's
`/codex/models` endpoint, with the account's authorization and a
`client_version` query parameter. The service decides which models it lists by
that version, so the provider sends the version of the Codex CLI release it was
last verified against, and that value is updated whenever the provider is
checked against a newer CLI. Reading the catalog never runs the CLI or an
inference.

- The response is validated. Models are sorted by ascending `priority`, and
  only entries with `visibility: list` are kept, as in the CLI's own model
  picker; hidden internal models are never shown.
- Model ids and reasoning levels come from the response, not from a list kept
  in Demi.
- Every `supported_reasoning_levels` entry and `default_service_tier` are
  kept. The web app's model selector offers the full effort list, and a
  conversation that names no effort takes the first, not Codex's
  `default_reasoning_level`
  ([A conversation's model settings](#a-conversations-model-settings)). An explicit effort, including `max` or
  `ultra` when advertised, reaches Codex's `reasoning.effort` unchanged.
- There is no Off option: omitting reasoning gets Codex's default instead of
  turning reasoning off. An advertised `none` effort stays a selectable level.
- The model's `service_tiers` are its tiers, and `priority` is Fast.
- A model whose `input_modalities` include `image` accepts attachments.
- An HTTP 401 answer makes the provider refresh the token it was refused with
  ([Token refresh](providers.md#token-refresh)) and read again, once.

Upstream references:
[models request](https://github.com/openai/codex/blob/3dc1e2a58406dc69db5812539adfee7d89fa9ef7/codex-rs/codex-api/src/endpoint/models.rs),
[model metadata and presets](https://github.com/openai/codex/blob/3dc1e2a58406dc69db5812539adfee7d89fa9ef7/codex-rs/protocol/src/openai_models.rs),
and [picker visibility](https://github.com/openai/codex/blob/3dc1e2a58406dc69db5812539adfee7d89fa9ef7/codex-rs/app-server/src/models.rs).

## Catalog cache

For example, the backend restarts at 09:55 with a stored Codex record that was
checked at 09:50. At 10:00 a user opens the model picker: the backend loads the
stored record, finds it 10 minutes old, and returns it without asking Codex. At
10:06 the record is 16 minutes old: the backend returns it at once, marked
stale, and starts one refresh. A second page that asks at the same
moment gets the same answer, and no second refresh starts. If Codex answers,
the new list replaces the record in memory and in storage. If Codex does not
answer within 10 seconds, the list from 09:50 stays, the failure appears as a
warning, and the backend waits one minute before it tries again by itself.

### What the backend keeps

Each provider entry has at most one catalog record: the validated catalog and
the time of its last successful check. The backend holds the record in memory
and stores it in the control store's `model_catalogs` table
([Storage](../backend/storage.md#control-records)). A record holds model
metadata only: never credentials, authentication state, quota, or whether a
conversation can use the model now. Different users and different entries
never share a record.

A record is keyed by a digest of the entry's configuration and its active
account. A record stored under another key is not used, so a changed entry
starts from an empty cache. A configured model list is not cached: it is read
from the entry every time and never waits for a fetch.

### Freshness and refresh

A record is fresh for 15 minutes, in memory and in storage alike. A restart
loads stored records before it makes any network request.

| Record when a read arrives | What the read returns | Refresh |
|---|---|---|
| Fresh | The record, at once | None |
| Expired | The record at once, marked stale | One refresh starts, or the read joins the running one |
| None | What the refresh returns, after waiting for it | One refresh starts, or the read joins the running one |
| Any, with `GET /api/models?refresh=true` | What the refresh returns, after waiting for it | One refresh starts even when the record is fresh or a retry is being held off, or the read joins the running one |

- At most one request per entry is in flight, however many readers wait.
- Every refresh has a limit of 10 seconds.
- A successful refresh replaces the record in memory and in storage.
- A failed refresh keeps the last good catalog and its time, adds the failure
  to the entry's warnings, and holds off automatic refreshes for one minute.
  Without a record, the entry's catalog is empty, with the failure as its
  warning. A refresh never clears a usable catalog before a replacement
  arrives.
- A source answer that is marked stale, such as the models.dev copy returned
  after a failed request, counts as a failed refresh. A provider therefore has
  no use for a catalog cache of its own: the backend would treat its cached
  answer as a failure, and one cache per catalog is enough.
- A refresh belongs to the cache, not to the reader that started it: a reader
  that stops waiting does not cancel it.
- The record read from storage and the catalog a source returns are both
  validated. A stored record that fails validation is reported as the entry's
  failure, never repaired or silently dropped; a source answer that fails
  validation is a failed refresh.

### Invalidation and shutdown

Editing an entry's configuration, changing its active account, or deleting the
entry invalidates its record: the backend drops the record from memory,
deletes the stored row, and cancels a refresh in flight. A cancelled or
invalidated refresh cannot write a record back. At shutdown the backend cancels
running refreshes and waits for them before it closes storage.

### What the web app receives

`GET /api/models` returns every entry the user infers with, each with its
catalog, `sourceFetchedAt`, `stale` and `warnings`, and with the provider's
authentication and runtime state, which the backend reads when it answers and
never stores. One entry's failure leaves the other entries and every configured
list intact. A configured or built-in list, and the empty catalog of an entry
that no refresh has filled, carry the Unix epoch as their `sourceFetchedAt`,
because they were never fetched. The backend builds each model's selection
([Request parameters](#request-parameters)) and sends it with the model, so
the web app never converts a catalog model itself.
[Web API](../product/web-api.md#model-configuration-and-provider-inspection)
defines the route.

### In the web app

- The web app keeps one catalog for the signed-in account. New and
  existing conversations use it; model metadata is never copied into a
  per-conversation cache.
- It asks for the catalog again after one minute at the earliest, so
  navigation does not request it repeatedly; nothing asks for it on a timer.
  Concurrent readers share one request, and a failed load waits one minute
  before it is tried again by itself.
- A change to any entry's configuration or active account loads the catalog
  again. An explicit refresh asks with `refresh=true`.
- Signing out clears the catalog, its timer and its pending request.
- The catalog does not depend on a conversation. To show whether a model is
  available, the web app combines each entry's health, and whether it needs a
  process on the user's Cloud, with the selected conversation's target;
  sending a message checks the real target.
- Opening a conversation does not wait for the catalog. The transcript shows
  while the composer loads its model choices, and a catalog failure never turns
  a readable transcript into a loading or failed conversation. The product and
  the gallery use the shared session and composer loading states.

## Request parameters

For example, a user picks GPT-5.5 on an `openai` entry, with the effort `high`
and Fast. The conversation's model selection records the model with its
catalog facts (a context window of 272,000 tokens, no output limit, the efforts
`low` to `xhigh`, and the tier `priority` marked Fast), the thinking setting
(effort `high`), and the tier `priority`. Every request the conversation makes
carries the model id, the output limit, the thinking setting and the tier.

A selection is built from a catalog model by one conversion, which the backend
applies before it sends the catalog to the web app and when a conversation
switches to the model. The conversation keeps its
selection: a changed catalog does not change a running conversation, which
keeps its model's facts until the user selects a model again. The one exception
is an entry with a configured model list. The backend applies the configured
facts again at every inference boundary, so an edit of a model's limits reaches
the next request of a running conversation, while the conversation's thinking
and tier choices stay as the user made them. A model that the configured list
does not name fails the request. The choice a new conversation starts with is
a [user preference](../product/web-api.md#user-preferences).

### A conversation's model settings

A conversation's model settings are one value: the model selection its record
holds, with the provider entry, the model and its facts, the thinking setting
and the service tier. Every tab and every device shows that value, a change
made anywhere reaches all of them, and the conversation's next provider request
uses it
([Sidebar mutations and read state](../product/web-api.md#sidebar-mutations-and-read-state)).
The page shows and changes the value as its settings, the parts a user
chooses:

| Part | Values |
|---|---|
| Model | A provider entry and one model of its catalog |
| Thinking effort | An effort the model lists, or `disabled`, thinking off, when the model can turn thinking off. A model that lists no efforts has no effort to choose, and its value is null |
| Service tier | A tier the model lists, or null for the vendor's default |

The backend turns an effort into the thinking setting `effort` with the
model's default summary, and `disabled` into thinking off. The effort is
always one the user can read in the model menu, which offers Off when the
model can turn thinking off and the efforts the model lists, and nothing else:
no choice leaves thinking to the vendor, whose effect the user could not know.
When a change names no effort, the backend chooses the first effort the model
lists, never thinking off: for a new conversation, a switch to a model that
does not list the conversation's effort, and any other change that names
none. The page reads the same first effort from the catalog, so the effort the
page shows is the one the request sends, and every request of a model that
lists efforts carries a thinking setting. For example, a switch to a model
that lists `low` to `max` records `low`, whether or not it can turn thinking
off. A model that lists no efforts has no reasoning choice, and its requests
carry no thinking setting.

A change names the parts it changes, and the value keeps the others. For
example, one tab turns Fast on while another raises the effort: the value ends
with both. The backend checks each part against the entry's catalog when it
applies the change and refuses a model the catalog does not list, or an effort
or tier the model does not offer, so the value holds only what its model
offered when it was chosen.

A switch to another model takes the effort and the tier the switch names; for
a part it does not name, the new model's first effort and the vendor's default
tier. The model menu names the conversation's effort when the new model offers
it, thinking off included, else the new model's first effort, and Fast when
the new model has a Fast tier of its own. So a switch from a model at `high`
with Fast to one that lists `high` and has a Fast tier keeps both, and a switch
to a model with neither takes the new model's first effort and the vendor's
default tier.

### Output limit

`outputLimit` is a positive whole number of tokens, or null when no
model-specific limit is known. The agent reads it for every request, including
continuations after the user switches models within one provider: a provider
runtime never keeps the first model's limit.

A request may also cap its own output. A title request asks for at most 1,024
tokens ([Conversation titles](../product/product.md#conversation-titles)); a
turn sets no cap. The field the vendor receives, in the table below, carries
the lower of the model's limit and the cap, and the cap alone when the model
has no limit. The cap belongs to the request, not to the model: a configured
model list replaces the model's limit at every inference boundary
([Request parameters](#request-parameters)), and the cap stays. For example,
with a configured model whose limit is 8,000 tokens, a turn sends 8,000 and a
title request sends 1,024.

| Family | Request field | Without a limit or a cap |
|---|---|---|
| `anthropic` | `max_tokens` | 32,000 |
| `google` | `generationConfig.maxOutputTokens` | 32,000 |
| `openai`, Responses | `max_output_tokens` | The field is omitted |
| `openai`, Chat Completions | `max_completion_tokens` | The field is omitted |
| `codex`, `claude-code`, `grok-build` | Not sent | The vendor's service or the CLI decides |

A configured model list refuses an output limit that is not a positive whole
number or that exceeds the model's context window. A model selection the
web app sends is refused by the same whole-number rule.

### Thinking and service tiers

Thinking and the service tier travel with each request. Each provider maps the
thinking setting onto its vendor's option, an effort level or a token budget.
For example, the `anthropic` family maps them onto the Messages API as follows:

| Thinking setting | Request |
|---|---|
| An effort, or adaptive thinking at an effort | `thinking: { type: "adaptive", display: "summarized" }` and `output_config: { effort }`; `display` is `omitted` when the setting turns summaries off. For a vendor other than Anthropic, the effort's budget instead, as a token budget below |
| A token budget | `thinking: { type: "enabled", budget_tokens }`, the budget kept between 1,024 and `max_tokens` minus 1,024, and 1,024 when `max_tokens` is 2,048 or less |
| Off, or none | No `thinking` field |

The efforts are the vendor's words, so a Claude model's `low` to `max` reach
Anthropic's API unchanged. The newest Claude models refuse a token budget,
which is why an effort never becomes one there. Adaptive thinking and
`output_config` are Anthropic's own, so an entry added from another models.dev
vendor on `@ai-sdk/anthropic` sends an effort as its budget, the ladder
Gemini uses below; the vendor policy says which form a vendor takes
([Vendors from models.dev](providers.md#vendors-from-modelsdev)). An entry that
names no vendor is Anthropic's.

The OpenAI-shaped formats level thinking by effort only:

| Thinking setting | Responses (`openai`, `codex`) | Chat Completions (`openai`, `grok-build`) |
|---|---|---|
| An effort | `reasoning: { effort, summary }`, the summary asked for, else `auto`; a summary turned off leaves `summary` out, and Codex receives `auto` | `reasoning_effort` |
| The effort `none` | `reasoning: { effort: "none" }` | `reasoning_effort: "none"` |
| Adaptive thinking at an effort | `reasoning: { effort, summary: "auto" }` | `reasoning_effort` |
| A token budget, off, or none | No `reasoning` field | No `reasoning_effort` field |

Gemini levels thinking by a token budget, `generationConfig.thinkingConfig`,
and asks for thought summaries unless thinking is off, because the model
thinks, and bills for it, either way:

| Thinking setting | `thinkingConfig` |
|---|---|
| An effort, or adaptive thinking at an effort | `includeThoughts: true` and the effort's budget: `low` 4,096, `medium` 16,384, `high` 32,768, `xhigh` 65,536, `max` 98,304 tokens, and `medium`'s for any other effort |
| A token budget | `includeThoughts: true` and the budget |
| Off | `includeThoughts: false`, `thinkingBudget: 0` |
| None | `includeThoughts: true` |

A model's catalog says what the product can offer: the model's effort levels,
and whether thinking can be turned off. Codex and Claude Code
models cannot turn thinking off ([The Codex catalog](#the-codex-catalog),
[Directories](#directories)).

Fast is a service tier: the tier the model's catalog marks `fast`, such as
Codex's `priority` or the Fast tier a configured model names. The tier travels
as `serviceTierId`; there is no separate Fast flag. The product offers only the
tiers the model's catalog lists, because a request field that can carry a tier
does not mean every vendor supports tiers.

A changed thinking setting makes the vendor process a conversation's history
again, which the user's choice allows
([The rule](providers.md#the-rule)). No vendor's cache documentation counts
the output limit or the service tier as part of the cached prefix, so a change
of either keeps the cache.

### Context limit

The window Demi uses for a model is the model's context window, unless the
user limited it. For example, Claude Opus on a user's Claude Code entry has a
window of 1,000,000 tokens. The user sets its Context to 300K in the model
menu of one conversation. From then on every conversation of that user that
infers with Claude Opus on that entry compacts at 240,000 tokens instead of
800,000, and its usage indicator counts against 300K. A conversation that
switches to GPT-5.5 uses GPT-5.5's own setting, and another user of the same
shared instance still has 1M.

The limits a model offers depend on its window:

| Model's context window | The menu's Context row offers |
|---|---|
| At most 500,000 tokens, or unknown | No row: the model's window is used |
| Over 500,000 and under 1,000,000 | The full window, 300K and 200K |
| 1,000,000 or more | The full window, 500K, 300K and 200K |

- The full window is the default, and it is the absence of a limit: a model
  the user never limited has no stored limit, and choosing the full window
  removes the stored one. The menu names the full window by its size, such
  as "1M", never "Default".
- A limit belongs to the user and to one model of one provider entry, named
  by the entry's id and the model's id. It is not part of a conversation's
  model settings: it applies to every session of the user that infers with
  that model, subagents included, and a subagent profile that names the model
  follows it. The Context row shows only its options, the full window and
  the limits, and no text about this scope.
- A stored limit applies only while the model's window offers it. When the
  catalog later reports a window that does not offer it, such as 400,000
  tokens, the model's full window is used and the menu shows it selected.
- The limit replaces the model's window wherever Demi uses the window for
  itself: every compaction threshold computed from the window
  ([When compaction runs](../agent/compaction.md#when-compaction-runs)), and
  the usage indicator's percentage and total. It changes nothing a vendor
  receives or enforces: the [request limits](#request-limits) still follow
  the model's own window, and so does the check that a reported usage can
  belong to one request ([Context estimate](../agent/compaction.md#context-estimate)).
- A session reads the user's limit each time it checks the token threshold:
  before a turn, after each response and before a model switch. A change
  reaches running conversations from their next check, and starts no
  compaction by itself.

The user's limits are a [user preference](../product/web-api.md#user-preferences),
which every page of the user receives. The Models & providers settings do not
show them: the model menu is the one place to see and change a model's
limit.

### Request limits

A vendor refuses a request that is too large, however few tokens it holds:
for example, 60 screenshots of 400 KB reach the Anthropic API's 32 MB before
they fill a large context window. Each runtime states its vendor's limits for
a model, and compaction keeps every request under them
([When compaction runs](../agent/compaction.md#when-compaction-runs)):

| Family | Body, as sent | Images per request |
|---|---|---|
| `anthropic`, `claude-code` | 32 MB | 100 for a model whose own context window is at most 200,000 tokens, 600 otherwise |
| `openai`, `codex` | 512 MB | 1,500 |
| `google` | 20 MB, since media travel inline | 3,600 |
| `grok-build` | Not documented | Not documented |

A megabyte here is 1,000,000 bytes. Claude Code's CLI sends its history to the
Anthropic API, so its limits are that API's. A compatible vendor of a family
may set lower limits, and a limit that is not documented is none: a request
refused as too large still leads to compaction
([Retries](../agent/failures-and-recovery.md#retries)). Every image has
already been fitted to what each vendor accepts of one image
([Images in the transcript](../agent/runtime.md#images-in-the-transcript)),
and a medium whose base64 takes more than half of the body limit is sent as
text ([Replay](../agent/runtime.md#replay)).

### Accepted attachment types

A model reads some files natively, as images, video or documents; every other
file reaches it by path. The file types a model can read natively are a closed
set: `png`, `jpg`, `jpeg`, `gif`, `webp`, `pdf`, and the video types `mp4`,
`mov`, `webm` and `m4v`. The set is part of the file-type table, which the
backend and the web app share through the generated contract
([Contracts](../architecture/contracts.md#logic-the-web-app-and-backend-share)).

A model's `acceptedExtensions` has three distinct meanings:

| Value | Meaning |
|---|---|
| `[]` | The model is known to read no file natively. |
| `["png", "pdf"]` | The model reads the listed types. Extensions omit the dot. |
| `null` | Which types the model reads is unknown. |

The conversion to a selection derives the list from the catalog's flags:
attachment support adds `png`, `jpg`, `jpeg`, `gif`, `webp` and `pdf`, and
video support adds the four video types. Only a model known to read video gets
the video types: unknown video support adds none. When attachment support is
unknown and the model is not known to read video, the list is null. A model
whose catalog states its exact types, such as a model of a configured list,
uses them instead, including an empty list or null.

Asking whether a model accepts a type answers yes, no or unknown; `jpg` and
`jpeg` are the same format. Where Demi decides whether to give a file to the
model natively, only a yes counts, so a file of unknown support is not sent
natively. Unknown and unsupported stay distinct, so a surface can say which one
applies when it explains why a file was not sent. For example, a shell command
whose output is a PNG image returns the image to the model only when the model
accepts `png` ([Tools](../agent/runtime.md#tools)). Native media beside a
message's attachment follows [Attachments](../product/product.md#attachments).
After a model switch, the history can hold media the new model does not
accept; each request replays those as text
([Replay](../agent/runtime.md#replay)).
