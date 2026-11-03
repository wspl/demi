---
status: proposed
---

# A compatible API's requests follow its models

## Problem

Alex adds a compatible API entry for a gateway, `https://llm.example.com/v1`
on Chat Completions, and adds the model `deepseek-v4-pro` to it. The model
thinks, calls a tool, and the next request of the turn is refused: DeepSeek's
thinking mode wants the earlier thinking back as `reasoning_content` on the
assistant message that called the tool. Demi sends it only for an entry of
the DeepSeek vendor. The same happens for every model that thinks that way
behind a gateway, and also for vendor entries other than DeepSeek's: Kimi's,
GLM's and Qwen's models want their thinking back too, which models.dev
records for each of them (`"interleaved": {"field": "reasoning_content"}`),
and Demi sends it to none of them.

Sam adds a compatible entry on the Messages API for a gateway that relays
both Claude and other models. A request of an effort is sent as Anthropic's
adaptive thinking (`thinking: {type: adaptive}` with `output_config.effort`)
to every model of the entry, since a compatible entry has no vendor and gets
the default of every vendor rule. Claude Sonnet 5.5 takes that and nothing
else; another vendor's model behind the same gateway may take only a token
budget, the form Demi sends every vendor on Messages but Anthropic, and its
request is refused or its thinking is ignored.

Both rules sit in a table by vendor (`VendorPolicy`), so an entry without a
vendor cannot have them, and a vendor's entry has them for all its models
alike. Yet they are facts of a model: models.dev states them per model
(`interleaved`, and `reasoning_options` with `effort` or `budget_tokens`).

## Decision

Each model of an entry carries the two facts, and the providers read them
from the model the request names. models.dev states both per model, so a
model added from it is filled; a compatible entry's model can be changed.

**Chat Completions: Interleaved thinking.** A model with Interleaved
thinking gets its earlier thinking back as `reasoning_content` on the
assistant message it preceded, and an empty `reasoning_content` on a
tool-call message without thinking, as DeepSeek requires of every such
message, also one written before the conversation switched to it. Only
thinking the same model wrote, unsigned, goes back: Demi reads it from
`reasoning_content` alone. A model without it gets no `reasoning_content`,
since a model that does not know the field may refuse it.

**Messages: Thinking.** A model has Effort, Anthropic's adaptive thinking at
the chosen reasoning level, or Token Budget, the level's number of tokens.

```text
Reasoning              Low · Medium · High · Max
Thinking               [ Effort ▾ ]   How a reasoning level is sent: as an
                                      effort, or as a token budget.
Interleaved thinking   [ On ]         Earlier thinking goes back with each
                                      tool call.
```

- A compatible entry's model shows the row of its entry's API, Thinking on
  Messages and Interleaved thinking on Chat Completions, and the user can
  change it, in Edit Model or with `demi providers models`. A vendor's model
  shows neither and the command refuses both: its facts come from the
  backend's models.dev copy each time, also when the entry keeps a list of
  its own, and a model models.dev does not list takes the defaults.
- A backend that has no copy yet, as after a restart, downloads one before
  it builds a vendor entry's provider. Until a download succeeds, the
  vendor's models take the defaults, and the first provider built after it
  has the models.dev facts.
- From models.dev: Interleaved thinking when the record's `interleaved` is
  `true` or names `reasoning_content`; Effort when the record is Anthropic's
  and lists an `effort` option, Token Budget otherwise, since no other
  vendor documents what its `effort` means on Messages. A Claude model whose
  record lists no `effort`, such as Haiku 4.5, lists no reasoning levels
  either, so it is never sent one.
- A model added by Custom starts with Effort and without Interleaved
  thinking.
- A compatible model stored before this keeps Effort and gets no
  Interleaved thinking: what it is sent now.
- So a vendor's entry changes only where models.dev says so: DeepSeek's
  models keep their thinking replay, and Kimi's, GLM's and Qwen's, among
  others, gain it.

The design to build from:

```rust
// crates/shared-types/src/model.rs
pub enum ThinkingForm { Effort, Budget }
/// What a provider needs to know of each model of an entry.
pub struct ModelWire { pub thinking_form: ThinkingForm, pub interleaved_thinking: bool }

// crates/web-api-protocol/src/providers.rs
// ConfiguredModel: `thinking_form` (`#[serde(default)]` Effort) and
// `interleaved_thinking` (default false). ModelMatch, an answer only, states
// both.

// crates/provider-registry/src/families.rs
// ApiKeyArgs: `vendor: VendorPolicy` becomes `models: ModelWires`, by model
// id: a compatible entry's from its configured models, a vendor's from the
// backend's models.dev copy. The registry rebuilds a provider when either
// changes.
```

- `VendorPolicy` and `policy_of` go. Its third field,
  `replay_assistant_status`, is set by no vendor, so it goes with them: no
  request rule is chosen by vendor any more.
- The Messages provider reads the model's form instead of
  `effort_as_budget`; the Chat Completions request reads the model's
  Interleaved thinking instead of `pass_back_reasoning_content`.

## Spike

None: the user chose to build without one.

## Prior art

- OpenCode keeps these as a model's `compatibility`, filled from models.dev
  and overridable per provider and per model in its configuration
  (`packages/schema/src/model-compatibility.ts`). On Chat Completions it
  sends thinking back in the field it arrived in, `reasoning_content` when
  unknown, and sends an empty one where DeepSeek needs it
  (`packages/ai/src/protocols/openai-chat.ts`): when models.dev says so,
  or when the provider, endpoint or model ID names DeepSeek. Demi takes the
  models.dev fact alone, as a field of the model, and no name matching.
- models.dev records, per model, the field a model wants its thinking back
  in (`interleaved`) and the thinking options it takes (`reasoning_options`).

## Alternatives

- A compatible entry picks a vendor whose rules it follows, from the
  models.dev list: one choice for a gateway that serves models of several
  vendors, and the vendor table still misses Kimi's and GLM's thinking.
- The rules as switches of a compatible entry: an entry's switch is wrong
  for one of a gateway's models whenever it serves models of two vendors.
- Chat Completions without a setting, sending thinking back whenever the
  history holds the model's own: the first request after a conversation
  switches to DeepSeek, or one after a tool call DeepSeek made without
  thinking, carries tool-call messages without `reasoning_content`, which
  DeepSeek refuses; and an empty one on every tool-call message is refused
  by models that do not know the field.

## Open questions

None. The user chose Effort for a model added by Custom.
