---
status: accepted
---

# A vendor's own step is an opaque block

## Problem

Some of what a model does happens at its vendor and reaches Demi only as a
record that the vendor alone can read: Anthropic's redacted reasoning, the
encrypted results of xAI's X search, OpenAI's hosted tool calls. Demi keeps
such records in three ways, each for one case:

- `RedactedThinkingBlock` keeps Anthropic's redacted reasoning, and the page
  draws it as thinking without text.
- A block's `entries` keep the Claude Code CLI's own session lines. They
  name no owner, because only Claude Code writes them; a record another
  provider kept there would go into the CLI's session file as it is.
- Everything else is dropped. The shared Responses reader maps reasoning,
  text and function calls, and skips every other item. When Alex's Grok
  agent searches X, the next request carries none of what it found, so the
  model searches again or answers from nothing, and the conversation shows
  no sign that a search happened.

## Decision

A step the vendor ran on its side is an **opaque block**: Demi keeps the
vendor's record of it, which only that vendor's provider reads, and a
**view** of it, which is what the page draws.

- The page draws an opaque block by its view. Each kind of view has its own
  look, so how a step shows is decided per kind and not per vendor: the
  search steps of [0176](0176-a-subscription-searches-with-its-vendors-search.md)
  are rows of a run with their sources; redacted reasoning is thinking
  without text, as now.
- A step whose kind Demi does not know is still kept and replayed, and shows
  as one line of its name and its vendor, such as *code_interpreter at Grok
  Build*, which reads the same while it runs and once it ended.
- A step that still runs when its request ends without a response, stopped,
  failed or cut short, ends as failed.
- A retry after a failure drops the reasoning of the attempt, as it drops
  its thinking, and keeps any other step the vendor ran, which the next
  attempt reads rather than runs again.
- A model reads an opaque block only when its provider owns the record.
  Another vendor's model reads nothing of it, as it reads nothing of another
  vendor's reasoning now.

### The vendor's record

Every entry of a block's `entries` names its owner, the tag a provider
already puts on its reasoning signatures: `anthropic:`, `claude-code:`,
`codex:`, `grok-build:`, or an API key entry's vendor such as `xai:`. A
compatible API's tag names its entry, `compatible:<entry id>:`, since two
compatible endpoints are two vendors; reasoning signed under the old
`compatible:` tag is no longer replayed.

- A provider reads only the entries it owns, as it replays only the
  reasoning signatures it recognizes. Claude Code's session file is then
  written from Claude Code's lines alone, whatever another provider kept on
  the same blocks.
- Entries stay the backend's: no client-facing form of a block carries them.

### Design

- `shared-types`:
  - `Entry` gains an owner and is stored as `{ "owner", "value" }`.
  - `Block::Opaque(OpaqueBlock)` with the common fields, `call_id` (null
    for a step the vendor gives whole), `status: OpaqueStatus` (`running`,
    `completed`, `failed`), `view: OpaqueView`, `ended_at`, `run` and
    `entries`.
  - `OpaqueView` is tagged by `kind`: `reasoning` (redacted reasoning) and
    `other { vendor, name }` here; 0176 adds `search`. A new kind is a new
    variant and a new component, never a vendor's name in the page's code.
  - `RedactedThinkingBlock` goes.
- `provider-common`:
  - `ProviderEvent::Opaque(OpaqueStep { call_id, status, view, entries })`
    starts a step, or updates the one of the running request with its call
    id, adding its entries after those the step holds. Claude Code's lines still
    reach their block through `Entries`.
  - `InferenceItem::Opaque { model_id, entries, kept_past_summary }` replaces
    `AssistantRedactedThinking`; a provider sends the entries it owns as they
    came, and nothing of a step past a summary that it would leave reasoning
    out for.
  - `wire::responses` turns every output item it does not map into an
    opaque step with the `other` view, running from the item's start and
    ended by its end, which share the item's id, else its place in the
    output; the item is kept as the entry and sent back as it came. The step
    shows under the entry's name.
- `provider-anthropic-api` keeps redacted reasoning as an entry it owns, the
  content block as the vendor sent it.
- `web-ui`: `visible-blocks` draws a `reasoning` view as thinking without
  text, as it drew a redacted thinking block; any other view is a step of
  its run, with a line of its own per kind; the run counts an `other` step
  as a use of its tool. `web-gallery`: steps of the `other` view that ended,
  failed and still run.
- Stored data: transcript block format 18. A release reads format 17 as 18
  without rewriting a row, as format 16 is read as 17:
  - every entry is owned by `claude-code:`, the only provider that wrote
    entries;
  - a `redacted_thinking` block is an opaque block with the `reasoning` view.
    Its data becomes an entry owned by `anthropic:` when it carries that tag,
    as the Anthropic API provider wrote it; Claude Code's own lines on the
    block keep any other.

  The step from 0.1.5742's schema renames the type the rows' column states.

## Spike

None: what the vendors send is known from 0173's runs, and this changes only
how Demi keeps it.

## Prior art

- CLIProxyAPI keeps the items it does not translate, such as xAI's server
  calls and their encrypted outputs, and sends them back to the same
  backend
  ([`xai_executor_request.go`](https://github.com/router-for-me/CLIProxyAPI/blob/90c447d3a8727d115aa7552ec93ce023736ed492/internal/runtime/executor/xai_executor_request.go#L1949-L2003)).
- The Vercel AI SDK carries a vendor's own data as provider metadata keyed by
  the provider's name, which only that provider reads back.

## Alternatives

- **A block per kind**, such as a search block. Each new vendor step would
  then be a new block type, a new migration and a new rule for replay, and
  an unknown one would still be dropped.
- **Untagged entries, read by the block's model.** A user block holds
  entries for every provider that replayed it, so the block's model cannot
  say whose they are.

## Open questions

None.
