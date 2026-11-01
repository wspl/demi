---
status: proposed
---

# A Grok Build model uses its largest window

## Problem

Alex runs a long conversation on Grok 4.7 through a Grok Build subscription.
The model menu and the usage indicator say its window is 256K, and Demi
compacts the conversation as it nears 256K, though Grok 4.7 takes 500K.

The proxy's catalog (`GET /v1/models`) states two fields for each model:

```json
{ "id": "grok-4.7", "context_window": 256000, "context_windows": [256000, 500000] }
```

`context_window` is the Grok CLI's default, and `context_windows` the sizes
its `/context-window` command offers. Demi reads only `context_window`.

## Decision

A Grok Build model's window is the largest size its catalog entry states,
500K for Grok 4.7 above. The model menu, the usage indicator and compaction
use it.

A conversation already on Grok 4.7 moves to 500K too, from its next request.
A model's facts, its window, output limit, thinking capabilities and file
types, now follow its entry's catalog in every conversation, as they already
did for an entry whose model list the user configured. Before, a conversation
kept the facts it took when the model was chosen: Alex's conversation from
before would stay at 256K while its model menu, which reads the catalog,
offered 500K, 300K and 200K, and a limit picked there would be ignored, since a
256K window offers none.

A window of 500K or more offers the Context row in the model menu, so Alex
can still ask Grok 4.7 for less:

```
Context   500K   300K   200K
```

The rule was "over 500K"; it becomes "500K or more". The 500K limit itself is
still offered only from a window of 1M up, since on a 500K window it is the
full window. A subagent profile's Context follows the same steps, as it does
today.

### Design

- `provider-grok-build`, `models.rs`: `GrokModel` reads `context_windows:
  Option<Vec<NonZeroU32>>`, and `ProviderModel::context_window` is the largest
  of it and `context_window`.
- `shared-types`, `context_limit.rs`: the 300K and 200K steps take
  `min_window: 500_000`. The steps are generated into `@demicodes/protocol`,
  so the page and the backend change together.
- `provider-registry`, `catalog.rs`: `ProviderRegistry::current_selection`
  replaces `configured_selection`. It gives a selection the facts the entry's
  catalog lists for its model, read as the catalog cache knows it now, so no
  request waits for a source, and keeps the selection's thinking and tier. A
  model the catalog does not list keeps its facts, since a catalog not read
  yet lists nothing; an entry's configured list refuses it, as before.
- The backend applies it where it applied the configured facts: the runtime
  a session opens with, each request's admission and the next account under
  rotation. The conversation's selection a tree opens with gets it too.
- `agent-session`: `SessionRuntime::current_model`, which the node answers
  through `ProviderResolver::current_model`. Every selection the session
  holds, its model's and those of a recorded switch, takes the current facts
  when the session is restored, before a switch lands or an edit's
  replacement starts, and before each request goes, a retry's included. The
  session saves its model's. Compaction's thresholds and its check that one
  response's input fits the model's window then read the window the vendor
  serves. A change is logged as `model.facts_changed`.
- A change of only the thinking or the tier takes the model's facts from the
  catalog, not from the conversation's record.
- Nothing stored changes. A limit is stored per model only when the user
  picks one, and no 500K model offered one before. The conversation's record
  keeps the facts of the last switch; whoever reads it applies the current
  ones.

## Spike

Whether the proxy needs anything in a request to accept more than 256K, as
some vendors want a header or a model name for a larger window: a request of
about 310,000 tokens, sent as Demi sends one and with nothing added, was
answered normally. The size is the client's choice: the Grok CLI sends nothing
for it either and only compacts later.

## Prior art

- The Grok CLI offers the sizes of `context_windows` with `/context-window`
  and in its `/model` picker, and starts each session at `context_window`.
  Demi starts at the largest; the Context row offers less.
- Demi's Codex catalog leaves out Codex's `max_context_window`, since nothing
  shows the service takes more than the window it states. For Grok Build the
  spike shows the proxy does.

## Alternatives

- **Keep a conversation's facts until the user picks the model again.** It
  leaves the model menu and the conversation apart whenever a catalog changes,
  and a limit the menu offers would do nothing.
- **Start at 256K, as the Grok CLI does, with 500K in the Context row.** The
  row offers limits below a model's window, and its default is the full
  window for every other model. Starting a model below its window would add a
  per-model default that no other provider has.
- **Add a 256K step.** It would differ from 200K by too little to be worth a
  choice, and it would appear on every 1M model too.

## Open questions

None.
