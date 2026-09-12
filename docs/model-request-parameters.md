# Model parameters used by Demi

`core.Model` describes the selected model. `modelSelectionFromCatalog` in the
provider package copies catalog data into that selection. `AgentSession` uses the
selection to run a conversation; each new `InferenceRequest` carries the current
model ID, output limit, thinking configuration, and service tier.

## Output limit

`Model.outputLimit` and `InferenceRequest.outputLimit` are a positive token count
or `null`. Null means no model-specific limit was supplied. The agent reads the
value for every request, including continuations after a same-provider model
switch. A provider runtime does not keep the first model's output limit.

The adapters that can set an output limit expose `Provider.supportsOutputLimit`
as true. An absent flag means that the adapter cannot set this parameter.

| Adapter | Request field | Explicit override | Default when no limit is supplied |
| --- | --- | --- | --- |
| Anthropic API | `max_tokens` | `request.maxTokens` | 32,000 |
| Google API | `generationConfig.maxOutputTokens` | `request.maxOutputTokens` | 32,000 |
| OpenAI Responses | `max_output_tokens` | `request.maxOutputTokens` | Omit the field |
| OpenAI Chat Completions | `max_completion_tokens` | `request.maxOutputTokens` | Omit the field |
| Codex subscription, Claude Code, Grok Build | Not set by these adapters | Not provided | The upstream service or process controls it |

An explicit adapter request option takes precedence over the current model's
limit. Existing `extraBody` overrides are applied last. For OpenAI-compatible
Chat Completions endpoints, an explicit `extraBody.max_tokens` is also respected
without sending a second, competing `max_completion_tokens` field.

Custom API model catalogs reject non-positive or non-integer output limits.
Agent protocol input validates the model's limit with the same requirement.
Changing a catalog does not silently change the model selection of an already
running session; callers select the updated model through the existing model
switch operation.

## Thinking, Fast, and context length

Thinking and service-tier values already travel with each inference request.
Adapters map thinking to the upstream effort or token-budget option they support.
Fast uses `serviceTierId`, not a separate runtime flag. Callers should offer only
the tiers that the provider's model catalog advertises; the presence of the request
field does not imply every upstream API supports tiers.

The selected model's context window already determines the agent's automatic
compaction threshold. This change does not alter the compaction algorithm or
introduce a new input/output budget policy.

## Attachment types

`Model.acceptedExtensions` has three distinct meanings:

| Value | Meaning |
| --- | --- |
| `[]` | The model is known to accept no direct attachments. |
| `['png', 'pdf']` | The model accepts the listed file types. Extensions omit the dot. |
| `null` | The accepted types are unknown. |

The provider catalog's attachment and video flags are converted into a type list
in one place, `modelSelectionFromCatalog`. If no attachment support is known,
the result stays null. A caller with exact type information can supply
`options.acceptedExtensions`; this replaces the derived list, including when the
override is empty or null.

`core.fileExtensionSupport` returns true, false, or null using these definitions.
It treats jpg and jpeg as the same format. `modelAcceptsMediaType` and
`modelAcceptsVideo` answer whether support is known, so they return false for
unknown types.

The shared Web UI attachment checker uses this function and accepts only a true
result. It no longer treats an empty list as allowing every file. This check is
for attachments sent directly to the model; uploading a file to a working
directory is a separate operation. Product callers can distinguish unknown from
unsupported when explaining why a direct attachment is unavailable.


## Refreshing a model catalog

`Provider.listModels({ refresh: true })` asks the adapter to check its remote
catalog even when a cached result is still within its freshness period. Calling
`listModels()` without that option retains normal caching.

- Codex bypasses its local catalog freshness check and fetches the authenticated
  catalog again.
- Claude Code passes the option to the shared models.dev client. The client
  sends a conditional request when it has an ETag or Last-Modified value; a
  304 response confirms the content is unchanged.
- Grok Build already fetches its catalog on every call, so explicit refresh
  has the same network behavior as a normal read.
- OpenAI API, Anthropic API, and Google API model lists come from configured
  data. Refresh returns the current configured list without making an inference
  or inventing a remote fetch.

Codex and models.dev preserve their existing failure behavior: when a remote
check fails and a usable cached list exists, the returned list is marked stale
and carries a warning. Without a usable cached list they report failure. Grok
Build returns its explicitly stale fallback list only when credentials are missing,
the catalog request fails, or a non-auth HTTP failure prevents discovery. Its
authentication errors and malformed successful responses are reported directly.
The content's sourceFetchedAt value is preserved when revalidation confirms no
change or when a cached list is returned after failure.

Refresh preserves the provider's configured model filter and default selection
policy. It does not edit caller-owned custom model configuration or switch the
model of an active session. Product backends can forward a refresh request to
this interface; adding that product endpoint is separate from this framework API.

## Codex model catalog

`provider-codex/src/models.ts` requests the authenticated Codex `/models` endpoint.
Its default `client_version` matches verified CLI release `0.153.4`; the upstream
service uses this parameter to gate model availability. Keep it current when
updating the adapter against Codex CLI. Embedders can override `clientVersion` on
`createCodexProvider`; model discovery never executes the CLI or runs inference.

The adapter validates the response schema, sorts models by upstream `priority`,
and exposes entries with `visibility: list`, matching the CLI's normal model
picker. It does not expose hidden internal models. The first visible entry is the
catalog default. Model IDs and reasoning levels come from the response, not a
local model allowlist.

`default_reasoning_level`, every advertised `supported_reasoning_levels` entry,
and `default_service_tier` are preserved in the provider catalog.
`provider/model-selection.ts` carries the default reasoning level into the
agent's model capabilities; `web/state/catalog.ts` carries the same default and
full effort list into the shared model selector. An explicit effort, including
`max` or `ultra` when advertised, reaches Codex's `reasoning.effort` unchanged.
The adapter disables the generic Off option: omitting reasoning invokes Codex's
default rather than turning it off. An advertised `none` effort remains an
explicit selectable level.

Upstream references: [models request](https://github.com/openai/codex/blob/3dc1e2a58406dc69db5812539adfee7d89fa9ef7/codex-rs/codex-api/src/endpoint/models.rs),
[model metadata and presets](https://github.com/openai/codex/blob/3dc1e2a58406dc69db5812539adfee7d89fa9ef7/codex-rs/protocol/src/openai_models.rs),
and [picker visibility](https://github.com/openai/codex/blob/3dc1e2a58406dc69db5812539adfee7d89fa9ef7/codex-rs/app-server/src/models.rs).


## Catalog data contracts

Codex uses its existing catalog schema at the response boundary. The shared
models.dev client validates its catalog before caching; Claude's catalog filter
receives that validated snapshot. Public conversion helpers taking `unknown`
validate independently. Numeric model limits are integer token counts and costs
are finite nonnegative numbers. Missing capabilities, limits and cost fields map
to null. Mappers do not re-interpret wrong types or manufacture values from them.
Callers receive independent models.dev snapshots, so mutation cannot corrupt the
cached validated catalog. Malformed refresh responses can use the last good
Codex/models.dev copy only with its stale marker and a value-free warning.

Grok's `model-schemas.ts` validates the models response before mapping. The
[models endpoint](https://docs.x.ai/developers/rest-api-reference/inference/models)
uses an object with a `data` array of entries identified by `id`. Demi consumes
`name`, `description`, `context_window`, `input_modalities` and the adapter's
reasoning metadata: `supports_reasoning_effort`, `reasoning_efforts` entries with
`id`/optional `default`, and `reasoning_effort`. Unknown extra fields are permitted.
A valid empty `data` array remains an empty live catalog. Invalid entries fail the
whole response instead of being skipped.

Grok context windows, when present, are positive integer counts. Missing context
size remains unknown. Native image support follows supplied input modalities;
missing modalities and unadvertised tool support remain unknown. An explicit false
reasoning capability remains false. Missing default effort stays null; order does
not select one. Duplicate model/effort IDs and conflicting or unavailable declared
defaults fail validation. Built-in fallback metadata is a separate static catalog,
marked stale with a reason explaining why live discovery was unavailable. Its
fixed epoch source date denotes static, unfetched content and is never substituted
for a missing or malformed date in a live catalog.
