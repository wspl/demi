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
